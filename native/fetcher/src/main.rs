//! The fetcher process (`architecture.md` section 6.1): it holds the
//! network connections of one channel and nothing else. The engine's web
//! module opens the channel, reads its contents file, fetches every page
//! the contents file lists in that order at the engine's polite rate, and
//! holds the channel's push connection; this program only moves bytes
//! between the agent and the sockets, and sends what arrives to the
//! trusted core on standard output.
//!
//! Usage: `neoteletext-fetcher <channel address> [--snapshot <PPP> <SSSS>]
//! [--allow-local <ip>]... [--proxy <url>] [--no-http3]`; one process serves
//! every tab of its channel in one jar that the trusted core adds to it,
//! each message on its pipes naming its tab (REQ-6980), and `--no-http3`
//! keeps every tab's push connection on HTTP/1.1 (REQ-6982); with `--snapshot` it fetches
//! one page as a stranger would and speaks only `protocol::snapshot`
//! (REQ-6884). A server on this computer or the local network is
//! refused unless named with `--allow-local` (the engine's REQ-SEC-006).
//! With `--proxy http://host:port` or `socks5://host:port` every
//! connection goes through that proxy (requirement 19.10); it is given only
//! here, when the fetcher starts (requirement 19.12).

// On Windows, a desktop program: no console window opens with it. The
// processes talk through the pipes they are given, not a console.
#![cfg_attr(windows, windows_subsystem = "windows")]
// No panicking constructs: this crate reads input from outside the
// process. The same list is in each such crate; scripts/check.sh checks
// that they agree.
#![deny(
    clippy::indexing_slicing,
    clippy::string_slice,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::arithmetic_side_effects,
    clippy::unreachable,
    clippy::todo,
    clippy::unimplemented
)]

use std::process::ExitCode;

use fetcher::{arguments, run, snapshot};

fn main() -> ExitCode {
    // First, before any thread: the network and nothing else (requirement 3.3).
    match confine::apply(confine::Role::Network, None) {
        Ok(report) => eprintln!("neoteletext-fetcher: confined: {report}"),
        Err(error) => {
            eprintln!("neoteletext-fetcher: {error}");
            return ExitCode::FAILURE;
        }
    }
    let read = match arguments(std::env::args().skip(1)) {
        Ok(read) => read,
        Err(error) => {
            eprintln!("neoteletext-fetcher: {error}");
            return ExitCode::FAILURE;
        }
    };
    // A snapshot is fetched by a process started for it alone, speaking
    // only protocol::snapshot (REQ-6884).
    let ran = match read.snapshot {
        Some((page, sub)) => snapshot(&read.channel, page, sub, &read.allowed, read.proxy),
        None => run(&read.channel, &read.allowed, read.proxy, read.http3),
    };
    match ran {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("neoteletext-fetcher: {error}");
            ExitCode::FAILURE
        }
    }
}
