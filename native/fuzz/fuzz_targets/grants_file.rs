//! `grants.txt`, the broker's kept grants (DEC-1551 item 1): what is read
//! back writes the same.
#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Ok(text) = core::str::from_utf8(data) else { return };
    let (grants, _) = neoteletext::grants::Grants::parse(text);
    assert!(grants.len() <= neoteletext::grants::MAX_GRANTS);
    assert_eq!(neoteletext::grants::Grants::parse(&grants.to_text()).0, grants);
});
