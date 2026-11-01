//! A `neottx://` link from outside the browser (REQ-1672): what it
//! wraps is always a web address.
#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Ok(text) = core::str::from_utf8(data) else { return };
    if let Ok(address) = neoteletext::address::outside_link(text) {
        assert!(address.file.is_none(), "{address:?}");
        assert!(address.text.starts_with("https://") || address.text.starts_with("http://"), "{address:?}");
    }
});
