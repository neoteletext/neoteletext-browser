//! The component vocabulary on NAPLPS in the standalone runner
//! (script-api.md sections 8 and 16; work package U3): a NAPLPS page of
//! all thirteen kinds run against its kept results.

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

/// Section 8 on NAPLPS, the page of all thirteen kinds (work package
/// U3's `native/script/tests/naplps_page/thirteen.js`) run against its
/// kept results, with scripted keys, typed text and a press of the
/// pointer on a region of the menu: each component's event is in the
/// kept log, line by line, and the pictures are what the runner's
/// receiver presents. The runner's stand-in for the decoder's side of a
/// step does not apply the cast's objects, so the highlights, the thumb,
/// the caret and the focus ring are not in the pictures, and the report
/// must say so, never pass silently. With `NT_BLESS` set the kept
/// results are written.
#[test]
fn the_naplps_page_of_all_thirteen_kinds_matches_its_kept_run() {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let kept = manifest.join("tests/pages/naplps-thirteen");
    let mut arguments = vec![
        manifest.join("../script/tests/naplps_page/thirteen.js").to_str().unwrap().to_owned(),
        "--page".into(),
        "100".into(),
        "--ticks".into(),
        "70".into(),
        "--input".into(),
        kept.join("input.txt").to_str().unwrap().to_owned(),
        "--png-at".into(),
        "0,45,69".into(),
        "--expect".into(),
        kept.join("expect").to_str().unwrap().to_owned(),
    ];
    if std::env::var_os("NT_BLESS").is_some() {
        arguments.push("--bless".into());
    }
    let arguments: Vec<&str> = arguments.iter().map(String::as_str).collect();
    let output = runner(&arguments);
    let all = said(&output);
    assert_eq!(output.status.code(), Some(0), "{all}");
    assert!(all.contains("NOT DRAWN") && all.contains("NAPLPS objects"), "{all}");
    let log = std::fs::read_to_string(kept.join("expect/log.txt")).unwrap();
    let events: Vec<&str> = log.lines().filter_map(|line| line.split_once("] ").map_or(Some(line), |(_, rest)| Some(rest))).collect();
    for line in [
        "made 13 components, 16 objects, 5 symbols; focus on story",
        "move menu 1",
        "move menu 3",
        "move tides 1",
        "change sound true",
        "change volume 5",
        "change name \"Mary Rose\"",
        "change name \"Mary Ros\"",
        "activate menu \"ship\"",
    ] {
        assert!(events.iter().any(|event| event.contains(line)), "the kept log has no line `{line}`:\n{log}");
    }
}
