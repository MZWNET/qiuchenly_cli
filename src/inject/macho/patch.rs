use super::header::{command_string, HeaderPadding};
use super::DYLIB_COMMAND_SIZE;

use anyhow::{ensure, Context, Result};
use goblin::mach::fat::FatArch;
use goblin::mach::header::{MH_BUNDLE, MH_DYLIB, MH_EXECUTE, SIZEOF_HEADER_32, SIZEOF_HEADER_64};
use goblin::mach::load_command::{CommandVariant, LoadCommand, LC_LOAD_DYLIB};
use goblin::mach::parse_magic_and_ctx;

pub(super) fn patch_slice(
    data: &mut [u8],
    load_path: &str,
    replace: Option<&str>,
    arch: Option<&FatArch>,
) -> Result<bool> {
    let (_, context) = parse_magic_and_ctx(data, 0)?;
    let context = context.context("不是受支持的 Mach-O 切片")?;
    let header_size = if context.is_big() {
        SIZEOF_HEADER_64
    } else {
        SIZEOF_HEADER_32
    };
    ensure!(data.len() >= header_size, "Mach-O header 被截断");
    let little_endian = context.is_little_endian();
    if let Some(arch) = arch {
        ensure!(
            u32_at(data, 4, little_endian) == arch.cputype
                && u32_at(data, 8, little_endian) == arch.cpusubtype,
            "fat 架构表与 Mach-O 切片 CPU 类型不符"
        );
    }
    let filetype = u32_at(data, 12, little_endian);
    ensure!(
        matches!(filetype, MH_EXECUTE | MH_DYLIB | MH_BUNDLE),
        "不支持 Mach-O 文件类型 0x{filetype:x}"
    );
    let ncmds = u32_at(data, 16, little_endian);
    let sizeofcmds = u32_at(data, 20, little_endian);
    ensure!(
        sizeofcmds as usize <= data.len() - header_size && ncmds <= sizeofcmds / 8,
        "Mach-O load commands 越界或被截断"
    );
    let commands_end = header_size + sizeofcmds as usize;
    let alignment = if context.is_big() { 8 } else { 4 };
    let mut padding = HeaderPadding {
        slice_len: data.len(),
        commands_end,
        end: data.len(),
        has_header_section: false,
    };
    let mut position = header_size;
    let mut replacement = None;
    let mut already_loaded = false;
    for _ in 0..ncmds {
        let remaining = &data[position..commands_end];
        ensure!(remaining.len() >= 8, "Mach-O load command header 被截断");
        let command_size = u32_at(remaining, 4, little_endian) as usize;
        ensure!(
            command_size >= 8
                && command_size.is_multiple_of(alignment)
                && command_size <= remaining.len(),
            "Mach-O load command 长度或对齐无效"
        );
        let command_bytes = &remaining[..command_size];
        let mut parsed_size = 0;
        let command = LoadCommand::parse(command_bytes, &mut parsed_size, context.le)?;
        if let CommandVariant::LoadDylib(dylib) = &command.command {
            let name = command_string(command_bytes, dylib.dylib.name, DYLIB_COMMAND_SIZE)?;
            let name_bytes = &command_bytes[name.clone()];
            already_loaded |= name_bytes == load_path.as_bytes();
            if replacement.is_none() && replace.is_some_and(|old| name_bytes == old.as_bytes()) {
                replacement = Some(position + name.start..position + name.end);
            }
        } else {
            padding.command(&command.command, command_bytes, data, position, context)?;
        }
        position += command_size;
    }
    ensure!(
        position == commands_end,
        "Mach-O sizeofcmds 与命令总长度不符"
    );
    if let Some(old) = replace {
        if let Some(range) = replacement {
            ensure!(
                load_path.len() <= range.len(),
                "linkAbility: 新引用路径过长，无法原地替换 {old}"
            );
            if &data[range.clone()] == load_path.as_bytes() {
                return Ok(false);
            }
            let start = range.start;
            data[range].fill(0);
            data[start..start + load_path.len()].copy_from_slice(load_path.as_bytes());

            return Ok(true);
        }
        ensure!(
            already_loaded,
            "linkAbility: 找不到引用 {old} 的 LC_LOAD_DYLIB"
        );

        return Ok(false);
    }
    if already_loaded {
        return Ok(false);
    }
    let command_size = load_path
        .len()
        .checked_add(DYLIB_COMMAND_SIZE + alignment)
        .context("注入库引用路径过长")?
        & !(alignment - 1);
    ensure!(
        padding.has_header_section && command_size <= padding.end - commands_end,
        "Mach-O header padding 不足，无法在不移动文件内容的情况下追加 LC_LOAD_DYLIB"
    );
    let new_end = commands_end + command_size;
    ensure!(
        data[commands_end..new_end].iter().all(|byte| *byte == 0),
        "Mach-O header padding 含非零内容，拒绝覆盖"
    );
    let command_size_u32 = u32::try_from(command_size).context("LC_LOAD_DYLIB 长度超出 u32")?;
    let new_ncmds = ncmds.checked_add(1).context("Mach-O ncmds 溢出")?;
    let new_sizeofcmds = sizeofcmds
        .checked_add(command_size_u32)
        .context("Mach-O sizeofcmds 溢出")?;
    let command = &mut data[commands_end..new_end];
    write_u32(command, 0, LC_LOAD_DYLIB, little_endian);
    write_u32(command, 4, command_size_u32, little_endian);
    write_u32(command, 8, DYLIB_COMMAND_SIZE as u32, little_endian);
    write_u32(command, 12, 2, little_endian);
    command[DYLIB_COMMAND_SIZE..DYLIB_COMMAND_SIZE + load_path.len()]
        .copy_from_slice(load_path.as_bytes());
    write_u32(data, 16, new_ncmds, little_endian);
    write_u32(data, 20, new_sizeofcmds, little_endian);

    Ok(true)
}

fn u32_at(data: &[u8], offset: usize, little_endian: bool) -> u32 {
    let bytes = data[offset..offset + 4].try_into().expect("已验证字段范围");

    if little_endian {
        u32::from_le_bytes(bytes)
    } else {
        u32::from_be_bytes(bytes)
    }
}

fn write_u32(data: &mut [u8], offset: usize, value: u32, little_endian: bool) {
    let bytes = if little_endian {
        value.to_le_bytes()
    } else {
        value.to_be_bytes()
    };
    data[offset..offset + 4].copy_from_slice(&bytes);
}
