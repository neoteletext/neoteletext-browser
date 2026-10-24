//! A channel's decoder, confined as the broker confines it, says it is
//! confined before it reads anything (finding F-12): the trusted core
//! counts a decoder that ends before this as one that could not start,
//! against an update's trial, and one that ends after it as a channel's.

use std::ffi::OsString;
use std::io::BufReader;
use std::path::Path;
use std::sync::mpsc;
use std::time::Duration;

use confine::{Role, Spawn};
use protocol::decoder::{FIRST_STORE_PAGES, FromDecoder, read_from_decoder};

/// Far longer than a confined process takes to start and write one
/// message, which is milliseconds.
const FIRST_MESSAGE_DEADLINE: Duration = Duration::from_secs(10);

#[test]
fn a_decoder_says_it_is_confined_before_it_reads_anything() {
    let program = Path::new(env!("CARGO_BIN_EXE_neoteletext-decoder"));
    let arguments = [OsString::from("--store-pages"), OsString::from(FIRST_STORE_PAGES.to_string())];
    let environment = confine::environment(Role::Decoder);
    let mut process =
        confine::spawn(&Spawn { program, arguments: &arguments, role: Role::Decoder, environment: &environment, local_network: false, writable: None })
            .unwrap();
    // Its input is held open and given nothing: no channel data.
    let input = process.stdin.take().unwrap();
    let output = process.stdout.take().unwrap();
    let (said, first) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = said.send(read_from_decoder(&mut BufReader::new(output)).map_err(|error| error.to_string()));
    });
    let first = first.recv_timeout(FIRST_MESSAGE_DEADLINE);
    drop(input);
    let _ = process.kill();
    let _ = process.wait();
    assert_eq!(first, Ok(Ok(Some(FromDecoder::Confined))), "its first message, with no data given");
}
