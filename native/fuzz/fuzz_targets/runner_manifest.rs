//! The standalone runner's manifest reader (script-api.md section 3.1;
//! `script_run::manifest`), which stands in the runner until the
//! decoder's own (target 4 of section 11.4, `script_manifest`, work
//! package F4) exists: arbitrary bytes as a script's head are read or
//! refused, never a panic, and what is read is within the bounds a
//! context's opening has.
#![no_main]

use libfuzzer_sys::fuzz_target;
use neoteletext_script::API_VERSION;
use protocol::script::{MAX_KEY_ASKS, MAX_KEY_NAME_BYTES, MAX_LANGUAGE_BYTES, MAX_TICK_DIVISOR};
use script_run::manifest::{OPENING, read};

fuzz_target!(|data: &[u8]| {
    // Half the inputs behind the opening every manifest has, so that the
    // JSON behind it is what is explored.
    let (first, rest) = data.split_first().unwrap_or((&0, &[]));
    let source = if first % 2 == 0 { [OPENING, rest].concat() } else { rest.to_vec() };
    let Ok(manifest) = read(&source) else { return };
    assert!((1..=API_VERSION).contains(&manifest.api));
    assert!(manifest.keys.len() <= MAX_KEY_ASKS);
    assert!(manifest.keys.iter().all(|key| !key.is_empty() && key.len() <= MAX_KEY_NAME_BYTES && !key.chars().any(char::is_control)));
    assert!(manifest.tick <= MAX_TICK_DIVISOR);
    assert!(manifest.language.as_ref().is_none_or(|tag| tag.len() <= MAX_LANGUAGE_BYTES));
});
