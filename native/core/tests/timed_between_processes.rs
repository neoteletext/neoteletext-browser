//! The timed test between the browser's real processes (neoteletext-common's
//! `docs/page-rules.md` RULE-66; REQ-6535), kept as an ignored test that
//! runs `scripts/timed-test.sh`: the interface, the trusted core, a confined
//! decoder and the sound helper as four processes on their real pipes, the
//! music player's page 101 open, keys pressed through XTEST, and six sets
//! of figures, each change of DEC-6580 and DEC-6635 switched off and then
//! on. It fails when a bound of RULE-64 or RULE-65 is passed with every
//! change on.
//!
//! Run by hand, with what it needs named:
//!
//! ```text
//! TIMED_RUN=~/devel/b2t TIMED_SERVER=.../teletext-server TIMED_PLAYER=.../musicplayer \
//!     cargo test -p neoteletext --test timed_between_processes -- --ignored
//! ```
#![allow(clippy::expect_used, clippy::panic, reason = "a test")]

use std::process::Command;

/// RULE-66: it needs a machine with Xvfb, `PipeWire`, `xdotool` and Python, a
/// server at program protocol 15 and the music player, which no gate has
/// on every machine, and it takes a quarter of an hour.
#[test]
#[ignore = "needs Linux, Xvfb, PipeWire, xdotool, a protocol 15 server and the music player (TIMED_RUN, TIMED_SERVER, TIMED_PLAYER), and a quarter of an hour"]
fn the_timed_test_between_the_real_processes() {
    let needed = |name: &str| std::env::var(name).unwrap_or_else(|_| panic!("{name} names what the timed test needs (see this file)"));
    let script = concat!(env!("CARGO_MANIFEST_DIR"), "/../../scripts/timed-test.sh");
    let status =
        Command::new(script).args([needed("TIMED_RUN"), needed("TIMED_SERVER"), needed("TIMED_PLAYER")]).status().expect("the timed test's script runs");
    assert!(status.success(), "a bound of RULE-64 or RULE-65 was passed, or the test could not run: see its output");
}
