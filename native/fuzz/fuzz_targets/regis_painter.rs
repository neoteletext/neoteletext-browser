//! A script's ReGIS painter program as the script host encodes it into a
//! drawing's contents or a macrograph's body (script-api.md section 6
//! and section 11.4, target 22, its ReGIS half; operations 207 RG_DEFINE
//! and 211 RG_MACROGRAPH; `neoteletext_script::host::
//! regis_painter_contents`). The words are a script's `Int32Array`.
//! Whatever they are, the encoder does not panic, takes or refuses the
//! program whole (the same program gives the same answer), and writes
//! no more than a drawing's contents may hold.
#![no_main]

use libfuzzer_sys::fuzz_target;
use neoteletext_script::host::regis_painter_contents;
use neoteletext_script::limits::regis::REGIS_MAX_CONTENTS_BYTES;

/// Language tags: the default, others the engine knows, and none.
const LANGUAGES: [&str; 4] = ["en", "de", "", "zz-qq-not-a-tag-at-all"];

fuzz_target!(|data: &[u8]| {
    let Some((&flags, rest)) = data.split_first() else { return };
    let program: Vec<i32> = rest
        .chunks(4)
        .map(|word| {
            let mut bytes = [0; 4];
            bytes[..word.len()].copy_from_slice(word);
            // Small words most of the time: the operations' own numbers.
            if flags & 0x80 == 0 { i32::from(bytes[0] % 64) - 8 } else { i32::from_le_bytes(bytes) }
        })
        .collect();
    let language = LANGUAGES[usize::from(flags) % LANGUAGES.len()];
    let macrograph = flags & 4 != 0;
    let written = regis_painter_contents(&program, language, macrograph);
    assert_eq!(regis_painter_contents(&program, language, macrograph), written, "the same program encoded differently");
    if let Ok(bytes) = &written {
        assert!(bytes.len() <= REGIS_MAX_CONTENTS_BYTES, "{} bytes of contents", bytes.len());
    }
});
