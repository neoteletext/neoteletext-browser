//! A fetcher, confined as the broker confines it, says it is confined
//! before it opens its channel (finding F-12): the trusted core counts a
//! fetcher that ends before this as one that could not start, against an
//! update's trial, and one that ends after it as its server's doing.
//!
//! Nothing here is timed. A deadline from the start would also time the
//! system admitting a newly built program: on macOS, Gatekeeper scans each
//! new program once before its first instruction runs, one scan at a time
//! for the whole computer, and a decoder queued behind the workspace's new
//! test programs sat at its first instruction for more than ten seconds
//! (24 October 2026), which says nothing of the fetcher.

use std::ffi::OsString;
use std::io::BufReader;
use std::path::Path;

use confine::{Role, Spawn};
use protocol::fetcher::{FromFetcher, read_from_fetcher};

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
    // Closing its input ends a fetcher, and it is closed at once: one that
    // opened its channel before saying it is confined sends what it has of
    // the channel first, or ends having said nothing, so its first message
    // is read until it comes or the fetcher ends, with no deadline.
    drop(process.stdin.take());
    let output = process.stdout.take().unwrap();
    let first = read_from_fetcher(&mut BufReader::new(output)).map_err(|error| error.to_string());
    let _ = process.kill();
    let _ = process.wait();
    assert_eq!(first, Ok(Some(FromFetcher::Confined)), "its first message, before anything of its channel");
}
