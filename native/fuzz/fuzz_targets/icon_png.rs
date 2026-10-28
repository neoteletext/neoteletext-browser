//! A PNG the icon's reader is given, as the release tool reads back what
//! `iconutil` wrote (decision 124): whatever it holds, reading it never
//! panics and never makes a picture larger than any icon, and a picture
//! read is written and read back as it was.
#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(image) = icon::png::decode(data) {
        assert!(image.width() <= icon::MAX_SIDE && image.height() <= icon::MAX_SIDE);
        let again = icon::png::encode(&image).expect("a picture read is written");
        assert_eq!(icon::png::decode(&again).as_ref(), Ok(&image));
    }
});
