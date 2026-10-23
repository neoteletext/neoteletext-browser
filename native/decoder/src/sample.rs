//! Sample page files for examples and tests: TTI, the engine's page file
//! format, with a coloured title, a few lines of text and a band of
//! mosaics on each page.

use std::fmt::Write;

/// Alpha colour codes 1 to 7 (EN 300 706 Table 26), written as TTI's
/// escape followed by the code plus 0x40.
const COLOURS: [char; 6] = ['A', 'B', 'C', 'D', 'E', 'F'];
/// Mosaic colour codes 0x11 to 0x17, and a run of solid mosaic blocks.
const MOSAICS: [char; 6] = ['Q', 'R', 'S', 'T', 'U', 'V'];
const SOLID_BLOCKS: &str = "\x7f\x7f\x7f\x7f\x7f\x7f\x7f\x7f\x7f\x7f";

/// One TTI page per number in `pages`, titled `titles[i % len]`.
#[must_use]
pub fn tti(pages: &[u16], titles: &[&str]) -> Vec<u8> {
    let mut out = String::new();
    // With no titles, every page is untitled.
    let titles = titles.iter().copied().cycle().chain(core::iter::repeat(""));
    for (((&page, colour), mosaic), title) in pages.iter().zip(COLOURS.iter().cycle()).zip(MOSAICS.iter().cycle().skip(1)).zip(titles) {
        let _ = write!(
            out,
            "PN,{page:03X}00\nSC,0000\nPS,8000\nOL,1,\x1b{colour}\x1b\x4d{title}\nOL,4,\x1bGPage {page:03X} of the sample channel\nOL,6,\x1bFThe teletext engine decoded and drew this.\nOL,10,\x1b{mosaic}{SOLID_BLOCKS}\nOL,11,\x1b{mosaic}{SOLID_BLOCKS}\n\n"
        );
    }
    out.into_bytes()
}
