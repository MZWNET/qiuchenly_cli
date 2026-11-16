use anyhow::{bail, ensure, Context, Result};
use goblin::mach::constants::{SECTION_TYPE, S_GB_ZEROFILL, S_THREAD_LOCAL_ZEROFILL, S_ZEROFILL};
use goblin::mach::fat::{FatArch, FAT_CIGAM, FAT_MAGIC, SIZEOF_FAT_ARCH, SIZEOF_FAT_HEADER};
use goblin::mach::header::{MH_BUNDLE, MH_DYLIB, MH_EXECUTE, SIZEOF_HEADER_32, SIZEOF_HEADER_64};
use goblin::mach::load_command::{self, CommandVariant, LoadCommand, LC_LOAD_DYLIB};
use goblin::mach::segment::Segment;
use goblin::mach::{parse_magic_and_ctx, peek, MultiArch};
use std::fs;
use std::ops::Range;
use std::path::Path;

const DYLIB_COMMAND_SIZE: usize = 24;

pub(super) fn patch_binary(path: &Path, load_path: &str, replace: Option<&str>) -> Result<()> {
    ensure!(
        !load_path.is_empty() && !load_path.as_bytes().contains(&0),
        "注入库引用路径不能为空或包含 NUL"
    );
    let mut data = fs::read(path).with_context(|| format!("读取 Mach-O {}", path.display()))?;
    let changed = match peek(&data, 0)? {
        FAT_MAGIC => {
            let fat = MultiArch::new(&data)?;
            ensure!(
                fat.narches > 0
                    && fat.narches <= (data.len() - SIZEOF_FAT_HEADER) / SIZEOF_FAT_ARCH,
                "fat 架构表为空或被截断"
            );
            let table_end = SIZEOF_FAT_HEADER + fat.narches * SIZEOF_FAT_ARCH;
            let mut arches = fat.arches()?;
            arches.sort_unstable_by_key(|arch| arch.offset);
            let mut previous_end = table_end;
            let mut changed = false;
            for arch in &arches {
                let range = file_range(data.len(), u64::from(arch.offset), u64::from(arch.size))?;
                ensure!(
                    !range.is_empty() && range.start >= previous_end,
                    "fat 切片为空、重叠或覆盖架构表"
                );
                ensure!(
                    arch.align < 32 && arch.offset % (1u32 << arch.align) == 0,
                    "fat 切片对齐无效"
                );
                previous_end = range.end;
                changed |= patch_slice(&mut data[range], load_path, replace, Some(arch))
                    .with_context(|| format!("处理 fat 切片 0x{:x}", arch.offset))?;
            }

            changed
        }
        FAT_CIGAM | 0xcafe_babf | 0xbfba_feca => bail!("不支持 FAT64 或字节交换的 fat 容器"),
        _ => patch_slice(&mut data, load_path, replace, None)?,
    };
    if changed {
        fs::write(path, &data).with_context(|| format!("写入 Mach-O {}", path.display()))?;
    }

    Ok(())
}

fn file_range(length: usize, offset: u64, size: u64) -> Result<Range<usize>> {
    ensure!(
        offset <= length as u64 && size <= length as u64 - offset,
        "Mach-O 文件内容越界或被截断: offset={offset}, size={size}, length={length}"
    );

    Ok(offset as usize..(offset + size) as usize)
}

struct HeaderPadding {
    slice_len: usize,
    commands_end: usize,
    end: usize,
    has_header_section: bool,
}

impl HeaderPadding {
    fn command(
        &mut self,
        command: &CommandVariant,
        command_bytes: &[u8],
        data: &[u8],
        offset: usize,
        context: goblin::container::Ctx,
    ) -> Result<()> {
        match command {
            CommandVariant::Segment32(segment) => {
                ensure!(!context.is_big(), "64 位 Mach-O 包含 LC_SEGMENT");
                file_range(
                    data.len(),
                    u64::from(segment.fileoff),
                    u64::from(segment.filesize),
                )?;
                self.segment(&Segment::from_32(data, segment, offset, context)?)?;
            }
            CommandVariant::Segment64(segment) => {
                ensure!(context.is_big(), "32 位 Mach-O 包含 LC_SEGMENT_64");
                file_range(data.len(), segment.fileoff, segment.filesize)?;
                self.segment(&Segment::from_64(data, segment, offset, context)?)?;
            }
            CommandVariant::Symtab(table) => {
                let symbol_size = if context.is_big() { 16 } else { 12 };
                self.content(
                    u64::from(table.symoff),
                    u64::from(table.nsyms) * symbol_size,
                )?;
                self.content(u64::from(table.stroff), u64::from(table.strsize))?;
            }
            CommandVariant::Dysymtab(table) => {
                let module_size = if context.is_big() { 56 } else { 52 };
                for (offset, size) in [
                    (table.tocoff, u64::from(table.ntoc) * 8),
                    (table.modtaboff, u64::from(table.nmodtab) * module_size),
                    (table.extrefsymoff, u64::from(table.nextrefsyms) * 4),
                    (table.indirectsymoff, u64::from(table.nindirectsyms) * 4),
                    (table.extreloff, u64::from(table.nextrel) * 8),
                    (table.locreloff, u64::from(table.nlocrel) * 8),
                ] {
                    self.content(u64::from(offset), size)?;
                }
            }
            CommandVariant::DyldInfo(info) | CommandVariant::DyldInfoOnly(info) => {
                for (offset, size) in [
                    (info.rebase_off, info.rebase_size),
                    (info.bind_off, info.bind_size),
                    (info.weak_bind_off, info.weak_bind_size),
                    (info.lazy_bind_off, info.lazy_bind_size),
                    (info.export_off, info.export_size),
                ] {
                    self.content(u64::from(offset), u64::from(size))?;
                }
            }
            CommandVariant::CodeSignature(info)
            | CommandVariant::SegmentSplitInfo(info)
            | CommandVariant::FunctionStarts(info)
            | CommandVariant::DataInCode(info)
            | CommandVariant::DylibCodeSignDrs(info)
            | CommandVariant::LinkerOptimizationHint(info)
            | CommandVariant::DyldExportsTrie(info)
            | CommandVariant::DyldChainedFixups(info) => {
                self.content(u64::from(info.dataoff), u64::from(info.datasize))?;
            }
            CommandVariant::EncryptionInfo32(info) => {
                self.content(u64::from(info.cryptoff), u64::from(info.cryptsize))?;
            }
            CommandVariant::EncryptionInfo64(info) => {
                self.content(u64::from(info.cryptoff), u64::from(info.cryptsize))?;
            }
            CommandVariant::TwolevelHints(info) => {
                self.content(u64::from(info.offset), u64::from(info.nhints) * 4)?;
            }
            CommandVariant::Symseg(info) => {
                self.content(u64::from(info.offset), u64::from(info.size))?;
            }
            CommandVariant::Note(info) => {
                self.content(info.offset, info.size)?;
            }
            CommandVariant::Main(entry) => {
                self.content(entry.entryoff, 1)?;
            }
            CommandVariant::IdDylib(dylib)
            | CommandVariant::LoadWeakDylib(dylib)
            | CommandVariant::ReexportDylib(dylib)
            | CommandVariant::LazyLoadDylib(dylib)
            | CommandVariant::LoadUpwardDylib(dylib) => {
                command_string(command_bytes, dylib.dylib.name, DYLIB_COMMAND_SIZE)?;
            }
            CommandVariant::LoadDylinker(linker)
            | CommandVariant::IdDylinker(linker)
            | CommandVariant::DyldEnvironment(linker) => {
                command_string(command_bytes, linker.name, 12)?;
            }
            CommandVariant::Rpath(path) => {
                command_string(command_bytes, path.path, 12)?;
            }
            CommandVariant::SubFramework(framework) => {
                command_string(command_bytes, framework.umbrella, 12)?;
            }
            CommandVariant::SubUmbrella(framework) => {
                command_string(command_bytes, framework.sub_umbrella, 12)?;
            }
            CommandVariant::SubClient(client) => {
                command_string(command_bytes, client.client, 12)?;
            }
            CommandVariant::SubLibrary(library) => {
                command_string(command_bytes, library.sub_library, 12)?;
            }
            CommandVariant::BuildVersion(version) => {
                let tools_size = command_bytes.len() - 24;
                ensure!(
                    tools_size % 8 == 0 && tools_size / 8 == version.ntools as usize,
                    "LC_BUILD_VERSION 工具表长度无效"
                );
            }
            CommandVariant::Uuid(_)
            | CommandVariant::Thread(_)
            | CommandVariant::Unixthread(_)
            | CommandVariant::Routines32(_)
            | CommandVariant::Routines64(_)
            | CommandVariant::VersionMinMacosx(_)
            | CommandVariant::VersionMinIphoneos(_)
            | CommandVariant::VersionMinTvos(_)
            | CommandVariant::VersionMinWatchos(_)
            | CommandVariant::SourceVersion(_)
            | CommandVariant::PrebindCksum(_) => {}
            _ => bail!(
                "不支持 Mach-O load command 0x{:x}，无法确认其文件内容边界",
                command.cmd()
            ),
        }

        Ok(())
    }

    fn content(&mut self, offset: u64, size: u64) -> Result<Range<usize>> {
        let range = file_range(self.slice_len, offset, size)?;
        if !range.is_empty() {
            ensure!(
                range.start >= self.commands_end,
                "Mach-O 文件内容与 load commands 重叠"
            );
            self.end = self.end.min(range.start);
        }

        Ok(range)
    }

    fn segment(&mut self, segment: &Segment<'_>) -> Result<()> {
        let (segment_size, section_size) = if segment.cmd == load_command::LC_SEGMENT_64 {
            (
                load_command::SIZEOF_SEGMENT_COMMAND_64,
                load_command::SIZEOF_SECTION_64,
            )
        } else {
            (
                load_command::SIZEOF_SEGMENT_COMMAND_32,
                load_command::SIZEOF_SECTION_32,
            )
        };
        let section_bytes = segment.cmdsize as usize - segment_size;
        ensure!(
            section_bytes.is_multiple_of(section_size)
                && section_bytes / section_size == segment.nsects as usize,
            "Mach-O section 表与 segment 命令长度不符"
        );
        let segment_start = segment.fileoff as usize;
        let segment_end = segment_start + segment.data.len();
        let contains_header = segment.fileoff == 0 && segment.filesize != 0;
        if contains_header {
            ensure!(
                segment_end >= self.commands_end,
                "Mach-O header 超出所属 segment"
            );
            self.end = self.end.min(segment_end);
        } else {
            self.content(segment.fileoff, segment.filesize)?;
        }
        let mut has_file_section = false;
        for section in segment {
            let (section, _) = section?;
            if !matches!(
                section.flags & SECTION_TYPE,
                S_ZEROFILL | S_GB_ZEROFILL | S_THREAD_LOCAL_ZEROFILL
            ) {
                let range = self.content(u64::from(section.offset), section.size)?;
                if !range.is_empty() {
                    ensure!(
                        range.start >= segment_start && range.end <= segment_end,
                        "Mach-O section 超出所属 segment"
                    );
                    has_file_section = true;
                }
            }
            self.content(u64::from(section.reloff), u64::from(section.nreloc) * 8)?;
        }
        if contains_header {
            if has_file_section {
                self.has_header_section = true;
            } else {
                self.end = self.commands_end;
            }
        }

        Ok(())
    }
}

fn command_string(command: &[u8], offset: u32, fixed_size: usize) -> Result<Range<usize>> {
    let start = offset as usize;
    ensure!(
        start >= fixed_size && start < command.len(),
        "Mach-O load command 字符串偏移无效"
    );
    let length = command[start..]
        .iter()
        .position(|byte| *byte == 0)
        .context("Mach-O load command 字符串缺少 NUL 终止符")?;
    let range = start..start + length;
    std::str::from_utf8(&command[range.clone()]).context("Mach-O load command 字符串不是 UTF-8")?;

    Ok(range)
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

fn patch_slice(
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

#[cfg(test)]
mod tests {
    use super::patch_binary;
    use std::fs;

    const HEADER_END: usize = 328;
    const TEXT_START: usize = 0x200;
    const SLICE_LEN: usize = 0x4040;
    const LOAD_PATH: &str = "@executable_path/q";

    fn set_u32(data: &mut [u8], offset: usize, value: u32) {
        data[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }

    fn set_u64(data: &mut [u8], offset: usize, value: u64) {
        data[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
    }

    fn thin(cpu: u32, content_offset: usize, dependency: Option<&str>) -> Vec<u8> {
        let mut data = vec![0; SLICE_LEN];
        let dependency_size = dependency.map_or(0, |name| (24 + name.len() + 1 + 7) & !7);
        set_u32(&mut data, 0, 0xfeed_facf);
        set_u32(&mut data, 4, cpu);
        set_u32(&mut data, 8, if cpu == 0x0100_0007 { 3 } else { 0 });
        set_u32(&mut data, 12, 6);
        set_u32(&mut data, 16, if dependency.is_some() { 5 } else { 4 });
        set_u32(&mut data, 20, (296 + dependency_size) as u32);
        set_u32(&mut data, 24, 0x85);
        set_u32(&mut data, 32, 0x19);
        set_u32(&mut data, 36, 152);
        data[40..46].copy_from_slice(b"__TEXT");
        set_u64(&mut data, 56, 0x1_0000_0000);
        set_u64(&mut data, 64, 0x4000);
        set_u64(&mut data, 80, 0x4000);
        set_u32(&mut data, 88, 7);
        set_u32(&mut data, 92, 5);
        set_u32(&mut data, 96, 1);
        data[104..110].copy_from_slice(b"__text");
        data[120..126].copy_from_slice(b"__TEXT");
        set_u64(&mut data, 136, 0x1_0000_0000 + content_offset as u64);
        set_u64(&mut data, 144, 32);
        set_u32(&mut data, 152, content_offset as u32);
        set_u32(&mut data, 156, 4);
        set_u32(&mut data, 168, 0x8000_0400);
        set_u32(&mut data, 184, 0x19);
        set_u32(&mut data, 188, 72);
        data[192..202].copy_from_slice(b"__LINKEDIT");
        set_u64(&mut data, 208, 0x1_0000_4000);
        set_u64(&mut data, 216, 0x4000);
        set_u64(&mut data, 224, 0x4000);
        set_u64(&mut data, 232, 0x40);
        set_u32(&mut data, 240, 1);
        set_u32(&mut data, 244, 1);
        set_u32(&mut data, 256, 2);
        set_u32(&mut data, 260, 24);
        set_u32(&mut data, 264, 0x4000);
        set_u32(&mut data, 268, 1);
        set_u32(&mut data, 272, 0x4010);
        set_u32(&mut data, 276, 0x10);
        set_u32(&mut data, 280, 0xd);
        set_u32(&mut data, 284, 48);
        set_u32(&mut data, 288, 24);
        set_u32(&mut data, 292, 2);
        set_u32(&mut data, 296, 0x1_0000);
        set_u32(&mut data, 300, 0x1_0000);
        data[304..324].copy_from_slice(b"@rpath/Example.dylib");
        if let Some(name) = dependency {
            set_u32(&mut data, HEADER_END, 0xc);
            set_u32(&mut data, HEADER_END + 4, dependency_size as u32);
            set_u32(&mut data, HEADER_END + 8, 24);
            data[HEADER_END + 24..HEADER_END + 24 + name.len()].copy_from_slice(name.as_bytes());
        }
        for instruction in data[content_offset..content_offset + 32]
            .as_chunks_mut::<4>()
            .0
        {
            instruction.copy_from_slice(if cpu == 0x0100_000c {
                &[0x1f, 0x20, 0x03, 0xd5]
            } else {
                &[0x90; 4]
            });
        }
        set_u32(&mut data, 0x4000, 1);
        data[0x4004] = 0xf;
        data[0x4005] = 1;
        set_u64(&mut data, 0x4008, 0x1_0000_0000 + content_offset as u64);
        data[0x4010..0x401a].copy_from_slice(b"\0_example\0");
        data[0x4020..].fill(0xa5);

        data
    }

    fn fat(first: &[u8], second: &[u8]) -> Vec<u8> {
        let mut data = vec![0x5a; 0xc000 + second.len()];
        data[..4].copy_from_slice(&0xcafe_babeu32.to_be_bytes());
        data[4..8].copy_from_slice(&2u32.to_be_bytes());
        for (header, offset, slice) in [(8, 0x4000, first), (28, 0xc000, second)] {
            for field in [0, 4] {
                let value = u32::from_le_bytes(slice[4 + field..8 + field].try_into().unwrap());
                data[header + field..header + field + 4].copy_from_slice(&value.to_be_bytes());
            }
            data[header + 8..header + 12].copy_from_slice(&(offset as u32).to_be_bytes());
            data[header + 12..header + 16].copy_from_slice(&(slice.len() as u32).to_be_bytes());
            data[header + 16..header + 20].copy_from_slice(&14u32.to_be_bytes());
            data[offset..offset + slice.len()].copy_from_slice(slice);
        }

        data
    }

    #[test]
    fn appends_thin_without_moving_content_and_is_idempotent() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("thin.dylib");
        let original = thin(0x0100_000c, TEXT_START, None);
        fs::write(&path, &original).unwrap();
        patch_binary(&path, LOAD_PATH, None).unwrap();
        let patched = fs::read(&path).unwrap();
        assert_eq!(patched.len(), original.len());
        assert_eq!(&patched[..16], &original[..16]);
        assert_eq!(&patched[16..20], &5u32.to_le_bytes());
        assert_eq!(&patched[20..24], &344u32.to_le_bytes());
        assert_eq!(&patched[24..HEADER_END], &original[24..HEADER_END]);
        assert_eq!(&patched[HEADER_END..HEADER_END + 4], &0xcu32.to_le_bytes());
        assert_eq!(
            &patched[HEADER_END + 4..HEADER_END + 8],
            &48u32.to_le_bytes()
        );
        assert_eq!(
            &patched[HEADER_END + 24..HEADER_END + 24 + LOAD_PATH.len()],
            LOAD_PATH.as_bytes()
        );
        assert_eq!(patched[HEADER_END + 24 + LOAD_PATH.len()], 0);
        assert_eq!(&patched[HEADER_END + 48..], &original[HEADER_END + 48..]);
        patch_binary(&path, LOAD_PATH, None).unwrap();
        assert_eq!(fs::read(&path).unwrap(), patched);
    }

    #[test]
    fn appends_fat_without_changing_slice_offsets_or_content() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("universal.dylib");
        let original = fat(
            &thin(0x0100_000c, TEXT_START, None),
            &thin(0x0100_0007, TEXT_START, None),
        );
        fs::write(&path, &original).unwrap();
        patch_binary(&path, LOAD_PATH, None).unwrap();
        let patched = fs::read(&path).unwrap();
        assert_eq!(patched.len(), original.len());
        assert_eq!(&patched[..0x4000], &original[..0x4000]);
        assert_eq!(
            &patched[0x4000 + SLICE_LEN..0xc000],
            &original[0x4000 + SLICE_LEN..0xc000]
        );
        for offset in [0x4000, 0xc000] {
            assert_eq!(&patched[offset + 16..offset + 20], &5u32.to_le_bytes());
            assert_eq!(&patched[offset + 20..offset + 24], &344u32.to_le_bytes());
            assert_eq!(
                &patched[offset + 24..offset + HEADER_END],
                &original[offset + 24..offset + HEADER_END]
            );
            assert_eq!(
                &patched[offset + HEADER_END + 24..offset + HEADER_END + 24 + LOAD_PATH.len()],
                LOAD_PATH.as_bytes()
            );
            assert_eq!(
                &patched[offset + HEADER_END + 48..offset + SLICE_LEN],
                &original[offset + HEADER_END + 48..offset + SLICE_LEN]
            );
        }
        patch_binary(&path, LOAD_PATH, None).unwrap();
        assert_eq!(fs::read(&path).unwrap(), patched);
    }

    #[test]
    fn replaces_link_ability_in_place_and_rejects_longer_paths() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("linked.dylib");
        let old = "/usr/lib/libiconv.2.dylib";
        let original = thin(0x0100_000c, TEXT_START, Some(old));
        fs::write(&path, &original).unwrap();
        assert!(patch_binary(&path, "/usr/lib/libiconv.22.dylib", Some(old)).is_err());
        assert_eq!(fs::read(&path).unwrap(), original);
        patch_binary(&path, LOAD_PATH, Some(old)).unwrap();
        let patched = fs::read(&path).unwrap();
        let name_start = HEADER_END + 24;
        assert_eq!(patched.len(), original.len());
        assert_eq!(&patched[..name_start], &original[..name_start]);
        assert_eq!(
            &patched[name_start..name_start + LOAD_PATH.len()],
            LOAD_PATH.as_bytes()
        );
        assert!(
            patched[name_start + LOAD_PATH.len()..name_start + old.len()]
                .iter()
                .all(|byte| *byte == 0)
        );
        assert_eq!(
            &patched[name_start + old.len()..],
            &original[name_start + old.len()..]
        );
        patch_binary(&path, LOAD_PATH, Some(old)).unwrap();
        assert_eq!(fs::read(&path).unwrap(), patched);
    }

    #[test]
    fn rejects_invalid_or_unpadded_binaries_without_writing() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("rejected.dylib");
        let valid = thin(0x0100_000c, TEXT_START, None);
        let mut no_padding = thin(0x0100_0007, 336, None);
        no_padding[336..368].fill(0);
        let mut occupied_padding = valid.clone();
        occupied_padding[HEADER_END] = 0x7f;
        let mut malformed_command = valid.clone();
        set_u32(&mut malformed_command, 36, 8);
        let mut unknown_command = valid.clone();
        set_u32(&mut unknown_command, 280, 0x1234_5678);
        let mut malformed_name = thin(0x0100_000c, TEXT_START, Some("/usr/lib/libiconv.2.dylib"));
        set_u32(&mut malformed_name, HEADER_END + 8, u32::MAX);
        let invalid_fat = fat(&valid, &no_padding);
        for original in [
            no_padding,
            occupied_padding,
            malformed_command,
            unknown_command,
            malformed_name,
            valid[..39].to_vec(),
            valid[..0x4020].to_vec(),
            invalid_fat,
        ] {
            fs::write(&path, &original).unwrap();
            assert!(patch_binary(&path, LOAD_PATH, None).is_err());
            assert_eq!(fs::read(&path).unwrap(), original);
        }
    }
}
