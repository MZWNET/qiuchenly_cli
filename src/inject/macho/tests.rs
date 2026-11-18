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
