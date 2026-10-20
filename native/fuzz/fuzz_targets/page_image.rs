//! The page images the interface process reads from the trusted core.
#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    core::hint::black_box(neoteletext_interface::image::decode(data).is_ok());
});
