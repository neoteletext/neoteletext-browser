//! What a run reads from disk, gathered by the process that is not
//! confined and given to the worker as named files ([`crate::blobs`]):
//! the worker runs every script and reads every byte of them, and opens
//! no file.
//!
//! A channel's `channel.ttx` and its page files are read only by the
//! engine's own readers (section 16): the channel file by
//! `neoteletext_web::descriptor::Descriptor::parse_channel`, here, to
//! find the channel's first page and title; a page file by the decoder's
//! engine, in [`crate::screen`].
//!
//! # The scripts a channel names: the seam for work package C2
//!
//! Which script a page has is said by the channel file's `scripts`
//! member and by a page file's `JS` line or `SCRIPT_REF` record (section
//! 12), and the engine reads all three from its branch `reqresp` on
//! (work package C1). The browser's workspace is pinned to an engine
//! commit before that, and moving the pin is work package C2's; a second
//! commit of the engine in one workspace is refused by the commit gate
//! (requirement 2.5). So until the pin moves, this build's engine cannot
//! say which script a page has, and the runner is told on its command
//! line: `--script FILE.js` and `--channel-script FILE.js`. [`scripts`]
//! is the one function that changes when the pin moves.

use std::io::Read;
use std::path::{Path, PathBuf};

use neoteletext_web::descriptor::Descriptor;
use neoteletext_web::url::Url;
use protocol::script::MAX_SCRIPT_BYTES;

use crate::Failure;
use crate::args::{NetworkMode, Options};
use crate::blobs::Blob;
use crate::limits::{DEFAULT_PAGE, MAX_CHANNEL_FILE_BYTES, MAX_INPUT_BYTES, MAX_NETWORK_FILE_BYTES, MAX_PAGE_FILE_BYTES};

/// The name the command line is passed under, its arguments with a NUL
/// between.
pub const ARGUMENTS: &str = "arguments";
/// The channel file.
pub const CHANNEL_FILE: &str = "channel.ttx";
/// The page file, by its form.
pub const PAGE_TTI: &str = "page.tti";
/// See [`PAGE_TTI`].
pub const PAGE_T42: &str = "page.t42";
/// The page-level script.
pub const PAGE_SCRIPT: &str = "script.page";
/// The channel-level script.
pub const CHANNEL_SCRIPT: &str = "script.channel";
/// The file name of the page-level script, for its address.
pub const PAGE_SCRIPT_NAME: &str = "script.page.name";
/// The file name of the channel-level script, for its address.
pub const CHANNEL_SCRIPT_NAME: &str = "script.channel.name";
/// The input script.
pub const INPUT: &str = "input";
/// The network file.
pub const NETWORK: &str = "network";
/// Whether the target is a page file opened from disk (section 12.7).
pub const FROM_DISK: &str = "from-disk";

/// What the engine's reader says of a channel file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Channel {
    /// The channel's title.
    pub title: String,
    /// Its first page.
    pub initial_page: u16,
}

/// Reads a channel's `channel.ttx` with the engine's reader.
///
/// # Errors
/// Status 4: the address is no channel's, or the engine refuses the file.
pub fn channel(bytes: &[u8], channel_url: &str) -> Result<Channel, Failure> {
    let base = Url::parse(&format!("{channel_url}{CHANNEL_FILE}"))
        .map_err(|error| Failure::usage(format!("{channel_url}: not an address the engine takes: {error:?}")))?;
    let descriptor = Descriptor::parse_channel(bytes, &base).map_err(|error| {
        let scripts = core::str::from_utf8(bytes).is_ok_and(|text| text.contains("\"scripts\""));
        let hint = if scripts {
            " (this build's engine is pinned before the commit that reads the member `scripts`: name the scripts with --script and --channel-script, and run a copy of the channel file without the member, until the pin moves)"
        } else {
            ""
        };
        Failure::usage(format!("channel.ttx: the engine's reader refused it: {error:?}{hint}"))
    })?;
    Ok(Channel { title: descriptor.name.as_str().to_owned(), initial_page: descriptor.initial_page.mpp() })
}

/// The scripts a page has, as the paths to read: the page-level one and
/// the channel-level one. Today they are the command line's; when the
/// engine pin holds work package C1 they come from the channel file and
/// the page file by the engine's `neoteletext_web::scripts`.
fn scripts(options: &Options, target_script: Option<&Path>) -> (Option<PathBuf>, Option<PathBuf>) {
    (options.script.clone().or_else(|| target_script.map(Path::to_path_buf)), options.channel_script.clone())
}

/// Reads `path`, at most `most` bytes of it.
///
/// # Errors
/// Status 4: it cannot be read, or is longer.
pub fn read(path: &Path, most: usize) -> Result<Vec<u8>, Failure> {
    let fail = |why: String| Failure::usage(format!("{}: {why}", path.display()));
    let file = std::fs::File::open(path).map_err(|error| fail(error.to_string()))?;
    let mut bytes = Vec::new();
    file.take(u64::try_from(most).unwrap_or(u64::MAX).saturating_add(1)).read_to_end(&mut bytes).map_err(|error| fail(error.to_string()))?;
    if bytes.len() > most { Err(fail(format!("longer than {most} bytes"))) } else { Ok(bytes) }
}

fn has_extension(path: &Path, extension: &str) -> bool {
    path.extension().is_some_and(|found| found.eq_ignore_ascii_case(extension))
}

/// A script's file name, for its address: the file's own when it is one
/// an address can hold, else a plain one.
fn script_name(path: &Path, fallback: &str) -> Vec<u8> {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| name.bytes().all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_')));
    name.unwrap_or(fallback).as_bytes().to_vec()
}

/// A file read for a run, and when it was last changed as its reading
/// began: what `--watch` compares, so that a change made while the run
/// was going is seen.
pub type Watched = (PathBuf, Option<std::time::SystemTime>);

/// When `path` was last changed; nothing for a file that cannot be asked,
/// which differs from any time.
#[must_use]
pub fn changed(path: &Path) -> Option<std::time::SystemTime> {
    std::fs::metadata(path).and_then(|metadata| metadata.modified()).ok()
}

/// Everything a run reads, as named files, and the files read, for
/// `--watch`.
///
/// # Errors
/// Status 4: a file that cannot be read or is too long, or a target that
/// is none of a folder, a page file and a script.
pub fn gather(options: &Options, arguments: &[String]) -> Result<(Vec<Blob>, Vec<Watched>), Failure> {
    let mut blobs: Vec<Blob> = vec![(ARGUMENTS.to_owned(), arguments.join("\0").into_bytes())];
    let mut paths: Vec<Watched> = Vec::new();
    let mut take = |name: &str, path: &Path, most: usize| -> Result<(), Failure> {
        // The time first, then the bytes: a change after this is a change
        // since the run.
        paths.push((path.to_path_buf(), changed(path)));
        blobs.push((name.to_owned(), read(path, most)?));
        Ok(())
    };
    let target = options.target.as_path();
    let (mut target_script, mut from_disk) = (None, false);
    if target.is_dir() {
        let channel_file = target.join(CHANNEL_FILE);
        let mut first = DEFAULT_PAGE;
        if channel_file.is_file() {
            let bytes = read(&channel_file, MAX_CHANNEL_FILE_BYTES)?;
            first = channel(&bytes, &options.channel_url)?.initial_page;
            take(CHANNEL_FILE, &channel_file, MAX_CHANNEL_FILE_BYTES)?;
        }
        let page = options.page.map_or(first, |(page, _)| page);
        // A channel's page file is NNN.tti or NNN.t42, its number in
        // hexadecimal capitals (the engine's REQ-508); a page with neither
        // is a JavaScript page (section 12.3).
        for (extension, name) in [("tti", PAGE_TTI), ("t42", PAGE_T42)] {
            let file = target.join(format!("{page:03X}.{extension}"));
            if file.is_file() {
                take(name, &file, MAX_PAGE_FILE_BYTES)?;
                break;
            }
        }
    } else if has_extension(target, "js") {
        target_script = Some(target);
    } else if has_extension(target, "tti") {
        take(PAGE_TTI, target, MAX_PAGE_FILE_BYTES)?;
        from_disk = true;
    } else if has_extension(target, "t42") {
        take(PAGE_T42, target, MAX_PAGE_FILE_BYTES)?;
        from_disk = true;
    } else {
        return Err(Failure::usage(format!("{}: not a channel's folder, a page file (.tti, .t42) or a script (.js)", target.display())));
    }
    let (page_script, channel_script) = scripts(options, target_script);
    if let Some(path) = &page_script {
        take(PAGE_SCRIPT, path, MAX_SCRIPT_BYTES)?;
    }
    if let Some(path) = &channel_script {
        take(CHANNEL_SCRIPT, path, MAX_SCRIPT_BYTES)?;
    }
    if let Some(path) = &options.input {
        take(INPUT, path, MAX_INPUT_BYTES)?;
    }
    if let NetworkMode::Mock(path) | NetworkMode::Replay(path) = &options.network {
        take(NETWORK, path, MAX_NETWORK_FILE_BYTES)?;
    }
    if let Some(path) = &page_script {
        blobs.push((PAGE_SCRIPT_NAME.to_owned(), script_name(path, "page.js")));
    }
    if let Some(path) = &channel_script {
        blobs.push((CHANNEL_SCRIPT_NAME.to_owned(), script_name(path, "channel.js")));
    }
    if from_disk {
        blobs.push((FROM_DISK.to_owned(), Vec::new()));
    }
    if page_script.is_none() && channel_script.is_none() {
        return Err(Failure::usage("nothing to run: no script. Give a script (.js), or name the page's with --script and the channel's with --channel-script"));
    }
    Ok((blobs, paths))
}
