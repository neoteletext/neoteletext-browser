//! A page image as the trusted core sends it to the interface (requirement
//! 9.3, decision 61): its palette of at most 32 colours, the colour of the
//! screen around it, and its dots in the smallest of three forms. Every
//! dot is an index into the palette, so colours stay exact.
//!
//! - **Raw:** one byte a dot.
//! - **Runs:** a dot value and how many dots in a row have it, 1 to 255.
//! - **Changes:** against the frame of the same page the interface holds,
//!   named by its version: dots unchanged, then dots changed and their
//!   values, in turn, to the end of the page.
//!
//! Measured on 800 real pages (22 October 2026): runs encode in a tenth of
//! the time of the earlier PNG images, and the changes that flashing makes
//! are a seventh of a PNG image's bytes. Raw bounds the worst case, so a
//! frame always fits one message.
//!
//! The reader faces a process that may have been compromised only in the
//! other direction, but reads as strictly as every reader here: each count
//! is bounded before use, the dots must fill the page exactly, and every
//! index must lie within the palette.

use interface_core::canvas::{PAGE_HEIGHT, PAGE_WIDTH};

/// The Colour Map of Presentation Levels 2.5 and 3.5: 32 entries (EN 300
/// 706 clause 12.4).
pub const MAX_COLOURS: usize = 32;
/// The dots of a page, 480 by 500.
pub const PAGE_DOTS: usize = PAGE_WIDTH as usize * PAGE_HEIGHT as usize;
/// Red, green and blue: the bytes of one palette entry.
pub(crate) const RGB_BYTES: usize = 3;
const FORM_RAW: u8 = 0;
const FORM_RUNS: u8 = 1;
const FORM_CHANGES: u8 = 2;
/// The longest run a pair can say.
const MAX_RUN: usize = u8::MAX as usize;
/// The most dots one group of a changes form can skip or change.
const MAX_GROUP: usize = u16::MAX as usize;
const VERSION_BYTES: usize = 4;
const GROUP_BYTES: usize = 4;
/// The longest image: a full palette, the screen colour, the form, and a
/// raw page, which is never larger than the form chosen.
pub const MAX_IMAGE: usize = 1 + MAX_COLOURS * RGB_BYTES + 1 + 1 + PAGE_DOTS;
const _: () = assert!(MAX_IMAGE + 6 <= crate::MAX_PAYLOAD, "an image and its key and version fit one message");
/// The fewest bytes a page in runs can take: every run its longest. Changes
/// no longer than this are never beaten by runs, which are then not tried.
const LEAST_RUNS: usize = 2 * PAGE_DOTS.div_ceil(MAX_RUN);
/// Room reserved for an image: the 95th percentile of 800 real pages in
/// runs, 28,939 bytes (measured 22 October 2026), rounded up; a longer one
/// grows.
const EXPECTED_IMAGE: usize = 32 * 1024;

/// A page image as read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Image {
    /// Red, green and blue of each colour.
    pub palette: Vec<[u8; 3]>,
    /// The palette index of the screen around the page: the page's Default
    /// Screen Colour or Full Screen Colour (EN 300 706 Table 4, clause 12.3).
    pub screen: u8,
    /// One palette index per dot, row by row.
    pub indices: Vec<u8>,
}

/// What is wrong with an image.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FrameError {
    /// The palette is empty or larger than 32 colours.
    Palette,
    /// The screen colour or a dot lies outside the palette.
    Index,
    /// The form byte names no form.
    Form,
    /// The dots do not fill the page exactly, or the bytes end early or
    /// run on.
    Shape,
    /// A run or a group of length 0 where one is not allowed.
    Empty,
    /// A changes form against a frame that is not the one held.
    Base,
}

impl core::fmt::Display for FrameError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(match self {
            Self::Palette => "palette of no colours or more than 32",
            Self::Index => "an index outside the palette",
            Self::Form => "an unknown form",
            Self::Shape => "dots that do not fill the page",
            Self::Empty => "an empty run or group",
            Self::Base => "changes against a frame not held",
        })
    }
}

fn header(palette: &[[u8; 3]], screen: u8, form: u8, capacity: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(capacity);
    out.push(u8::try_from(palette.len()).unwrap_or(0));
    for colour in palette {
        out.extend_from_slice(colour);
    }
    out.push(screen);
    out.push(form);
    out
}

fn runs(palette: &[[u8; 3]], screen: u8, indices: &[u8]) -> Vec<u8> {
    let mut out = header(palette, screen, FORM_RUNS, EXPECTED_IMAGE);
    let mut rest = indices;
    while let Some(&value) = rest.first() {
        let run = rest.iter().take(MAX_RUN).take_while(|&&dot| dot == value).count();
        out.push(value);
        out.push(u8::try_from(run).unwrap_or(u8::MAX));
        rest = rest.get(run..).unwrap_or_default();
    }
    out
}

fn changes(palette: &[[u8; 3]], screen: u8, indices: &[u8], base: u32, previous: &[u8]) -> Vec<u8> {
    let mut out = header(palette, screen, FORM_CHANGES, EXPECTED_IMAGE);
    out.extend_from_slice(&base.to_be_bytes());
    let mut at = 0;
    let from = |at: usize| indices.get(at..).unwrap_or_default().iter().zip(previous.get(at..).unwrap_or_default()).take(MAX_GROUP);
    while at < indices.len() {
        let same = from(at).take_while(|(now, before)| now == before).count();
        at = at.saturating_add(same);
        let changed = from(at).take_while(|(now, before)| now != before).count();
        out.extend_from_slice(&u16::try_from(same).unwrap_or(u16::MAX).to_be_bytes());
        out.extend_from_slice(&u16::try_from(changed).unwrap_or(u16::MAX).to_be_bytes());
        out.extend_from_slice(indices.get(at..at.saturating_add(changed)).unwrap_or_default());
        at = at.saturating_add(changed);
    }
    out
}

/// Encodes a page image in its smallest form: changes against `previous`
/// (the version and indices the interface holds) when given, runs, or raw.
/// `None` when the image is not a page's shape.
#[must_use]
pub fn encode(palette: &[[u8; 3]], screen: u8, indices: &[u8], previous: Option<(u32, &[u8])>) -> Option<Vec<u8>> {
    if palette.is_empty() || palette.len() > MAX_COLOURS || indices.len() != PAGE_DOTS || usize::from(screen) >= palette.len() {
        return None;
    }
    let against = previous.filter(|(_, previous)| previous.len() == PAGE_DOTS).map(|(base, previous)| changes(palette, screen, indices, base, previous));
    let mut best = match against {
        Some(against) if against.len() <= LEAST_RUNS => against,
        Some(against) => {
            let whole = runs(palette, screen, indices);
            if against.len() < whole.len() { against } else { whole }
        }
        None => runs(palette, screen, indices),
    };
    if best.len() > MAX_IMAGE {
        best = header(palette, screen, FORM_RAW, MAX_IMAGE);
        best.extend_from_slice(indices);
    }
    Some(best)
}

/// The version a changes form is against, so the reader can find the frame
/// it needs; none for the other forms.
#[must_use]
pub fn base(bytes: &[u8]) -> Option<u32> {
    let colours = usize::from(*bytes.first()?);
    let form_at = 1_usize.saturating_add(colours.saturating_mul(RGB_BYTES)).saturating_add(1);
    if *bytes.get(form_at)? != FORM_CHANGES {
        return None;
    }
    let version = bytes.get(form_at.saturating_add(1)..form_at.saturating_add(1).saturating_add(VERSION_BYTES))?;
    Some(u32::from_be_bytes(version.try_into().ok()?))
}

/// Reads a page image; `held` is the version and indices of the frame of
/// the same page the reader holds, which a changes form is applied to.
///
/// # Errors
/// The first rule the bytes break.
pub fn decode(bytes: &[u8], held: Option<(u32, &[u8])>) -> Result<Image, FrameError> {
    let (&colours, rest) = bytes.split_first().ok_or(FrameError::Shape)?;
    let colours = usize::from(colours);
    if colours == 0 || colours > MAX_COLOURS {
        return Err(FrameError::Palette);
    }
    let (palette, rest) = rest.split_at_checked(colours.saturating_mul(RGB_BYTES)).ok_or(FrameError::Shape)?;
    let palette = palette.as_chunks::<RGB_BYTES>().0.to_vec();
    let [screen, form, body @ ..] = rest else { return Err(FrameError::Shape) };
    if usize::from(*screen) >= colours {
        return Err(FrameError::Index);
    }
    let indices = match *form {
        FORM_RAW if body.len() == PAGE_DOTS => body.to_vec(),
        FORM_RAW => return Err(FrameError::Shape),
        FORM_RUNS => {
            let mut indices = Vec::with_capacity(PAGE_DOTS);
            let (pairs, odd) = body.as_chunks::<2>();
            if !odd.is_empty() {
                return Err(FrameError::Shape);
            }
            for &[value, run] in pairs {
                if run == 0 {
                    return Err(FrameError::Empty);
                }
                if indices.len().saturating_add(usize::from(run)) > PAGE_DOTS {
                    return Err(FrameError::Shape);
                }
                indices.extend(core::iter::repeat_n(value, usize::from(run)));
            }
            indices
        }
        FORM_CHANGES => {
            let (version, mut rest) = body.split_at_checked(VERSION_BYTES).ok_or(FrameError::Shape)?;
            let version = u32::from_be_bytes(version.try_into().map_err(|_| FrameError::Shape)?);
            let previous = held.filter(|(held, previous)| *held == version && previous.len() == PAGE_DOTS).ok_or(FrameError::Base)?.1;
            let mut indices = previous.to_vec();
            let mut at = 0_usize;
            while at < PAGE_DOTS {
                let (group, after) = rest.split_at_checked(GROUP_BYTES).ok_or(FrameError::Shape)?;
                let [same_high, same_low, changed_high, changed_low] = group else { return Err(FrameError::Shape) };
                let same = usize::from(u16::from_be_bytes([*same_high, *same_low]));
                let changed = usize::from(u16::from_be_bytes([*changed_high, *changed_low]));
                if same == 0 && changed == 0 {
                    return Err(FrameError::Empty);
                }
                at = at.saturating_add(same);
                let end = at.saturating_add(changed);
                let (values, after) = after.split_at_checked(changed).ok_or(FrameError::Shape)?;
                indices.get_mut(at..end).ok_or(FrameError::Shape)?.copy_from_slice(values);
                at = end;
                rest = after;
            }
            if at != PAGE_DOTS || !rest.is_empty() {
                return Err(FrameError::Shape);
            }
            indices
        }
        _ => return Err(FrameError::Form),
    };
    if indices.len() != PAGE_DOTS {
        return Err(FrameError::Shape);
    }
    // The largest index, in one pass the compiler can vectorise.
    if indices.iter().copied().max().is_some_and(|largest| usize::from(largest) >= colours) {
        return Err(FrameError::Index);
    }
    Ok(Image { palette, screen: *screen, indices })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing, clippy::arithmetic_side_effects, reason = "a test")]
mod tests {
    use super::*;

    const PALETTE: [[u8; 3]; 3] = [[0, 0, 0], [255, 0, 0], [17, 51, 255]];

    fn page() -> Vec<u8> {
        (0..PAGE_DOTS).map(|dot| u8::from(dot / 1000 % 3 == 1)).collect()
    }

    /// Requirement 9.3: each form gives back the exact palette, screen
    /// colour and dots.
    #[test]
    fn every_form_reads_back_exactly() {
        let first = page();
        let runs = encode(&PALETTE, 2, &first, None).unwrap();
        assert_eq!(runs[1 + 3 * 3 + 1], FORM_RUNS);
        assert_eq!(decode(&runs, None).unwrap(), Image { palette: PALETTE.to_vec(), screen: 2, indices: first.clone() });

        let mut flashed = first.clone();
        flashed[1234..1300].fill(2);
        flashed[PAGE_DOTS - 1] = 2;
        let against = encode(&PALETTE, 2, &flashed, Some((7, &first))).unwrap();
        assert_eq!(base(&against), Some(7));
        assert!(against.len() < 200, "a few changed dots are a few bytes: {}", against.len());
        assert_eq!(decode(&against, Some((7, &first))).unwrap().indices, flashed);
        assert_eq!(decode(&against, Some((6, &first))), Err(FrameError::Base), "against another frame");
        assert_eq!(decode(&against, None), Err(FrameError::Base), "against no frame");

        // Alternate dots make the longest runs: raw is chosen, and the
        // largest image is within its bound.
        let noise: Vec<u8> = (0..PAGE_DOTS).map(|dot| u8::try_from(dot % 2).unwrap()).collect();
        let raw = encode(&PALETTE, 0, &noise, Some((1, &first))).unwrap();
        assert_eq!(raw[1 + 3 * 3 + 1], FORM_RAW);
        assert!(raw.len() <= MAX_IMAGE);
        assert_eq!(decode(&raw, None).unwrap().indices, noise);
        assert_eq!(base(&raw), None);
    }

    /// Requirement 9.3: anything else is refused.
    #[test]
    fn a_broken_image_is_refused() {
        let first = page();
        assert_eq!(encode(&PALETTE, 3, &first, None), None, "screen outside the palette");
        assert_eq!(encode(&[], 0, &first, None), None);
        assert_eq!(encode(&PALETTE, 0, &first[1..], None), None);
        let good = encode(&PALETTE, 1, &first, None).unwrap();
        let broken = |at: usize, value: u8| {
            let mut bytes = good.clone();
            bytes[at] = value;
            decode(&bytes, None)
        };
        assert_eq!(broken(0, 0), Err(FrameError::Palette));
        assert_eq!(broken(0, 33), Err(FrameError::Palette));
        assert_eq!(broken(10, 3), Err(FrameError::Index), "screen");
        assert_eq!(broken(11, 9), Err(FrameError::Form));
        assert_eq!(broken(12, 3), Err(FrameError::Index), "a dot");
        assert_eq!(broken(13, 0), Err(FrameError::Empty));
        assert_eq!(decode(&good[..good.len() - 2], None), Err(FrameError::Shape), "short");
        let mut long = good.clone();
        long.extend_from_slice(&[0, 1]);
        assert_eq!(decode(&long, None), Err(FrameError::Shape), "too many dots");
        // A changes form whose groups run past the page.
        let mut flashed = first.clone();
        flashed[0] = 2;
        let mut against = encode(&PALETTE, 0, &flashed, Some((1, &first))).unwrap();
        let groups = 1 + 9 + 2 + VERSION_BYTES;
        against[groups..groups + 2].copy_from_slice(&u16::MAX.to_be_bytes());
        assert_eq!(decode(&against, Some((1, &first))), Err(FrameError::Shape));
        // Every dot unchanged, but the palette shrank below the held dots'
        // index: each dot is checked against the new palette.
        let held: Vec<u8> = vec![2; PAGE_DOTS];
        let mut unchanged = header(&PALETTE[..2], 0, FORM_CHANGES, 0);
        unchanged.extend_from_slice(&5_u32.to_be_bytes());
        let mut left = PAGE_DOTS;
        while left > 0 {
            let same = left.min(MAX_GROUP);
            unchanged.extend_from_slice(&u16::try_from(same).unwrap().to_be_bytes());
            unchanged.extend_from_slice(&0_u16.to_be_bytes());
            left -= same;
        }
        assert_eq!(decode(&unchanged, Some((5, &held))), Err(FrameError::Index));
    }
}
