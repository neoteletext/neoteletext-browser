//! The component vocabulary in the standalone runner (script-api.md
//! sections 8 and 16; work package U1): a teletext page of all thirteen
//! kinds run against its kept results.

use std::path::Path;
use std::process::{Command, Output};

const PROGRAM: &str = env!("CARGO_BIN_EXE_neoteletext-script-run");

/// Runs the program in its own process, not confined.
fn runner(arguments: &[&str]) -> Output {
    Command::new(PROGRAM).arg("run").args(arguments).arg("--unconfined").output().unwrap()
}

fn said(output: &Output) -> String {
    format!("{}\n{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr))
}

/// Section 8, the page of all thirteen kinds (work package U1's
/// `native/script/tests/ui/thirteen.js`) run against its kept results,
/// with scripted keys, typed text and a press of the pointer: each
/// component's event is in the log, and the page plane the components
/// write is drawn by the decoder's engine. The runner's stand-in for the
/// decoder's side of a step does not draw the page's local enhancement
/// data, so the components' backgrounds, highlights and controls are not
/// in the pictures, and the report must say so, never pass silently.
#[test]
fn the_page_of_all_thirteen_kinds_matches_its_kept_run() {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let kept = manifest.join("tests/pages/thirteen");
    let output = runner(&[
        manifest.join("../script/tests/ui/thirteen.js").to_str().unwrap(),
        "--page",
        "100",
        "--ticks",
        "70",
        "--input",
        kept.join("input.txt").to_str().unwrap(),
        "--png-at",
        "0,45,69",
        "--expect",
        kept.join("expect").to_str().unwrap(),
    ]);
    let all = said(&output);
    assert_eq!(output.status.code(), Some(0), "{all}");
    assert!(all.contains("matched the 6 kept files"), "{all}");
    let log = std::fs::read_to_string(kept.join("expect/log.txt")).unwrap();
    for line in [
        "made 13 components, 12 controls, 31 triplets; focus on story",
        "move menu 1",
        "move menu 3",
        "move tides 1",
        "change sound true",
        "change volume 5",
        "change name \"Mary Rose\"",
        "change name \"Mary Ros\"",
        "activate menu 3",
    ] {
        assert!(log.contains(line), "the kept log has no line `{line}`:\n{log}");
    }
    // The decoder's own code draws all of it: the overlay's colours behind
    // the status line, the chosen row, the button and the field in focus.
    assert!(!all.contains("NOT DRAWN"), "{all}");
}
