//! The standalone runner's network file reader (script-api.md section
//! 16; `script_run::network`): arbitrary bytes are read or refused, never
//! a panic; every answer read is one the web client's own writer takes;
//! and the file written from what was read is read back the same, which
//! is what makes a recording replay.
#![no_main]

use libfuzzer_sys::fuzz_target;
use protocol::web::write_answer;
use script_run::limits::{MAX_NETWORK_ENTRIES, MAX_NETWORK_STEPS};
use script_run::network::{parse, write};

fuzz_target!(|data: &[u8]| {
    let Ok(entries) = parse(data) else { return };
    assert!(entries.len() <= MAX_NETWORK_ENTRIES);
    for entry in &entries {
        assert!(entry.replies.len() <= MAX_NETWORK_STEPS);
        for reply in &entry.replies {
            write_answer(&mut std::io::sink(), &reply.answer).expect("an answer of a network file is one a web client may send");
        }
    }
    // A method or an address is a word of the file: written back, it is
    // the same word, so the file reads back as the same entries.
    assert_eq!(parse(&write(&entries)).as_ref(), Ok(&entries), "a network file written from its entries reads back the same");
});
