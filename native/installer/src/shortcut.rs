//! A Start menu shortcut, written as the file it is: the Shell Link
//! Binary File Format ([MS-SHLLINK], Microsoft's open specification),
//! with no COM call, so no unsafe code. A link to a local program holds a
//! header (section 2.1), a link information structure naming the target
//! by its path (section 2.3, with its Unicode path), and the strings of
//! its name, working folder and icon (section 2.4); the extra data ends
//! with its terminal block (section 2.5).

use std::path::Path;

/// Section 2.1: `HeaderSize`, always 0x4C.
const HEADER_SIZE: u32 = 0x4C;
/// Section 2.1: `LinkCLSID`, 00021401-0000-0000-C000-000000000046, as its
/// bytes (the first three fields little-endian).
const LINK_CLSID: [u8; 16] = [0x01, 0x14, 0x02, 0x00, 0x00, 0x00, 0x00, 0x00, 0xC0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x46];
/// Section 2.1.1's `LinkFlags`.
const HAS_LINK_TARGET_ID_LIST: u32 = 0x0000_0001;
const HAS_LINK_INFO: u32 = 0x0000_0002;
const HAS_NAME: u32 = 0x0000_0004;
const HAS_WORKING_DIR: u32 = 0x0000_0010;
const HAS_ICON_LOCATION: u32 = 0x0000_0040;
const IS_UNICODE: u32 = 0x0000_0080;
/// Section 2.1.2: `FILE_ATTRIBUTE_NORMAL`.
const FILE_ATTRIBUTE_NORMAL: u32 = 0x0000_0080;
/// Section 2.1: `SW_SHOWNORMAL`.
const SW_SHOWNORMAL: u32 = 0x0000_0001;
/// Section 2.3: `LinkInfoHeaderSize` with the Unicode offsets present.
const LINK_INFO_HEADER_SIZE: u32 = 0x24;
/// Section 2.3: `LinkInfoFlags`' `VolumeIDAndLocalBasePath`.
const VOLUME_ID_AND_LOCAL_BASE_PATH: u32 = 0x0000_0001;
/// Section 2.3.1: `DRIVE_FIXED`.
const DRIVE_FIXED: u32 = 0x0000_0003;
/// Section 2.3.1: `VolumeLabelOffset`, just after the four fields.
const VOLUME_LABEL_OFFSET: u32 = 0x10;

fn push_u32(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn utf16(text: &str) -> Vec<u16> {
    text.encode_utf16().collect()
}

/// Section 2.4: a string as its count of characters, then UTF-16LE.
fn push_string(out: &mut Vec<u8>, text: &str) {
    let units = utf16(text);
    out.extend_from_slice(&u16::try_from(units.len()).unwrap_or(u16::MAX).to_le_bytes());
    for unit in units.iter().take(usize::from(u16::MAX)) {
        out.extend_from_slice(&unit.to_le_bytes());
    }
}

/// A length as the structure's 32-bit field.
fn length(bytes: usize) -> u32 {
    u32::try_from(bytes).unwrap_or(u32::MAX)
}

/// The shell's item for the computer: a root item (0x1F, sort index 0x50)
/// naming `CLSID_MyComputer`, {20D04FE0-3AEA-1069-A2D8-08002B30309D}.
const COMPUTER_ITEM: [u8; 18] = [0x1F, 0x50, 0xE0, 0x4F, 0xD0, 0x20, 0xEA, 0x3A, 0x69, 0x10, 0xA2, 0xD8, 0x08, 0x00, 0x2B, 0x30, 0x30, 0x9D];
/// A drive's item: its type, then the drive's root in ASCII, in a
/// fixed field.
const DRIVE_ITEM_TYPE: u8 = 0x2F;
const DRIVE_ITEM_BYTES: usize = 23;
/// A file system item's type: a folder, or a file.
const FOLDER_ITEM_TYPE: u8 = 0x31;
const FILE_ITEM_TYPE: u8 = 0x32;
/// Its attributes: `FILE_ATTRIBUTE_DIRECTORY`, `FILE_ATTRIBUTE_ARCHIVE`.
const FOLDER_ATTRIBUTES: u16 = 0x10;
const FILE_ATTRIBUTES: u16 = 0x20;

/// One item: its size, which counts itself, then its bytes (section 2.2.2).
fn push_item(out: &mut Vec<u8>, item: &[u8]) {
    out.extend_from_slice(&u16::try_from(item.len().saturating_add(2)).unwrap_or(u16::MAX).to_le_bytes());
    out.extend_from_slice(item);
}

/// Section 2.2: the target as the shell's item list, the computer, the
/// drive, then each folder and the file by name, with no times or sizes
/// (the shell reads the rest from the file system). Windows reads a
/// link's target path from this list: a link with only the link
/// information below resolved to nothing (measured in the virtual
/// machine, 23 October 2026). A name is given in ASCII, a character that
/// is not as `?`; the link information carries the Unicode path.
fn id_list(target: &str) -> Vec<u8> {
    let mut items = Vec::new();
    push_item(&mut items, &COMPUTER_ITEM);
    let (drive, rest) = target.split_at_checked(3).unwrap_or((target, ""));
    let mut drive_item = vec![0_u8; DRIVE_ITEM_BYTES];
    if let Some(first) = drive_item.first_mut() {
        *first = DRIVE_ITEM_TYPE;
    }
    for (slot, byte) in drive_item.iter_mut().skip(1).zip(drive.bytes()) {
        *slot = byte;
    }
    push_item(&mut items, &drive_item);
    let parts: Vec<&str> = rest.split('\\').filter(|part| !part.is_empty()).collect();
    for (index, part) in parts.iter().enumerate() {
        let file = index.saturating_add(1) == parts.len();
        let mut item = vec![if file { FILE_ITEM_TYPE } else { FOLDER_ITEM_TYPE }, 0];
        // File size, then DOS date and time: unknown, zero.
        item.extend_from_slice(&[0; 8]);
        item.extend_from_slice(&(if file { FILE_ATTRIBUTES } else { FOLDER_ATTRIBUTES }).to_le_bytes());
        item.extend(part.chars().map(|character| if character.is_ascii() { u8::try_from(character).unwrap_or(b'?') } else { b'?' }));
        item.push(0);
        if item.len() % 2 == 1 {
            item.push(0);
        }
        push_item(&mut items, &item);
    }
    // The terminal item.
    items.extend_from_slice(&[0, 0]);
    let mut out = u16::try_from(items.len()).unwrap_or(u16::MAX).to_le_bytes().to_vec();
    out.extend_from_slice(&items);
    out
}

/// Section 2.3: the link information naming `target` by its local path.
fn link_info(target: &str) -> Vec<u8> {
    // Section 2.3.1: the volume, with an empty label.
    let mut volume = Vec::new();
    push_u32(&mut volume, 0);
    push_u32(&mut volume, DRIVE_FIXED);
    push_u32(&mut volume, 0);
    push_u32(&mut volume, VOLUME_LABEL_OFFSET);
    volume.push(0);
    let volume_size = length(volume.len());
    if let Some(first) = volume.get_mut(..4) {
        first.copy_from_slice(&volume_size.to_le_bytes());
    }
    // The system code page's path, which a reader without the Unicode one
    // uses: the path where it is ASCII, else a question mark for each
    // character that is not.
    let ansi: Vec<u8> = target.chars().map(|character| if character.is_ascii() { u8::try_from(character).unwrap_or(b'?') } else { b'?' }).chain([0]).collect();
    let suffix = [0_u8];
    let unicode: Vec<u8> = utf16(target).into_iter().chain([0]).flat_map(u16::to_le_bytes).collect();
    let unicode_suffix = [0_u8, 0];
    let volume_offset = LINK_INFO_HEADER_SIZE;
    let base_offset = volume_offset.saturating_add(length(volume.len()));
    let suffix_offset = base_offset.saturating_add(length(ansi.len()));
    let unicode_offset = suffix_offset.saturating_add(length(suffix.len()));
    let unicode_suffix_offset = unicode_offset.saturating_add(length(unicode.len()));
    let total = unicode_suffix_offset.saturating_add(length(unicode_suffix.len()));
    let mut out = Vec::new();
    for field in
        [total, LINK_INFO_HEADER_SIZE, VOLUME_ID_AND_LOCAL_BASE_PATH, volume_offset, base_offset, 0, suffix_offset, unicode_offset, unicode_suffix_offset]
    {
        push_u32(&mut out, field);
    }
    out.extend_from_slice(&volume);
    out.extend_from_slice(&ansi);
    out.extend_from_slice(&suffix);
    out.extend_from_slice(&unicode);
    out.extend_from_slice(&unicode_suffix);
    out
}

/// A shortcut to the program `target`, started in `working`, named
/// `name`, with the program's own icon.
#[must_use]
pub fn shortcut(target: &Path, working: &Path, name: &str) -> Vec<u8> {
    let target = target.display().to_string();
    let mut out = Vec::new();
    push_u32(&mut out, HEADER_SIZE);
    out.extend_from_slice(&LINK_CLSID);
    push_u32(&mut out, HAS_LINK_TARGET_ID_LIST | HAS_LINK_INFO | HAS_NAME | HAS_WORKING_DIR | HAS_ICON_LOCATION | IS_UNICODE);
    push_u32(&mut out, FILE_ATTRIBUTE_NORMAL);
    // CreationTime, AccessTime, WriteTime: unknown, zero.
    out.extend_from_slice(&[0; 24]);
    // FileSize, IconIndex.
    push_u32(&mut out, 0);
    push_u32(&mut out, 0);
    push_u32(&mut out, SW_SHOWNORMAL);
    // HotKey, Reserved1, Reserved2, Reserved3.
    out.extend_from_slice(&[0; 2 + 2 + 4 + 4]);
    debug_assert_eq!(out.len(), HEADER_SIZE as usize);
    out.extend_from_slice(&id_list(&target));
    out.extend_from_slice(&link_info(&target));
    push_string(&mut out, name);
    push_string(&mut out, &working.display().to_string());
    push_string(&mut out, &target);
    // Section 2.5: the terminal block.
    push_u32(&mut out, 0);
    out
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing, clippy::arithmetic_side_effects, reason = "a test")]
mod tests {
    use super::*;

    fn u32_at(bytes: &[u8], at: usize) -> u32 {
        u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap())
    }

    #[test]
    fn a_shortcut_has_the_specifications_structure() {
        let target = r"C:\Users\Ann\AppData\Local\Programs\NeoTeletext\NeoTeletext.exe";
        let link = shortcut(Path::new(target), Path::new(r"C:\Users\Ann\AppData\Local\Programs\NeoTeletext"), "NeoTeletext");
        assert_eq!(u32_at(&link, 0), HEADER_SIZE);
        assert_eq!(&link[4..20], &LINK_CLSID);
        assert_eq!(u32_at(&link, 0x14) & HAS_LINK_TARGET_ID_LIST, HAS_LINK_TARGET_ID_LIST);
        // The item list: the computer, the drive, then one item per name,
        // the last a file, ending with an empty item.
        let list = HEADER_SIZE as usize;
        let list_size = usize::from(u16::from_le_bytes([link[list], link[list + 1]]));
        let mut at = list + 2;
        let mut items = Vec::new();
        loop {
            let size = usize::from(u16::from_le_bytes([link[at], link[at + 1]]));
            if size == 0 {
                break;
            }
            items.push(link[at + 2..at + size].to_vec());
            at += size;
        }
        assert_eq!(at + 2, list + 2 + list_size, "the list's size counts its items and the terminal");
        assert_eq!(items[0], COMPUTER_ITEM);
        assert_eq!(&items[1][..4], b"\x2FC:\\");
        let names: Vec<String> =
            items[2..].iter().map(|item| String::from_utf8(item[12..].split(|byte| *byte == 0).next().unwrap().to_vec()).unwrap()).collect();
        assert_eq!(names, ["Users", "Ann", "AppData", "Local", "Programs", "NeoTeletext", "NeoTeletext.exe"]);
        assert!(items[2..items.len() - 1].iter().all(|item| item[0] == FOLDER_ITEM_TYPE));
        assert_eq!(items[items.len() - 1][0], FILE_ITEM_TYPE);
        let info = list + 2 + list_size;
        let info_size = u32_at(&link, info) as usize;
        assert_eq!(u32_at(&link, info + 4), LINK_INFO_HEADER_SIZE);
        // The local path, as ASCII and as UTF-16, at the offsets given.
        let base = info + u32_at(&link, info + 16) as usize;
        assert_eq!(&link[base..base + target.len()], target.as_bytes());
        let unicode = info + u32_at(&link, info + 28) as usize;
        let read: Vec<u16> = link[unicode..unicode + target.len() * 2].chunks(2).map(|pair| u16::from_le_bytes([pair[0], pair[1]])).collect();
        assert_eq!(String::from_utf16(&read).unwrap(), target);
        // The name's string follows the link information.
        let name = info + info_size;
        assert_eq!(u16::from_le_bytes([link[name], link[name + 1]]), 11);
        assert_eq!(u32_at(&link, link.len() - 4), 0, "the terminal block");
    }
}
