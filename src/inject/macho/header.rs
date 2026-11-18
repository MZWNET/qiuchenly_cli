use super::{file_range, DYLIB_COMMAND_SIZE};
use anyhow::{bail, ensure, Context, Result};
use goblin::container::Ctx;
use goblin::mach::constants::{SECTION_TYPE, S_GB_ZEROFILL, S_THREAD_LOCAL_ZEROFILL, S_ZEROFILL};
use goblin::mach::load_command::{self, CommandVariant};
use goblin::mach::segment::Segment;
use std::ops::Range;

pub(super) struct HeaderPadding {
    pub(super) slice_len: usize,
    pub(super) commands_end: usize,
    pub(super) end: usize,
    pub(super) has_header_section: bool,
}

impl HeaderPadding {
    pub(super) fn command(
        &mut self,
        command: &CommandVariant,
        command_bytes: &[u8],
        data: &[u8],
        offset: usize,
        context: Ctx,
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

pub(super) fn command_string(
    command: &[u8],
    offset: u32,
    fixed_size: usize,
) -> Result<Range<usize>> {
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
