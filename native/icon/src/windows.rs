//! The Windows programs' icon: a compiled resource file (`.res`) holding
//! the icon at each of [`SIZES`] as `RT_ICON` resources and the group that
//! names them as one `RT_GROUP_ICON`, which a build script gives the MSVC
//! linker as an input (Microsoft's linker documentation, "LINK input
//! files": a `.res` file is converted and linked into the program). The
//! layout is Microsoft's "Resource File Formats": each resource a
//! `RESOURCEHEADER` and its data, on four-byte boundaries, after the empty
//! resource that marks a 32-bit file; an icon's group a `NEWHEADER` and one
//! `RESDIR` for each image, following the images; each image as in an
//! `.ico` file, a `BITMAPINFOHEADER` with the colour bits and then the AND
//! mask, or since Windows Vista a PNG. No crate and no resource compiler
//! is needed.

use std::path::{Path, PathBuf};

use crate::{BYTES_PER_PIXEL, Image, Refused};

/// The sides of the images the programs carry: Microsoft's sizes for a
/// desktop program's icon (16, 24, 32, 48 and 256, its app icon
/// construction guidance), and 64, the 32 of a display at twice the scale,
/// made exactly from the art.
pub const SIZES: [u32; 6] = [16, 24, 32, 48, 64, 256];
/// The images at this side and above are PNG, as Microsoft recommends for
/// 256; smaller ones are bitmaps, which every Windows reads.
const PNG_FROM: u32 = 256;
/// The icon group's name: the program's first icon, which Explorer, a
/// shortcut naming the program with icon index 0 and the uninstall entry
/// show.
pub const GROUP_ID: u16 = 1;
/// `RT_ICON` and `RT_GROUP_ICON` (`winuser.h`: `RT_GROUP_ICON` is
/// `RT_ICON + DIFFERENCE`, 3 + 11).
const RT_ICON: u16 = 3;
const RT_GROUP_ICON: u16 = 14;
/// A name or type given by number: 0xFFFF, then the number
/// (`RESOURCEHEADER`'s remarks).
const ORDINAL: u16 = 0xFFFF;
/// `RESOURCEHEADER`'s `MemoryFlags`, ignored by Windows and kept for the
/// resource compiler's sake as it writes them: `MOVEABLE` (0x0010) and
/// `DISCARDABLE` (0x1000) for an image, with `PURE` (0x0020) for a group.
const MOVEABLE: u16 = 0x0010;
const PURE: u16 = 0x0020;
const DISCARDABLE: u16 = 0x1000;
/// `LANG_NEUTRAL` with `SUBLANG_NEUTRAL` (`winnt.h`): the icon is no
/// language's, so it is found whatever the user's.
const LANGUAGE_NEUTRAL: u16 = 0;
/// The header of a resource named and typed by number: `DataSize`,
/// `HeaderSize`, the type and the name of four bytes each,
/// `DataVersion`, `MemoryFlags`, `LanguageId`, `Version` and
/// `Characteristics`.
const RESOURCE_HEADER_BYTES: u32 = 32;
/// `NEWHEADER`'s `ResType` for an icon (`RES_ICON`).
const RES_ICON: u16 = 1;
/// `BITMAPINFOHEADER`'s size, its own first field (`wingdi.h`).
const BITMAP_HEADER_BYTES: u32 = 40;
/// One plane, of 32 bits a pixel: blue, green, red and alpha.
const PLANES: u16 = 1;
const BITS_PER_PIXEL: u16 = 32;
/// `BI_RGB`: not compressed.
const BI_RGB: u32 = 0;
/// An `.ico` image's height is the colour bitmap's and the mask's
/// together, twice the icon's.
const BITMAP_HEIGHTS: u32 = 2;
/// The mask's rows, and every resource, on four-byte boundaries.
const ALIGNMENT: usize = 4;
/// Bits in a byte of the mask, the leftmost pixel in the highest bit.
const BITS_PER_BYTE: u32 = 8;
const LEFTMOST: u8 = 0x80;
/// `ICONRESDIR`'s width and height are a byte; 0 means 256.
const SIDE_IN_A_BYTE: u32 = 256;

/// `length` rounded up to [`ALIGNMENT`].
fn aligned(length: usize) -> Option<usize> {
    length.checked_next_multiple_of(ALIGNMENT)
}

/// Appends `value`, little-endian, as Windows stores every field.
fn push_u16(out: &mut Vec<u8>, value: u16) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn push_u32(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_le_bytes());
}

/// Appends the resource of type `kind` numbered `name` holding `data`,
/// then pads to the next resource's boundary.
fn push_resource(out: &mut Vec<u8>, kind: u16, name: u16, flags: u16, data: &[u8]) -> Result<(), Refused> {
    push_u32(out, u32::try_from(data.len()).map_err(|_| Refused("a resource too large"))?);
    push_u32(out, RESOURCE_HEADER_BYTES);
    for number in [kind, name] {
        push_u16(out, ORDINAL);
        push_u16(out, number);
    }
    // DataVersion.
    push_u32(out, 0);
    push_u16(out, flags);
    push_u16(out, LANGUAGE_NEUTRAL);
    // Version, Characteristics.
    push_u32(out, 0);
    push_u32(out, 0);
    out.extend_from_slice(data);
    out.resize(aligned(out.len()).ok_or(Refused("a resource file too large"))?, 0);
    Ok(())
}

/// One image as an `.ico` file holds it below [`PNG_FROM`]: a
/// `BITMAPINFOHEADER` for 32-bit pixels at twice the height, the pixels
/// from the bottom row up as blue, green, red and alpha, then the AND mask
/// from the bottom up, a bit a pixel, set where the pixel is wholly
/// transparent, each row padded to four bytes.
fn bitmap(image: &Image) -> Result<Vec<u8>, Refused> {
    let side = image.width();
    let mask_row = aligned(usize::try_from(side.div_ceil(BITS_PER_BYTE)).map_err(|_| Refused("too wide"))?).ok_or(Refused("too wide"))?;
    let rows = usize::try_from(side).map_err(|_| Refused("too tall"))?;
    let colour_bytes = image.rgba().len();
    let mask_bytes = mask_row.checked_mul(rows).ok_or(Refused("too large"))?;
    let image_bytes = u32::try_from(colour_bytes.checked_add(mask_bytes).ok_or(Refused("too large"))?).map_err(|_| Refused("too large"))?;
    let mut out = Vec::new();
    push_u32(&mut out, BITMAP_HEADER_BYTES);
    push_u32(&mut out, side);
    push_u32(&mut out, side.checked_mul(BITMAP_HEIGHTS).ok_or(Refused("too tall"))?);
    push_u16(&mut out, PLANES);
    push_u16(&mut out, BITS_PER_PIXEL);
    push_u32(&mut out, BI_RGB);
    push_u32(&mut out, image_bytes);
    // Pixels a metre across and down, colours used and important: none
    // said.
    for _ in 0..4 {
        push_u32(&mut out, 0);
    }
    let row_bytes = usize::try_from(side).ok().and_then(|side| side.checked_mul(BYTES_PER_PIXEL)).ok_or(Refused("too wide"))?;
    for row in image.rgba().chunks_exact(row_bytes).rev() {
        for &[red, green, blue, alpha] in row.as_chunks::<BYTES_PER_PIXEL>().0 {
            out.extend_from_slice(&[blue, green, red, alpha]);
        }
    }
    for row in image.rgba().chunks_exact(row_bytes).rev() {
        let mut mask = vec![0_u8; mask_row];
        for (column, &[_, _, _, alpha]) in row.as_chunks::<BYTES_PER_PIXEL>().0.iter().enumerate() {
            let bits = usize::try_from(BITS_PER_BYTE).unwrap_or(1);
            let bit = column.checked_rem(bits).and_then(|shift| u32::try_from(shift).ok()).and_then(|shift| LEFTMOST.checked_shr(shift)).unwrap_or(0);
            if alpha == 0
                && let Some(byte) = column.checked_div(bits).and_then(|index| mask.get_mut(index))
            {
                *byte |= bit;
            }
        }
        out.extend_from_slice(&mask);
    }
    Ok(out)
}

/// The resource file of the icon made from `art` at each of [`SIZES`]:
/// the empty resource, the images numbered from 1, then their group,
/// numbered [`GROUP_ID`].
///
/// # Errors
/// When an image cannot be made, which for the owner's art does not
/// happen.
pub fn resource_file(art: &Image) -> Result<Vec<u8>, Refused> {
    let mut out = Vec::new();
    // The empty resource: type and name 0, no data.
    push_resource(&mut out, 0, 0, 0, &[])?;
    let mut group = Vec::new();
    push_u16(&mut group, 0);
    push_u16(&mut group, RES_ICON);
    push_u16(&mut group, u16::try_from(SIZES.len()).map_err(|_| Refused("too many images"))?);
    for (number, side) in (1_u16..).zip(SIZES) {
        let image = art.scaled(side)?;
        let data = if side >= PNG_FROM { crate::png::encode(&image)? } else { bitmap(&image)? };
        push_resource(&mut out, RT_ICON, number, MOVEABLE | DISCARDABLE, &data)?;
        let in_a_byte = u8::try_from(side % SIDE_IN_A_BYTE).map_err(|_| Refused("too large"))?;
        // ICONRESDIR: width, height, colours (0: more than 256), reserved.
        group.extend_from_slice(&[in_a_byte, in_a_byte, 0, 0]);
        push_u16(&mut group, PLANES);
        push_u16(&mut group, BITS_PER_PIXEL);
        push_u32(&mut group, u32::try_from(data.len()).map_err(|_| Refused("an image too large"))?);
        push_u16(&mut group, number);
    }
    push_resource(&mut out, RT_GROUP_ICON, GROUP_ID, MOVEABLE | PURE | DISCARDABLE, &group)?;
    Ok(out)
}

/// The file a build script writes in `folder` (its `OUT_DIR`) and gives
/// the linker.
pub const RESOURCE_FILE: &str = "neoteletext-icon.res";

/// Writes the owner's art as [`resource_file`] to [`RESOURCE_FILE`] in
/// `folder`, for a build script to give the linker: where it is.
///
/// # Errors
/// When the art cannot be read or the file written.
pub fn write_resource_file(folder: &Path) -> Result<PathBuf, String> {
    let bytes = crate::source().and_then(|art| resource_file(&art)).map_err(|refused| refused.to_string())?;
    let path = folder.join(RESOURCE_FILE);
    std::fs::write(&path, bytes).map_err(|error| format!("{}: {error}", path.display()))?;
    Ok(path)
}

#[cfg(test)]
mod tests;
