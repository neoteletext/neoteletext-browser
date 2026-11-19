//! A decoder in snapshot mode (REQ-6885, neoteletext-common's
//! `docs/tab-thumbnails.md` section 10): arbitrary bytes and a place are
//! drawn, and every snapshot drawn holds the blanking property: its source,
//! drawn again, gives the same image with reveal off, and drawing it again
//! changes nothing more, so nothing hidden was left to blank.
#![no_main]

use decoder::snapshot::draw;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Some((place, bytes)) = data.split_first_chunk::<4>() else { return };
    let page = 0x100 | (u16::from_be_bytes([place[0], place[1]]) & 0x7FF);
    let sub = u16::from_be_bytes([place[2], place[3]]) & 0x3F7F;
    let Ok(first) = draw(page, sub, bytes) else { return };
    let second = draw(page, first.sub, &first.source).expect("a blanked source draws again");
    assert_eq!(second.image, first.image, "the same image with reveal off");
});
