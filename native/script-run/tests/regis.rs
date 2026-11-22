//! The ReGIS API's two worked examples (script-api.md section 6.4; work
//! packages R1 and R2, rdm TODO-7298 and TODO-7300) run by the standalone
//! runner against their kept results: the scripts as the specification
//! has them (`native/script/tests/regis_page/`), their ReGIS drawn by the
//! decoder's own engine with the decoder's own fonts.

use std::path::Path;
use std::process::{Command, Output, Stdio};

const PROGRAM: &str = env!("CARGO_BIN_EXE_neoteletext-script-run");

/// Runs one of the examples for `ticks` against the kept results of
/// `page`, with pictures at `pictures`.
fn example(script: &str, page: &str, ticks: &str, pictures: &str) -> Output {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let kept = manifest.join("tests/pages").join(page);
    Command::new(PROGRAM)
        .arg("run")
        .arg(manifest.join("../script/tests/regis_page").join(script))
        .args(["--ticks", ticks, "--sound-allowed", "--png-at", pictures, "--input"])
        .arg(kept.join("input.txt"))
        .arg("--expect")
        .arg(kept.join("expect"))
        .arg("--unconfined")
        .stdin(Stdio::null())
        .output()
        .unwrap()
}

fn said(output: &Output) -> String {
    format!("{}{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr))
}

/// The page: a background with its title, a caption, and a button whose
/// release navigates. The run gives no `painted` report: the example
/// answers one by playing sound 1, and a JavaScript page with no page
/// file has no way yet to bring a sound (the specification's section 18),
/// so the call would be refused and the kept log would hold an error's
/// stack, whose line numbers move with every change to the prelude. The
/// script library's own test of the example gives the report and hears
/// the sound.
#[test]
fn the_regis_page_example_matches_its_kept_run() {
    let output = example("example-page.js", "regis-page", "40", "0,39");
    let all = said(&output);
    assert_eq!(output.status.code(), Some(0), "{all}");
    assert!(all.contains("matched the 5 kept files"), "{all}");
    assert!(all.contains("runner note navigate 106"), "the button's release navigates: {all}");
}

/// The game: drawings moved by their properties, one commit a step.
#[test]
fn the_regis_game_example_matches_its_kept_run() {
    let output = example("example-game.js", "regis-game", "100", "0,99");
    let all = said(&output);
    assert_eq!(output.status.code(), Some(0), "{all}");
    assert!(all.contains("matched the 5 kept files"), "{all}");
}
