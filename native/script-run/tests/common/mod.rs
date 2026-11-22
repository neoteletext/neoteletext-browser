//! What the runner's tests share: a run made in this process from
//! scripts and files given as text, through the same function the worker
//! calls.
#![allow(dead_code, reason = "each test file uses its own part of this")]

use script_run::Status;
use script_run::blobs::{Blob, find};
use script_run::files;
use script_run::run::{MESSAGE, STATUS};
use script_run::worker::run_given;

/// A manifest that claims everything the tests give: the keys named,
/// text, pointer moves, the wheel and every tick.
pub const CLAIMS: &str = r#"/*! nt {"api": 1, "renderer": "teletext", "keys": ["Enter", "a"], "text": true, "pointer": "move", "wheel": true, "tick": 1} */"#;

/// What one run of the tests is given.
#[derive(Default)]
pub struct Given<'a> {
    /// The arguments after `run TARGET`.
    pub arguments: &'a [&'a str],
    /// The page-level script.
    pub page: Option<&'a str>,
    /// The channel-level script.
    pub channel: Option<&'a str>,
    /// The input script.
    pub input: Option<&'a str>,
    /// The network file.
    pub network: Option<&'a str>,
    /// A page file, as TTI.
    pub page_file: Option<&'a str>,
}

/// The files of a run of `given`, as the unconfined process would gather
/// them, run as the worker runs them.
pub fn run(given: &Given<'_>) -> Vec<Blob> {
    let mut arguments = vec!["run".to_owned(), "page.js".to_owned()];
    arguments.extend(given.arguments.iter().map(|argument| (*argument).to_owned()));
    let mut blobs: Vec<Blob> = vec![(files::ARGUMENTS.to_owned(), arguments.join("\0").into_bytes())];
    let texts = [
        (files::PAGE_SCRIPT, given.page),
        (files::CHANNEL_SCRIPT, given.channel),
        (files::INPUT, given.input),
        (files::NETWORK, given.network),
        (files::PAGE_TTI, given.page_file),
    ];
    for (name, text) in texts {
        if let Some(text) = text {
            blobs.push((name.to_owned(), text.as_bytes().to_vec()));
        }
    }
    run_given(&blobs)
}

/// A run's status.
pub fn status(made: &[Blob]) -> Status {
    Status::of(find(made, STATUS).unwrap()[0] - b'0').unwrap()
}

/// A run's message, or nothing.
pub fn message(made: &[Blob]) -> String {
    find(made, MESSAGE).map(|bytes| String::from_utf8_lossy(bytes).into_owned()).unwrap_or_default()
}

/// A run's file as text.
pub fn text(made: &[Blob], name: &str) -> String {
    String::from_utf8(find(made, name).unwrap_or_else(|| panic!("the run made no {name}: {}", message(made))).to_vec()).unwrap()
}

/// The log's lines.
pub fn log(made: &[Blob]) -> Vec<String> {
    text(made, script_run::run::LOG_FILE).lines().map(str::to_owned).collect()
}

/// A PNG's width and height, from its header chunk (the PNG
/// specification, section 11.2.2: the first chunk, at byte 16).
pub fn png_size(png: &[u8]) -> (u32, u32) {
    assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
    (u32::from_be_bytes(png[16..20].try_into().unwrap()), u32::from_be_bytes(png[20..24].try_into().unwrap()))
}
