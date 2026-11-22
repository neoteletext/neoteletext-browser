//! A script file's manifest as the decoder reads it, before any of the
//! script runs anywhere (script-api.md section 3.1 and section 11.4,
//! target 4; `decoder::scripts::manifest::read`). Whatever the bytes, the
//! reader does not panic; what it takes is within every bound the script
//! link holds a context's opening to, so that a manifest the decoder
//! accepts never makes an opening the link refuses; and only the head of
//! the file decides: bytes after the manifest's end change nothing.
#![no_main]

use decoder::scripts::limits::FIRST_API;
use decoder::scripts::manifest::read;
use libfuzzer_sys::fuzz_target;
use protocol::script::{ContextFlags, ContextId, ContextOpen, Level, MAX_KEY_ASKS, MAX_KEY_NAME_BYTES, MAX_LANGUAGE_BYTES, MAX_TICK_DIVISOR, Renderer};

/// Bytes past any manifest: a script's own text.
const TAIL: &[u8] = b"\nfor (;;) {} /* */ \"*/\"";

fuzz_target!(|data: &[u8]| {
    let Ok(manifest) = read(data) else { return };
    assert!(data.starts_with(b"/*! nt"), "a file that does not open with the manifest was read");
    assert!(manifest.api >= FIRST_API);
    assert!(manifest.tick <= MAX_TICK_DIVISOR);
    assert!(manifest.keys.len() <= MAX_KEY_ASKS);
    for (index, key) in manifest.keys.iter().enumerate() {
        assert!(!key.is_empty() && key.len() <= MAX_KEY_NAME_BYTES && !key.chars().any(char::is_control), "{key:?}");
        assert!(manifest.keys.iter().skip(index + 1).all(|other| other != key), "{key:?} is claimed twice");
    }
    assert!(manifest.language.as_ref().is_none_or(|language| !language.is_empty() && language.len() <= MAX_LANGUAGE_BYTES));
    // What the decoder took, the link takes: a page context opened with
    // the manifest's claims is an opening its check passes.
    let open = ContextOpen {
        context: ContextId::Page,
        api: manifest.api,
        renderer: manifest.renderer.unwrap_or(Renderer::Teletext),
        level: Level::TwoFive,
        page: 0x100,
        subcode: 0,
        flags: ContextFlags { text: manifest.text, wheel: manifest.wheel, network: manifest.network, ..ContextFlags::default() },
        tick_divisor: manifest.tick,
        language: manifest.language.clone().unwrap_or_default(),
        channel_url: "https://example.test/chan/".into(),
        page_url: "https://example.test/chan/100.tti".into(),
        script_url: "https://example.test/chan/page.js".into(),
        channel_title: "Example".into(),
        seed: [0; 32],
        epoch_seconds: 0,
        zone_minutes: 0,
        keys: manifest.keys.clone(),
        sounds: Vec::new(),
        source_bytes: u32::try_from(data.len()).unwrap_or(u32::MAX),
    };
    if let Err(error) = open.check() {
        panic!("a manifest the decoder took makes an opening the link refuses: {error:?}\n{manifest:?}");
    }
    // The file's head alone decides.
    let mut longer = data.to_vec();
    longer.extend_from_slice(TAIL);
    assert_eq!(read(&longer).as_ref(), Ok(&manifest), "bytes after the manifest changed what it says");
});
