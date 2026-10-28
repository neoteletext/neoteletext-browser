#![allow(clippy::unwrap_used, clippy::indexing_slicing, clippy::arithmetic_side_effects, clippy::cast_possible_truncation, reason = "a test")]

use super::*;
use crate::source;

fn u16_at(bytes: &[u8], at: usize) -> u16 {
    u16::from_le_bytes(bytes[at..at + 2].try_into().unwrap())
}

fn u32_at(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap())
}

/// One resource as `RESOURCEHEADER` lays it out: its type and name by
/// number, its memory flags, its language and its data.
#[derive(Debug)]
struct Resource {
    kind: u16,
    name: u16,
    flags: u16,
    language: u16,
    data: Vec<u8>,
}

/// The resources of a resource file, read by Microsoft's "Resource File
/// Formats" and checked as they are read: every header 32 bytes with
/// numbers for its type and name, every version and characteristic 0,
/// every resource starting on a four-byte boundary.
fn resources(file: &[u8]) -> Vec<Resource> {
    let mut at = 0;
    let mut found = Vec::new();
    while at < file.len() {
        assert_eq!(at % 4, 0, "a resource on a four-byte boundary");
        let data_size = u32_at(file, at) as usize;
        assert_eq!(u32_at(file, at + 4), 32, "HeaderSize");
        assert_eq!(u16_at(file, at + 8), 0xFFFF, "the type by number");
        assert_eq!(u16_at(file, at + 12), 0xFFFF, "the name by number");
        assert_eq!(u32_at(file, at + 16), 0, "DataVersion");
        assert_eq!(u32_at(file, at + 24), 0, "Version");
        assert_eq!(u32_at(file, at + 28), 0, "Characteristics");
        found.push(Resource {
            kind: u16_at(file, at + 10),
            name: u16_at(file, at + 14),
            flags: u16_at(file, at + 20),
            language: u16_at(file, at + 22),
            data: file[at + 32..at + 32 + data_size].to_vec(),
        });
        let end = at + 32 + data_size;
        assert!(file[end..end.next_multiple_of(4).min(file.len())].iter().all(|byte| *byte == 0), "padding is zeros");
        at = end.next_multiple_of(4);
    }
    assert_eq!(at, file.len(), "nothing after the last resource");
    found
}

/// The documented layout: the empty resource first; an `RT_ICON` for each
/// size, numbered from 1; then the `RT_GROUP_ICON` numbered 1, whose
/// `NEWHEADER` and `RESDIR`s name each image with its size, bits, length
/// and number; each image below 256 a `BITMAPINFOHEADER`, the pixels from
/// the bottom up in blue, green, red and alpha, and an AND mask clear for
/// the opaque art; 256 a PNG of the art at that size.
#[test]
fn the_resource_file_has_the_documented_layout() {
    let art = source().unwrap();
    let file = resource_file(&art).unwrap();
    let found = resources(&file);
    assert_eq!(found.len(), 1 + SIZES.len() + 1);
    let empty = &found[0];
    assert_eq!((empty.kind, empty.name, empty.flags, empty.language, empty.data.len()), (0, 0, 0, 0, 0), "the empty resource of a 32-bit file");
    let group = found.last().unwrap();
    assert_eq!((group.kind, group.name, group.flags, group.language), (14, GROUP_ID, 0x1030, 0));
    assert_eq!((u16_at(&group.data, 0), u16_at(&group.data, 2), u16_at(&group.data, 4)), (0, 1, SIZES.len() as u16), "NEWHEADER");
    assert_eq!(group.data.len(), 6 + 14 * SIZES.len(), "a RESDIR of 14 bytes an image");
    for (index, side) in SIZES.into_iter().enumerate() {
        let icon = &found[1 + index];
        let number = index as u16 + 1;
        assert_eq!((icon.kind, icon.name, icon.flags, icon.language), (3, number, 0x1010, 0), "{side}");
        let entry = &group.data[6 + 14 * index..6 + 14 * (index + 1)];
        assert_eq!(&entry[..4], &[(side % 256) as u8, (side % 256) as u8, 0, 0], "ICONRESDIR of {side}");
        assert_eq!((u16_at(entry, 4), u16_at(entry, 6)), (1, 32), "planes and bits of {side}");
        assert_eq!(u32_at(entry, 8) as usize, icon.data.len(), "BytesInRes of {side}");
        assert_eq!(u16_at(entry, 12), number, "the image's number");
        let image = art.scaled(side).unwrap();
        if side >= 256 {
            assert_eq!(crate::png::decode(&icon.data).unwrap(), image, "256 is a PNG");
            continue;
        }
        let data = &icon.data;
        let mask_row = (side as usize).div_ceil(32) * 4;
        let pixels = (side * side) as usize * 4;
        assert_eq!(u32_at(data, 0), 40, "biSize");
        assert_eq!((u32_at(data, 4), u32_at(data, 8)), (side, 2 * side), "biWidth and biHeight, the mask's rows counted");
        assert_eq!((u16_at(data, 12), u16_at(data, 14), u32_at(data, 16)), (1, 32, 0), "biPlanes, biBitCount, BI_RGB");
        assert_eq!(u32_at(data, 20) as usize, pixels + mask_row * side as usize, "biSizeImage");
        assert!((24..40).step_by(4).all(|at| u32_at(data, at) == 0));
        assert_eq!(data.len(), 40 + pixels + mask_row * side as usize);
        for y in 0..side {
            for x in 0..side {
                let at = 40 + (((side - 1 - y) * side + x) * 4) as usize;
                let [red, green, blue, alpha] = image.pixel(x, y).unwrap();
                assert_eq!(&data[at..at + 4], &[blue, green, red, alpha], "{side}: {x}, {y}, the bottom row first");
            }
        }
        assert!(data[40 + pixels..].iter().all(|byte| *byte == 0), "{side}: the opaque art's mask is clear");
    }
}

/// The AND mask: a bit a pixel from the left, set where the pixel is
/// wholly transparent, rows from the bottom up, each padded to four bytes.
#[test]
fn the_mask_marks_transparent_pixels() {
    let side = 16;
    let mut rgba = vec![0xff; side * side * 4];
    // Transparent: the top row's first pixel and the bottom row's last.
    rgba[3] = 0;
    rgba[(side * side - 1) * 4 + 3] = 0;
    let image = Image::new(16, 16, rgba).unwrap();
    let data = bitmap(&image).unwrap();
    let mask = &data[40 + side * side * 4..];
    assert_eq!(mask.len(), 4 * side);
    assert_eq!(&mask[..4], &[0x00, 0x01, 0, 0], "the bottom row, first in the mask: its last pixel");
    assert_eq!(&mask[4 * (side - 1)..], &[0x80, 0x00, 0, 0], "the top row, last: its first pixel");
    assert!(mask[4..4 * (side - 1)].iter().all(|byte| *byte == 0));
}

#[test]
fn the_build_script_file_is_the_resource_file() {
    let folder = std::env::temp_dir().join(format!("neoteletext-icon-res-{}", std::process::id()));
    std::fs::create_dir_all(&folder).unwrap();
    let path = write_resource_file(&folder).unwrap();
    assert_eq!(path, folder.join(RESOURCE_FILE));
    assert_eq!(std::fs::read(&path).unwrap(), resource_file(&source().unwrap()).unwrap());
    std::fs::remove_dir_all(&folder).unwrap();
}
