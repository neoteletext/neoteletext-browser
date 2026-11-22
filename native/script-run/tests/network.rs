//! The standalone runner's `--network mock` with the script library's
//! web APIs (script-api.md section 16, "Answers the network", and
//! section 9): a page that fetches and opens an event stream, answered
//! from a table, as a kept run.

use std::path::Path;
use std::process::Command;

const PROGRAM: &str = env!("CARGO_BIN_EXE_neoteletext-script-run");

/// "Mock and replay are deterministic and need no socket": the kept run
/// of a page whose script fetches a file, fetches one the table does
/// not have, and hears an event stream until it closes it. A change to
/// what leaves for the web client, to the tick an answer is heard on or
/// to what the script is given fails it, on any machine.
#[test]
fn a_page_that_fetches_matches_its_kept_run_with_the_mock_network() {
    let folder = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/pages/tides");
    let path = |name: &str| folder.join(name).to_str().unwrap().to_owned();
    let output = Command::new(PROGRAM)
        .arg("run")
        .args([&path(""), "--ticks", "30", "--network", "mock", &path("network.txt"), "--png-at", "0,29"])
        .args(["--expect", &path("expect"), "--unconfined"])
        .output()
        .unwrap();
    let said = format!("{}\n{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
    assert_eq!(output.status.code(), Some(0), "{said}");
    assert!(said.contains("matched the 5 kept files"), "{said}");
    // The run did what it was written to do: three requests, one with no entry.
    assert!(said.contains("network: 3 requests, 1 unanswered"), "{said}");
    for line in [
        "page log head 200 OK basic application/json https://channel.test/tides.json",
        "page log height 3 at ",
        "page log feed second id 7 from https://channel.test at ",
    ] {
        assert!(said.contains(line), "{line}: {said}");
    }
    assert!(!said.contains("never heard"), "{said}");
}
