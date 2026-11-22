//! The example channel of script pages (rdm TODO-7310, work package X1 of
//! `neoteletext-common/docs/script-api.md`) in the standalone runner
//! (section 16): every page of the channel of the repository
//! neoteletext-server-channel-scriptdemo run against the results kept
//! there, so that a change to how any part of the API draws, sounds or
//! logs shows as a page of the channel that no longer matches.
//!
//! The channel keeps its own table of runs, `runs/runs.txt`, a line for
//! each: the page, the ticks it is stepped and the ticks a picture is
//! kept after. A run's folder `runs/PAGE` holds its input script and its
//! network table, when it has them, and its kept results.

use std::path::{Path, PathBuf};
use std::process::Command;

const PROGRAM: &str = env!("CARGO_BIN_EXE_neoteletext-script-run");
/// The environment variable that names the channel repository's
/// checkout, and where it is looked for without it, under the home
/// folder.
const CHECKOUT_VARIABLE: &str = "NEOTELETEXT_SCRIPTDEMO";
const CHECKOUT_DEFAULT: &str = "devel/neoteletext-server-channel-scriptdemo-worktrees/reqresp";
/// The fewest runs the table holds: one for each page the channel had
/// when this test was written, so that a table cut short does not pass.
const FEWEST_RUNS: usize = 15;

fn checkout() -> PathBuf {
    let checkout = std::env::var_os(CHECKOUT_VARIABLE).map_or_else(|| PathBuf::from(std::env::var_os("HOME").unwrap()).join(CHECKOUT_DEFAULT), PathBuf::from);
    assert!(checkout.join("runs/runs.txt").is_file(), "no kept runs at {}: set {CHECKOUT_VARIABLE} to the channel repository's checkout", checkout.display());
    checkout
}

/// One line of the table: a page, its ticks and its pictures.
fn runs(table: &str) -> Vec<(String, String, String)> {
    table
        .lines()
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(|line| {
            let fields: Vec<&str> = line.split_ascii_whitespace().collect();
            let [page, ticks, pictures] = fields.as_slice() else { panic!("a line of runs.txt is `PAGE TICKS PICTURES`, not `{line}`") };
            ((*page).to_owned(), (*ticks).to_owned(), (*pictures).to_owned())
        })
        .collect()
}

/// The arguments of one run: the channel's folder with its page, the
/// run's input and network table where its folder has them, and its kept
/// results.
fn arguments(checkout: &Path, page: &str, ticks: &str, pictures: &str) -> Vec<String> {
    let folder = checkout.join("runs").join(page);
    let path = |path: PathBuf| path.to_str().unwrap().to_owned();
    let mut arguments = vec![path(checkout.join("channel")), "--page".into(), page.into(), "--ticks".into(), ticks.into(), "--png-at".into(), pictures.into()];
    if folder.join("input.txt").is_file() {
        arguments.extend(["--input".into(), path(folder.join("input.txt"))]);
    }
    if folder.join("network.txt").is_file() {
        arguments.extend(["--network".into(), "mock".into(), path(folder.join("network.txt"))]);
    }
    arguments.extend(["--expect".into(), path(folder.join("expect"))]);
    arguments
}

/// Section 16, "Compares with kept results", for every page of the
/// example channel: each run of the channel's table ends with status 0,
/// ran and matched, and its log holds no error of a script.
#[test]
#[ignore = "needs the checkout of neoteletext-server-channel-scriptdemo's branch reqresp, at ~/devel/neoteletext-server-channel-scriptdemo-worktrees/reqresp or where NEOTELETEXT_SCRIPTDEMO names it"]
fn every_page_of_the_example_channel_matches_its_kept_run() {
    let checkout = checkout();
    let runs = runs(&std::fs::read_to_string(checkout.join("runs/runs.txt")).unwrap());
    assert!(runs.len() >= FEWEST_RUNS, "runs.txt holds {} runs, fewer than the channel's {FEWEST_RUNS} pages", runs.len());
    let mut failures = Vec::new();
    for (page, ticks, pictures) in &runs {
        let output = Command::new(PROGRAM).arg("run").args(arguments(&checkout, page, ticks, pictures)).arg("--unconfined").output().unwrap();
        let said = format!("{}\n{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
        if output.status.code() != Some(0) || !said.contains("matched the ") {
            failures.push(format!("page {page}: status {:?}\n{said}", output.status.code()));
        }
        let log = std::fs::read_to_string(checkout.join("runs").join(page).join("expect/log.txt")).unwrap();
        for line in log.lines().filter(|line| line.contains(" error ") || line.contains(" warn ")) {
            failures.push(format!("page {page}: its kept log holds: {line}"));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
