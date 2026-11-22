//! The live network (script-api.md section 16, "Answers the network":
//! `--network live` "through the real web client against a local server
//! it starts from `--serve DIR`", and `--network record FILE`): a page
//! whose script fetches files of its own channel, answered by the real
//! web client, the program `neoteletext-fetcher`, from the runner's own
//! server of the channel's folder.
//!
//! The web client is a program of its own, started confined: these tests
//! need it built beside the runner (`cargo build -p neoteletext-fetcher`,
//! which a build of the whole workspace does) in a build folder the
//! system lets make a user namespace, and fail saying so without it.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const PROGRAM: &str = env!("CARGO_BIN_EXE_neoteletext-script-run");

fn folder() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/pages/lighthouse")
}

fn runner(arguments: &[&str]) -> Output {
    Command::new(PROGRAM).arg("run").args(arguments).output().unwrap()
}

fn said(output: &Output) -> String {
    format!("{}\n{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr))
}

/// The page's own lines of a run's output, without their ticks: a live
/// answer comes when it comes.
fn page_lines(all: &str) -> Vec<String> {
    let mut lines: Vec<String> = all.lines().filter_map(|line| line.split_once(" page log ").map(|(_, text)| text.to_owned())).collect();
    lines.sort();
    lines
}

/// A live run fetches through the real web client from the runner's own
/// server; a recording of it is a network file whose replay, with no
/// socket and no web client, gives the page the same answers.
#[test]
fn a_live_run_fetches_from_the_served_folder_and_its_recording_replays() {
    let folder = folder();
    let folder = folder.to_str().unwrap();
    let recording = Path::new(env!("CARGO_TARGET_TMPDIR")).join("script-run-live-recording.txt");
    let _ = std::fs::remove_file(&recording);
    let output = runner(&[folder, "--ticks", "600", "--network", "record", recording.to_str().unwrap(), "--serve", folder]);
    let all = said(&output);
    assert_eq!(output.status.code(), Some(0), "{all}");
    assert!(all.contains("NOT confined (the live network is asked for)"), "{all}");
    let heard = page_lines(&all);
    assert_eq!(heard, ["light 200 application/json 12 white", "missing 404"], "{all}");
    assert!(all.contains("network: 2 requests, 0 unanswered") && all.contains("(live: not the same every run)"), "{all}");

    // The recording names the server's address, which the replay is told
    // is the channel's.
    let recorded = std::fs::read_to_string(&recording).unwrap();
    let address = recorded.lines().find_map(|line| line.strip_prefix("request fetch GET ")).and_then(|url| url.strip_suffix("light.json")).unwrap().to_owned();
    assert!(address.starts_with("http://127.0.0.1:") && recorded.contains("head 200") && recorded.contains("head 404"), "{recorded}");
    let replayed = runner(&[folder, "--ticks", "600", "--network", "replay", recording.to_str().unwrap(), "--channel-url", &address, "--unconfined"]);
    let all = said(&replayed);
    assert_eq!(replayed.status.code(), Some(0), "{all}");
    assert_eq!(page_lines(&all), heard, "{all}");
}
