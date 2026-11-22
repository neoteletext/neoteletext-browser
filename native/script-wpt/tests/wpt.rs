//! The web-platform-tests, run in the standalone runner (work package
//! W3; script-api.md section 9.4): each suite's worker-scope test files,
//! every case compared with what `expectations/` keeps for it.
//!
//! The tests need the suite, fetched by `fetch.py` to a folder the
//! environment variable `NEOTELETEXT_WPT` names. Without it they cannot
//! run, so each is ignored with that reason, and asked for without it
//! each fails: `cargo test -p neoteletext-script-wpt -- --ignored`.
//! With `NEOTELETEXT_WPT_BLESS=1` a suite's expectations are written
//! from what ran; a failing case no rule of `expectations/reasons.txt`
//! explains is written as `UNLISTED`, and fails the comparison until it
//! has a departure or a to-do.

mod wpt {
    pub mod expect;
    pub mod page;
    pub mod server;
    pub mod tree;
}

use std::path::{Path, PathBuf};

use script_wpt::{BLESS_VARIABLE, ROOT_VARIABLE};
use wpt::expect::{Kept, Reasons};
use wpt::page::{FileResult, run_case};
use wpt::tree::{Case, Scope, cases};

/// Why a suite's test is ignored: what it needs and how to get it.
macro_rules! needs_the_suite {
    () => {
        "needs the web-platform-tests at the folder $NEOTELETEXT_WPT names (native/script-wpt/fetch.py fetches them at the pinned commit)"
    };
}

/// The folder of the fetched suite. A test asked for without it fails.
fn root() -> PathBuf {
    let Some(root) = std::env::var_os(ROOT_VARIABLE).map(PathBuf::from) else {
        panic!("{ROOT_VARIABLE} is not set: {}", needs_the_suite!());
    };
    assert!(root.join("resources/testharness.js").is_file(), "{} holds no resources/testharness.js: {}", root.display(), needs_the_suite!());
    root
}

/// Runs every case of `folders` and compares, or with the bless variable
/// writes, the expectations kept as `expectations/NAME.txt`. With
/// `waits`, no file is run: each is kept as not run, for that reason.
fn suite_of(name: &str, folders: &[&str], waits: Option<&'static str>) {
    let root = root();
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let reasons = Reasons::read(&manifest.join("expectations/reasons.txt"));
    let mut kept = Kept::default();
    for folder in folders {
        for case in cases(&root, folder) {
            match (&case.scope, waits) {
                (Scope::Worker, None) => {
                    let result: FileResult = run_case(&root, &case);
                    kept.ran(&case, &result, &reasons);
                }
                (Scope::Worker, Some(why)) => kept.skipped(&case, why, &reasons),
                (Scope::Skipped(why), _) => kept.skipped(&case, why, &reasons),
            }
        }
    }
    println!("{name}: {}", kept.counts());
    let path = manifest.join(format!("expectations/{name}.txt"));
    let text = kept.text(name);
    if std::env::var_os(BLESS_VARIABLE).is_some_and(|value| value == "1") {
        std::fs::write(&path, text).unwrap();
        std::fs::write(Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("wpt-{name}.said.txt")), kept.said()).unwrap();
        println!("{name}: wrote {}", path.display());
        return;
    }
    let before = std::fs::read_to_string(&path).unwrap_or_default();
    let (old, new): (Vec<&str>, Vec<&str>) = (before.lines().collect(), text.lines().collect());
    let gone: Vec<&&str> = old.iter().filter(|line| !new.contains(line)).take(40).collect();
    let come: Vec<&&str> = new.iter().filter(|line| !old.contains(line)).take(40).collect();
    assert!(gone.is_empty() && come.is_empty(), "{name}: the results are not the kept ones.\nno longer so: {gone:#?}\nnew: {come:#?}");
    let unlisted = kept.unlisted();
    assert!(unlisted.is_empty(), "{name}: cases fail with no departure and no to-do: {unlisted:#?}");
}

fn suite(name: &str, folders: &[&str]) {
    suite_of(name, folders, None);
}

/// URL standard: `URL`, `URLSearchParams`.
#[test]
#[ignore = needs_the_suite!()]
fn url() {
    suite("url", &["url"]);
}

/// Encoding standard: `TextEncoder`, `TextDecoder` and their streams.
#[test]
#[ignore = needs_the_suite!()]
fn encoding() {
    suite("encoding", &["encoding"]);
}

/// Streams standard, by the bundled library.
#[test]
#[ignore = needs_the_suite!()]
fn streams() {
    suite("streams", &["streams"]);
}

/// Fetch standard: the API-level folders, which the mock network can
/// answer (`Headers`, `Request`, `Response`, bodies, abort, the basics).
#[test]
#[ignore = needs_the_suite!()]
fn fetch_api() {
    suite("fetch-api", &["fetch/api/headers", "fetch/api/request", "fetch/api/response", "fetch/api/body", "fetch/api/abort", "fetch/api/basic"]);
}

/// Fetch standard: the folders that test the network's rules (CORS,
/// redirects, credentials, policies), which only the real web client
/// against the suite's server can answer.
#[test]
#[ignore = needs_the_suite!()]
fn fetch_network() {
    // Not run against the mock network, which applies no rule of the
    // network: what these would show there is the mock, not the browser.
    suite_of("fetch-network", &["fetch/api/cors", "fetch/api/redirect", "fetch/api/credentials", "fetch/api/policies"], Some("LIVE-NETWORK"));
}

/// Cookies (RFC 6265bis): the suite's tests are pages.
#[test]
#[ignore = needs_the_suite!()]
fn cookies() {
    suite("cookies", &["cookies"]);
}

/// The WebSocket standard: `WebSocket` and `CloseEvent`.
#[test]
#[ignore = needs_the_suite!()]
fn websockets() {
    suite("websockets", &["websockets"]);
}

/// HTML's server-sent events.
#[test]
#[ignore = needs_the_suite!()]
fn eventsource() {
    suite("eventsource", &["eventsource"]);
}

/// File API (`Blob`, `File`) and `FormData`.
#[test]
#[ignore = needs_the_suite!()]
fn file_api_and_form_data() {
    suite("fileapi-formdata", &["FileAPI", "xhr/formdata"]);
}

/// The suite is absent in an ordinary run: the tests above are ignored,
/// and the pinned commit is the one `fetch.py` fetches.
#[test]
fn the_fetch_script_fetches_the_commit_the_expectations_name() {
    let script = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("fetch.py")).unwrap();
    assert!(script.contains(&format!("COMMIT = \"{}\"", script_wpt::COMMIT)));
    let _ = Case::default();
}
