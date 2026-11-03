//! A picture converted for the Tools menu (`docs/picture-tools.md`): the
//! panel's choices as the engine's converter's options, and the page it
//! makes. Only a decoder started with `--picture` runs this, confined as
//! every decoder is, so the picture's decoders never run in the trusted
//! core (requirement 3.2).

use interface_core::picture::{ALIGNS, PictureChoice};
use neoteletext_picture::{Align, Fit, Level, Options, convert, parse_colour};
use protocol::decoder::FromDecoder;
use protocol::picture::MAX_PICTURE_PAGE;

const _: () = assert!(protocol::picture::MAX_PICTURE_FILE == neoteletext_picture::MAX_INPUT_BYTES, "the broker reads what the converter takes");

/// The engine's names of the backgrounds, in the panel's order: the Level 1
/// colours by number (EN 300 706 Table 26).
const BACKGROUND_NAMES: [&str; 8] = ["black", "red", "green", "yellow", "blue", "magenta", "cyan", "white"];

/// The engine's names of the alignments, in the panel's order.
const ALIGN_NAMES: [&str; ALIGNS.len()] = ["top-left", "top", "top-right", "left", "center", "right", "bottom-left", "bottom", "bottom-right"];

/// The converter's options for the panel's choices. Objects stay off: the
/// decoders draw with the `modern-classic` profile, which has no objects, so
/// a page that kept its rows in them would show none of its colours.
#[must_use]
pub fn options(choice: PictureChoice) -> Options {
    let level = [Level::OnePointFive, Level::TwoPointFive, Level::ThreePointFive].get(usize::from(choice.level)).copied().unwrap_or_default();
    let fit = [Fit::Crop, Fit::Contain, Fit::Stretch].get(usize::from(choice.fit)).copied().unwrap_or_default();
    let align = ALIGN_NAMES.get(usize::from(choice.align)).and_then(|name| Align::parse(name));
    let background = BACKGROUND_NAMES.get(usize::from(choice.background)).and_then(|name| parse_colour(name)).unwrap_or_default();
    Options { level, fit, align, background, hold_mosaics: choice.hold_mosaics && level == Level::OnePointFive, objects: false, ..Options::default() }
}

/// The page of the picture in `bytes`, or why there is none.
#[must_use]
pub fn converted(bytes: &[u8], choice: PictureChoice) -> FromDecoder {
    let page = match convert(bytes, &options(choice)) {
        Ok(page) => page,
        Err(error) => return FromDecoder::PictureRefused(error.to_string()),
    };
    let t42 = match page.t42() {
        Ok(t42) => t42,
        Err(error) => return FromDecoder::PictureRefused(error.to_string()),
    };
    if page.tti.len() > MAX_PICTURE_PAGE || t42.len() > MAX_PICTURE_PAGE {
        return FromDecoder::PictureRefused(String::from("the page made is larger than the browser takes"));
    }
    FromDecoder::PictureConverted { tti: page.tti.into_bytes(), t42, summary: page.summary }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::panic, clippy::arithmetic_side_effects, reason = "a test")]
mod tests {
    use super::*;

    /// Every choice the panel can send maps onto the converter's options
    /// without a conflict it would refuse.
    #[test]
    fn every_choice_is_a_valid_conversion() {
        for level in 0..3 {
            for fit in 0..3 {
                for align in 0..9 {
                    for background in 0..8 {
                        for hold_mosaics in [false, true] {
                            let choice = PictureChoice { level, fit, align, background, hold_mosaics: hold_mosaics && level == 0 };
                            assert!(choice.is_valid());
                            assert_eq!(options(choice).check(), Ok(()), "{choice:?}");
                        }
                    }
                }
            }
        }
        let options = options(PictureChoice { level: 0, fit: 1, align: 0, background: 4, hold_mosaics: true });
        assert_eq!(options.background, [0, 0, 255]);
        assert_eq!(options.align, Align::parse("top-left"));
        assert!(options.hold_mosaics && !options.objects);
    }

    /// A picture that is none of the five formats is refused with the
    /// converter's reason, not a panic.
    #[test]
    fn bytes_of_no_picture_are_refused() {
        assert!(matches!(converted(b"not a picture", PictureChoice::default()), FromDecoder::PictureRefused(_)));
    }

    /// A 24-bit BMP of 64 by 48 pixels, written by hand.
    fn bmp() -> Vec<u8> {
        let (width, height) = (64_u32, 48_u32);
        let row = (width * 3).next_multiple_of(4);
        let mut bytes = b"BM".to_vec();
        for value in [54 + row * height, 0, 54, 40, width, height] {
            bytes.extend(value.to_le_bytes());
        }
        bytes.extend(1_u16.to_le_bytes());
        bytes.extend(24_u16.to_le_bytes());
        for value in [0_u32, row * height, 2835, 2835, 0, 0] {
            bytes.extend(value.to_le_bytes());
        }
        for y in 0..height {
            for x in 0..width {
                bytes.extend([u8::try_from(x * 4).unwrap(), u8::try_from(y * 5).unwrap(), 200]);
            }
        }
        bytes
    }

    /// A picture becomes page 100, the page `neoteletext:picture#100`
    /// opens, in both forms, within the protocol's bound.
    #[test]
    fn a_picture_becomes_page_100_in_both_forms() {
        let FromDecoder::PictureConverted { tti, t42, summary } = converted(&bmp(), PictureChoice { level: 0, ..PictureChoice::default() }) else {
            panic!("not converted");
        };
        assert!(String::from_utf8(tti).unwrap().lines().any(|line| line == "PN,10000"));
        assert!(!t42.is_empty() && t42.len().is_multiple_of(42));
        assert!(summary.starts_with("8 colours"), "{summary}");
    }
}
