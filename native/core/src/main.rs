//! The trusted core (design section 5): it keeps the tabs, their history
//! and their channels, reads each channel's data and hands it to that
//! channel's decoder at a fixed pace, forwards the pages that arrive,
//! checks every frame, encodes page images, and decides what the trusted
//! strip shows. It never parses teletext data and never draws; the
//! interface never reaches a decoder.
//!
//! Usage: `neoteletext --version`, which prints the version and ends, or
//! `neoteletext [--allow-local <ip>]... [address]...`, each address
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
    clippy::string_slice,
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

/// The version this program was built as: a release's, or a development
/// build's with its count (the build script, REQ-7047). The one place the
/// program reads it; the broker's mark is made of it and checked against it.
const VERSION: &str = env!("NEOTELETEXT_VERSION");
/// Alone on the command line, it prints the version and nothing starts.
const VERSION_FLAG: &str = "--version";

/// The trusted core: confined first (decision 56), then the browser.
fn core() -> Result<(), String> {
    let report = confine::apply(Role::Core, stored::folder().as_deref())?;
    eprintln!("neoteletext: the trusted core is confined: {report}");
    // RULE-66: the timed test's marks carry each thread's use of the processor.
    #[cfg(all(feature = "timed-test", target_os = "linux"))]
    protocol::timed_test::read_threads_with(confine::scheduling::thread_usage);
    let broker = Broker::from_parent().map_err(|error| error.to_string())?;
    trusted_core::run(&broker, std::env::args().skip(2), &report.to_string())
}

fn main() -> ExitCode {
    let mut arguments = std::env::args_os().skip(1);
    let version_asked = arguments.next().is_some_and(|argument| argument == VERSION_FLAG) && arguments.next().is_none();
    let step = if version_asked {
        broker::print_version()
    } else if std::env::args().nth(1).as_deref() == Some(broker::CORE_FLAG) {
        core()
    } else {
        broker::broker()
    };
    match step {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("neoteletext: {error}");
            ExitCode::FAILURE
        }
    }
}
