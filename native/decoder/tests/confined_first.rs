//! A channel's decoder, confined as the broker confines it, says it is
//! confined before it reads anything (finding F-12): the trusted core
//! counts a decoder that ends before this as one that could not start,
//! against an update's trial, and one that ends after it as a channel's.
//!
//! Nothing here is timed. A deadline from the start would also time the
//! system admitting a newly built program: on macOS, Gatekeeper scans each
//! new program once before its first instruction runs, one scan at a time
//! for the whole computer, and a decoder queued behind the workspace's new
//! test programs sat at its first instruction for more than ten seconds
//! (24 October 2026), which says nothing of the decoder.

use std::ffi::OsString;
use std::io::BufReader;
use std::path::Path;

use confine::{Role, Spawn};
use protocol::decoder::{FIRST_STORE_PAGES, FromDecoder, read_from_decoder};

#[test]
fn a_decoder_says_it_is_confined_before_it_reads_anything() {
    let program = Path::new(env!("CARGO_BIN_EXE_neoteletext-decoder"));
    let arguments = [OsString::from("--store-pages"), OsString::from(FIRST_STORE_PAGES.to_string())];
    let environment = confine::environment(Role::Decoder);
    let mut process =
        confine::spawn(&Spawn { program, arguments: &arguments, role: Role::Decoder, environment: &environment, local_network: false, writable: None })
            .unwrap();
    // Its input ends at once, with no channel data: a decoder that read
    // before saying it is confined finds the end and ends having said
    // nothing, so its first message is read until it comes or the decoder
    // ends, with no deadline.
    drop(process.stdin.take());
    let output = process.stdout.take().unwrap();
    let first = read_from_decoder(&mut BufReader::new(output)).map_err(|error| error.to_string());
    let _ = process.kill();
    let _ = process.wait();
    assert_eq!(first, Ok(Some(FromDecoder::Confined)), "its first message, with no data given");
}
