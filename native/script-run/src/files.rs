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
//! # The scripts a channel names
//!
//! Which script a page has is said by the channel file's `scripts`
//! member and by a page file's `JS` line or `SCRIPT_REF` record (section
//! 12), and only the engine's readers say it: [`channel`] reads the
//! member with `neoteletext_web::descriptor::page_scripts`, and the
//! decoder's engine reads a page file's own reference as it reads the
//! page ([`crate::screen::Screen::script_names`]). A script is a file of
//! its channel's folder and of no other place (section 12.2), so this
//! process passes the worker every file of the folder whose name is a
//! script name (`ScriptName::parse`), unread, and the worker takes the
//! ones the readers name.

use std::io::Read;
use std::path::{Path, PathBuf};

use neoteletext_core::types::ScriptName;
use neoteletext_web::descriptor::{Descriptor, page_scripts};
use neoteletext_web::url::Url;
use protocol::script::MAX_SCRIPT_BYTES;

use crate::Failure;
use crate::args::{NetworkMode, Options};
use crate::blobs::Blob;
use crate::limits::{DEFAULT_PAGE, MAX_CHANNEL_FILE_BYTES, MAX_FOLDER_SCRIPTS, MAX_INPUT_BYTES, MAX_NETWORK_FILE_BYTES, MAX_PAGE_FILE_BYTES};

/// The name the command line is passed under, its arguments with a NUL
/// between.
pub const ARGUMENTS: &str = "arguments";
/// The channel file.
pub const CHANNEL_FILE: &str = "channel.ttx";
/// The page file, by its form.
pub const PAGE_TTI: &str = "page.tti";
/// See [`PAGE_TTI`].
pub const PAGE_T42: &str = "page.t42";
/// The page-level script, when the run's target is one script; and the
/// channel-level script beside it, which only a test gives so.
pub const PAGE_SCRIPT: &str = "script.page";
/// See [`PAGE_SCRIPT`].
pub const CHANNEL_SCRIPT: &str = "script.channel";
/// What a script file of the channel's folder is passed under, before
/// its file name.
pub const FOLDER_SCRIPT: &str = "js.";
/// The file name of the page-level script, for its address.
pub const PAGE_SCRIPT_NAME: &str = "script.page.name";
/// The file name of the channel-level script, for its address.
pub const CHANNEL_SCRIPT_NAME: &str = "script.channel.name";
/// The input script.
pub const INPUT: &str = "input";
/// The network file.
pub const NETWORK: &str = "network";
/// The name, before its extension, of a page file opened from disk: a
/// channel's page file is named by its page number (the engine's
/// REQ-508), which is then the page the run opens.
pub const PAGE_FILE_STEM: &str = "page.stem";
/// Whether the target is a page file opened from disk (section 12.7).
pub const FROM_DISK: &str = "from-disk";

/// What the engine's readers say of a channel file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Channel {
    /// The channel's title.
    pub title: String,
    /// Its first page.
    pub initial_page: u16,
    /// The channel-level script's name.
    pub channel_script: Option<String>,
    /// Each entry of `scripts.pages`: the page, its sub-code when the key
    /// names a sub-page, and the script's name.
    pub page_scripts: Vec<(u16, Option<u16>, String)>,
    /// The channel is live (its file has the member `push`), so it runs
    /// no script and its `scripts` member is ignored (section 12.1).
    pub scripts_ignored: bool,
}

impl Channel {
    /// The script the channel file names for exactly this page and
    /// sub-code (section 12.1: keys are exact, nothing is inherited). A
    /// page with no sub-pages, sub-code 0, is the page written with none.
    #[must_use]
    pub fn page_script(&self, page: u16, subcode: u16) -> Option<&str> {
        let key = (subcode != 0).then_some(subcode);
        self.page_scripts.iter().find(|(held, sub, _)| *held == page && *sub == key).map(|(_, _, name)| name.as_str())
    }
}

/// Reads a channel's `channel.ttx` with the engine's readers.
///
/// # Errors
/// Status 4: the address is no channel's, or the engine refuses the file.
pub fn channel(bytes: &[u8], channel_url: &str) -> Result<Channel, Failure> {
    let base = Url::parse(&format!("{channel_url}{CHANNEL_FILE}"))
        .map_err(|error| Failure::usage(format!("{channel_url}: not an address the engine takes: {error:?}")))?;
    let refused = |error| Failure::usage(format!("channel.ttx: the engine's reader refused it: {error:?}"));
    let descriptor = Descriptor::parse_channel(bytes, &base).map_err(refused)?;
    let mut entries = Vec::new();
    let scripts = page_scripts(bytes, &mut |page, subcode, name| {
        entries.push((page.mpp(), subcode.map(neoteletext_core::types::SubCode::value), name.as_str().to_owned()));
    })
    .map_err(refused)?;
    let ignored = descriptor.scripts_ignored();
    Ok(Channel {
        title: descriptor.name.as_str().to_owned(),
        initial_page: descriptor.initial_page.mpp(),
        channel_script: scripts.channel.filter(|_| !ignored).map(|name| name.as_str().to_owned()),
        page_scripts: if ignored { Vec::new() } else { entries },
        scripts_ignored: ignored,
    })
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

/// The script files of `folder`: every file whose name is a script name
/// (section 12.2), by name, at most [`MAX_FOLDER_SCRIPTS`].
fn folder_scripts(folder: &Path) -> Result<Vec<(String, PathBuf)>, Failure> {
    let fail = |error: std::io::Error| Failure::usage(format!("{}: {error}", folder.display()));
    let mut found = Vec::new();
    for entry in std::fs::read_dir(folder).map_err(fail)? {
        let path = entry.map_err(fail)?.path();
        let Some(name) = path.file_name().and_then(|name| name.to_str()).filter(|name| ScriptName::parse(name.as_bytes()).is_some()) else { continue };
        if path.is_file() {
            found.push((name.to_owned(), path.clone()));
        }
    }
    found.sort();
    if found.len() > MAX_FOLDER_SCRIPTS {
        return Err(Failure::usage(format!("MAX_FOLDER_SCRIPTS: {} holds more than {MAX_FOLDER_SCRIPTS} script files", folder.display())));
    }
    Ok(found)
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
    let (mut from_disk, mut folder) = (false, None);
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
        folder = Some(target.to_path_buf());
    } else if has_extension(target, "js") {
        take(PAGE_SCRIPT, target, MAX_SCRIPT_BYTES)?;
    } else if has_extension(target, "tti") || has_extension(target, "t42") {
        take(if has_extension(target, "tti") { PAGE_TTI } else { PAGE_T42 }, target, MAX_PAGE_FILE_BYTES)?;
        // The script a file opened from disk names is read only from the
        // file's own folder (section 12.7 rule 1).
        from_disk = true;
        folder = Some(target.parent().filter(|parent| !parent.as_os_str().is_empty()).map_or_else(|| PathBuf::from("."), Path::to_path_buf));
    } else {
        return Err(Failure::usage(format!("{}: not a channel's folder, a page file (.tti, .t42) or a script (.js)", target.display())));
    }
    if let Some(folder) = &folder {
        for (name, path) in folder_scripts(folder)? {
            take(&format!("{FOLDER_SCRIPT}{name}"), &path, MAX_SCRIPT_BYTES)?;
        }
    }
    if let Some(path) = &options.input {
        take(INPUT, path, MAX_INPUT_BYTES)?;
    }
    if let NetworkMode::Mock(path) | NetworkMode::Replay(path) = &options.network {
        take(NETWORK, path, MAX_NETWORK_FILE_BYTES)?;
    }
    if has_extension(target, "js")
        && let Some(name) = target.file_name().and_then(|name| name.to_str()).filter(|name| ScriptName::parse(name.as_bytes()).is_some())
    {
        blobs.push((PAGE_SCRIPT_NAME.to_owned(), name.as_bytes().to_vec()));
    }
    if from_disk {
        blobs.push((FROM_DISK.to_owned(), Vec::new()));
        let stem = target.file_stem().and_then(|stem| stem.to_str()).filter(|stem| stem.len() == 3 && stem.bytes().all(|byte| byte.is_ascii_hexdigit()));
        if let Some(stem) = stem {
            blobs.push((PAGE_FILE_STEM.to_owned(), stem.as_bytes().to_vec()));
        }
    }
    Ok((blobs, paths))
}
