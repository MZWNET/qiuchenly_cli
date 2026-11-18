mod header;
mod patch;

#[cfg(test)]
mod tests;

use anyhow::{bail, ensure, Context, Result};
use goblin::mach::fat::{FAT_CIGAM, FAT_MAGIC, SIZEOF_FAT_ARCH, SIZEOF_FAT_HEADER};
use goblin::mach::{peek, MultiArch};
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
                changed |= patch::patch_slice(&mut data[range], load_path, replace, Some(arch))
                    .with_context(|| format!("处理 fat 切片 0x{:x}", arch.offset))?;
            }

            changed
        }
        FAT_CIGAM | 0xcafe_babf | 0xbfba_feca => bail!("不支持 FAT64 或字节交换的 fat 容器"),
        _ => patch::patch_slice(&mut data, load_path, replace, None)?,
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
