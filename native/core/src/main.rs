//! The trusted core (design section 5): it keeps the tabs, their history
//! and their channels, reads each channel's data and hands it to that
//! channel's decoder at a fixed pace, forwards the pages that arrive,
//! checks every frame, encodes page images, and decides what the trusted
//! strip shows. It never parses teletext data and never draws; the
//! interface never reaches a decoder.
//!
//! Usage: `neoteletext [--allow-local <ip>]... [address]...`, each address
//! a web channel's folder (`http://host/folder/`), a `file://` address, or
//! an absolute path to a recording, a page file or a folder of page files.
//! A channel on this computer or the local network is fetched only from
//! the addresses given with `--allow-local` or allowed in the settings
//! (the engine's REQ-SEC-006). With no address, the tab opens the home
//! address of the settings, which the trusted core keeps between runs
//! (`settings-home-console-proxy.md`). The decoder and the interface are
//! found beside this program.

// On Windows, a desktop program: no console window opens with it. The
// processes talk through the pipes they are given, not a console.
#![cfg_attr(windows, windows_subsystem = "windows")]
// No panicking constructs: this crate reads input from outside the
// process. The same list is in each such crate; scripts/check.sh checks
// that they agree.
#![deny(
    clippy::indexing_slicing,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::arithmetic_side_effects,
    clippy::unreachable,
    clippy::todo,
    clippy::unimplemented
)]

mod broker;
mod session;
mod source;
mod trusted_core;

use std::process::ExitCode;

use confine::Role;
use confine::broker::Broker;
use neoteletext::settings as stored;

/// The trusted core: confined first (decision 56), then the browser.
fn core() -> Result<(), String> {
    let report = confine::apply(Role::Core, stored::folder().as_deref())?;
    eprintln!("neoteletext: the trusted core is confined: {report}");
    let broker = Broker::from_parent().map_err(|error| error.to_string())?;
    trusted_core::run(&broker, std::env::args().skip(2), &report.to_string())
}

fn main() -> ExitCode {
    let step = if std::env::args().nth(1).as_deref() == Some(broker::CORE_FLAG) { core() } else { broker::broker() };
    match step {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("neoteletext: {error}");
            ExitCode::FAILURE
        }
    }
}
