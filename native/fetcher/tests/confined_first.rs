//! A fetcher, confined as the broker confines it, says it is confined
//! before it opens its channel (finding F-12): the trusted core counts a
//! fetcher that ends before this as one that could not start, against an
//! update's trial, and one that ends after it as its server's doing.

use std::ffi::OsString;
use std::io::BufReader;
use std::path::Path;
use std::sync::mpsc;
use std::time::Duration;

use confine::{Role, Spawn};
use protocol::fetcher::{FromFetcher, read_from_fetcher};

/// Far longer than a confined process takes to start and write one
/// message, which is milliseconds.
const FIRST_MESSAGE_DEADLINE: Duration = Duration::from_secs(10);

/// A channel that cannot be reached: `.invalid` is never a host (RFC 6761
/// section 6.4), so nothing the fetcher could say of it comes first.
const UNREACHABLE: &str = "https://news.invalid/";

#[test]
fn a_fetcher_says_it_is_confined_before_it_opens_its_channel() {
    let program = Path::new(env!("CARGO_BIN_EXE_neoteletext-fetcher"));
    let arguments = [OsString::from(UNREACHABLE)];
    let environment = confine::environment(Role::Network);
    let mut process =
        confine::spawn(&Spawn { program, arguments: &arguments, role: Role::Network, environment: &environment, local_network: false, writable: None })
            .unwrap();
    // Closing its input ends a fetcher: it is held open.
    let input = process.stdin.take().unwrap();
    let output = process.stdout.take().unwrap();
    let (said, first) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = said.send(read_from_fetcher(&mut BufReader::new(output)).map_err(|error| error.to_string()));
    });
    let first = first.recv_timeout(FIRST_MESSAGE_DEADLINE);
    drop(input);
    let _ = process.kill();
    let _ = process.wait();
    assert_eq!(first, Ok(Ok(Some(FromFetcher::Confined))), "its first message, before anything of its channel");
}
