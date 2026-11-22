//! One test file as a page's script, run in the standalone runner, and
//! what its cases reported.

use std::fmt::Write as _;
use std::path::Path;

use script_run::blobs::{Blob, find};
use script_run::{args, files, run};

use super::server::{Server, origin, substitute};
use super::tree::Case;

/// The ticks a file is given: testharness.js's 10 seconds, and its 60
/// for a file that asks for the long timeout, with room for the results
/// to be written a few lines a tick.
const TICKS: u64 = 60 * 12;
const LONG_TICKS: u64 = 60 * 62;
/// The harness's own end of a file whose tests never finish, in
/// milliseconds, before the ticks run out.
const TIMEOUT_MS: u64 = 10_000;
const LONG_TIMEOUT_MS: u64 = 60_000;
/// The ticks left after that for the results to be written: the largest
/// file here reports under ten thousand cases, forty a tick.
const REPORT_TICKS: u64 = 400;
/// The largest script the runner takes (the script library's `MAX_SCRIPT_BYTES`).
const MAX_PAGE_BYTES: usize = neoteletext_script::limits::MAX_SCRIPT_BYTES;

/// What a test file's run reported.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FileResult {
    /// The harness's status for the file: `OK`, `ERROR`, `TIMEOUT`, or
    /// why the file gave no report at all.
    pub harness: String,
    /// What says more of it: the harness's message, or the script's last
    /// error.
    pub detail: String,
    /// Each case: its name, its status and its message.
    pub tests: Vec<(String, String, String)>,
}

fn json_string(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + text.len() / 8 + 2);
    out.push('"');
    for character in text.chars() {
        match character {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\u{2028}' => out.push_str("\\u2028"),
            '\u{2029}' => out.push_str("\\u2029"),
            control if u32::from(control) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", u32::from(control));
            }
            other => out.push(other),
        }
    }
    out.push('"');
    out
}

fn read(root: &Path, path: &str) -> Option<String> {
    let text = String::from_utf8_lossy(&std::fs::read(root.join(path)).ok()?).into_owned();
    Some(if path.contains(".sub.") { substitute(&text) } else { text })
}

/// The page's script for `case`: the manifest, then one sloppy-mode
/// evaluation of the preamble, testharness.js, the scripts the file's
/// `META` names, the file and the epilogue, as wptserve's worker wrapper
/// loads them into one global scope.
fn page(root: &Path, case: &Case) -> Result<String, String> {
    let folder = case.path.rsplit_once('/').map_or("", |(folder, _)| folder);
    let file = case.path.rsplit('/').next().unwrap_or_default().replace(".any.js", ".any.worker.js");
    let url = format!("{}/{folder}/{file}{}", origin(&case.path), case.variant);
    let timeout = if case.long { LONG_TIMEOUT_MS } else { TIMEOUT_MS };
    let mut source = format!("var __WPT = {{ url: {}, timeout: {timeout} }};\n", json_string(&url));
    source.push_str(include_str!("preamble.js"));
    let mut parts = vec!["resources/testharness.js".to_owned()];
    parts.extend(case.scripts.iter().cloned());
    parts.push(case.path.clone());
    for part in parts {
        let text = read(root, &part).ok_or_else(|| format!("no file {part}"))?;
        source.push_str("\n;\n");
        source.push_str(&text);
    }
    source.push_str("\n;\n");
    source.push_str(include_str!("epilogue.js"));
    let page = format!("/*! nt {{\"api\": 1, \"renderer\": \"teletext\", \"network\": true}} */\n(0, eval)({});\n", json_string(&source));
    if page.len() > MAX_PAGE_BYTES { Err(format!("{} bytes", page.len())) } else { Ok(page) }
}

/// Runs `case` and gives what it reported.
pub fn run_case(root: &Path, case: &Case) -> FileResult {
    let page = match page(root, case) {
        Ok(page) => page,
        Err(why) => return FileResult { harness: if why.ends_with("bytes") { "TOO-LARGE".into() } else { "NO-FILE".into() }, detail: why, tests: Vec::new() },
    };
    let folder = case.path.rsplit_once('/').map_or("", |(folder, _)| folder);
    let ticks = if case.long { LONG_TICKS } else { TICKS } + REPORT_TICKS;
    let arguments: Vec<String> =
        ["run", "page.js", "--ticks", &ticks.to_string(), "--channel-url", &format!("{}/{folder}/", origin(&case.path)), "--unconfined"]
            .iter()
            .map(|text| (*text).to_owned())
            .collect();
    let options = args::parse(&arguments).unwrap_or_else(|failure| panic!("the runner's arguments: {}", failure.message));
    let name = case.path.rsplit('/').next().unwrap_or_default().replace(".any.js", ".any.worker.js");
    let blobs: Vec<Blob> = vec![(files::PAGE_SCRIPT.to_owned(), page.into_bytes()), (files::PAGE_SCRIPT_NAME.to_owned(), name.into_bytes())];
    let outcome = match run::run(&options, &blobs, Box::new(Server::new(root))) {
        Ok(outcome) => outcome,
        Err(failure) => return FileResult { harness: "REFUSED".into(), detail: failure.message, tests: Vec::new() },
    };
    let log = String::from_utf8_lossy(find(&outcome.files, run::LOG_FILE).unwrap_or_default()).into_owned();
    let mut result = FileResult { harness: "NO-REPORT".into(), ..FileResult::default() };
    let mut ended = false;
    for line in log.lines() {
        // A line is its tick, its context, its level and its text.
        let text = line.splitn(4, ' ').nth(3).unwrap_or_default();
        if let Some(status) = text.strip_prefix("WPT S ") {
            let (code, message) = status.split_once(' ').unwrap_or((status, ""));
            let harness = match code {
                "0" => "OK",
                "1" => "ERROR",
                "2" => "TIMEOUT",
                "3" => "PRECONDITION_FAILED",
                _ => "UNKNOWN",
            };
            harness.clone_into(&mut result.harness);
            message.clone_into(&mut result.detail);
        } else if let Some(test) = text.strip_prefix("WPT T ") {
            let (code, rest) = test.split_once(' ').unwrap_or((test, ""));
            let (name, message) = rest.split_once(" ||| ").unwrap_or((rest, ""));
            let status = match code {
                "0" => "PASS",
                "1" => "FAIL",
                "2" => "TIMEOUT",
                "3" => "NOTRUN",
                "4" => "PRECONDITION_FAILED",
                _ => "UNKNOWN",
            };
            result.tests.push((name.trim_end().to_owned(), status.to_owned(), message.to_owned()));
        } else if text == "WPT END" {
            ended = true;
        } else if !ended
            && (text.starts_with("uncaught") || text.contains("stopped") || text.starts_with("unhandled"))
            && result.tests.is_empty()
            && result.harness == "NO-REPORT"
        {
            result.detail = text.chars().take(160).collect();
        }
    }
    if !ended && result.harness != "NO-REPORT" {
        result.harness = format!("{}-CUT", result.harness);
    }
    if result.harness == "NO-REPORT" && result.detail.is_empty() {
        result.detail = outcome.report.message.chars().take(160).collect();
    }
    result
}
