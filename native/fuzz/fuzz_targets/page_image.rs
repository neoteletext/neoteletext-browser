//! The page images the interface reads (requirement 9.3, decision 61):
//! whatever the bytes, reading keeps every bound, against a held frame or
//! none, and an image read is written and read back the same.
#![no_main]

use libfuzzer_sys::fuzz_target;
use protocol::frame::{PAGE_DOTS, decode, encode};

fuzz_target!(|data: &[u8]| {
    // A held frame of stripes, so that changes have something to change.
    let held: Vec<u8> = (0..PAGE_DOTS).map(|dot| u8::from(dot % 7 == 0)).collect();
    for against in [None, Some((0_u32, held.as_slice())), protocol::frame::base(data).map(|version| (version, held.as_slice()))] {
        if let Ok(image) = decode(data, against) {
            assert_eq!(image.indices.len(), PAGE_DOTS);
            assert!(image.palette.len() <= 32 && usize::from(image.screen) < image.palette.len());
            let again = encode(&image.palette, image.screen, &image.indices, against).expect("an image read is a page");
            assert_eq!(decode(&again, against).unwrap(), image);
        }
    }
});
