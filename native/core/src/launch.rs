//! How the browser was launched (REQ-1673). The operating system substitutes
//! a link or a file's name into a registered command line, and on Windows
//! quotes and backslashes in a crafted one can add parameters. So a launch
//! carrying a marker is exactly the marker and one argument: anything else is
//! refused whole, and no switch before or after it takes effect. On Linux
//! the desktop entry's `%u` is one bare argument, and the same rule holds for
//! a link: alone, or refused.

use std::collections::VecDeque;
use std::ffi::OsString;
use std::time::{Duration, Instant};

use crate::address::{is_outside_link, outside_link};

/// The marker of a registered link.
pub const OPEN_LINK: &str = "--open-link";
/// The marker of a registered file.
pub const OPEN_FILE: &str = "--open-file";

/// What a launch is.
#[derive(Debug, PartialEq, Eq)]
pub enum Launch {
    /// An ordinary command line, with no link on it.
    Plain,
    /// One link, checked by the address rules.
    Link(String),
    /// One file, as the system named it. Reading it is decided later.
    File(String),
}

/// Why a launch was refused, in words that repeat none of it (REQ-1672
/// item 12).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refused {
    /// Something other than one argument came with the marker.
    NotOneArgument,
    /// The link breaks the address rules.
    BadLink,
    /// The file's name is not a path, or looks like a switch.
    BadFile,
}

impl std::fmt::Display for Refused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::NotOneArgument => "a registered link or file must be the one argument",
            Self::BadLink => "the link is not one this browser opens",
            Self::BadFile => "the file's name is not a path",
        })
    }
}

fn is_marker(argument: &OsString) -> bool {
    argument == OPEN_LINK || argument == OPEN_FILE
}

fn is_link(argument: &OsString) -> bool {
    argument.to_str().is_some_and(is_outside_link)
}

/// Reads a command line's arguments, before anything else looks at them.
///
/// # Errors
/// When a marker or a link came with anything else, or breaks its own rules.
pub fn read(arguments: &[OsString]) -> Result<Launch, Refused> {
    if !arguments.iter().any(|argument| is_marker(argument) || is_link(argument)) {
        return Ok(Launch::Plain);
    }
    match arguments {
        [marker, argument] if marker == OPEN_LINK => link(argument),
        [marker, argument] if marker == OPEN_FILE => file(argument),
        [argument] if is_link(argument) => link(argument),
        _ => Err(Refused::NotOneArgument),
    }
}

fn link(argument: &OsString) -> Result<Launch, Refused> {
    let text = argument.to_str().ok_or(Refused::BadLink)?;
    outside_link(text).map_err(|_| Refused::BadLink)?;
    Ok(Launch::Link(text.to_owned()))
}

fn file(argument: &OsString) -> Result<Launch, Refused> {
    let text = argument.to_str().ok_or(Refused::BadFile)?;
    let path = crate::address::parse(text).map_err(|_| Refused::BadFile)?;
    if path.file.is_none() {
        return Err(Refused::BadFile);
    }
    Ok(Launch::File(text.to_owned()))
}

/// The most links from outside opened in [`OUTSIDE_LINK_INTERVAL`]: more than
/// a viewer clicks by hand, few enough that no program can fill a window with
/// tabs (REQ-1672 item 11). Chrome limits only on the sending side.
pub const MAX_OUTSIDE_LINKS: usize = 5;
/// The time [`MAX_OUTSIDE_LINKS`] is counted over.
pub const OUTSIDE_LINK_INTERVAL: Duration = Duration::from_secs(10);

/// The limit on links from outside: those beyond it are dropped, not queued,
/// so that nothing is left to go off later.
#[derive(Debug, Default)]
pub struct OutsideLinks {
    opened: VecDeque<Instant>,
}

impl OutsideLinks {
    /// Whether a link may open at `now`, counting it if so.
    pub fn admit(&mut self, now: Instant) -> bool {
        while self.opened.front().is_some_and(|at| now.saturating_duration_since(*at) >= OUTSIDE_LINK_INTERVAL) {
            self.opened.pop_front();
        }
        let room = self.opened.len() < MAX_OUTSIDE_LINKS;
        if room {
            self.opened.push_back(now);
        }
        room
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, reason = "a test")]
mod tests {
    use super::*;

    /// REQ-1672 item 11: a burst opens only what the limit allows, and
    /// nothing later; a viewer's handful goes through.
    #[test]
    fn a_burst_of_links_is_cut_off_and_nothing_comes_later() {
        let start = Instant::now();
        let mut links = OutsideLinks::default();
        let opened = (0..100).filter(|_| links.admit(start)).count();
        assert_eq!(opened, MAX_OUTSIDE_LINKS);
        assert!(!links.admit(start + OUTSIDE_LINK_INTERVAL.saturating_sub(Duration::from_millis(1))));
        assert!(links.admit(start + OUTSIDE_LINK_INTERVAL), "and what was dropped does not count against later");
    }

    fn arguments(list: &[&str]) -> Vec<OsString> {
        list.iter().map(OsString::from).collect()
    }

    const LINK: &str = "neoteletext://https://news.example/302";

    #[test]
    fn a_marker_and_one_good_argument_is_a_launch() {
        assert_eq!(read(&arguments(&[OPEN_LINK, LINK])), Ok(Launch::Link(LINK.into())));
        assert_eq!(read(&arguments(&[LINK])), Ok(Launch::Link(LINK.into())), "Linux's %u");
        let file_name = if cfg!(windows) { r"C:\pages\100.tti" } else { "/pages/100.tti" };
        assert_eq!(read(&arguments(&[OPEN_FILE, file_name])), Ok(Launch::File(file_name.into())));
        assert_eq!(read(&arguments(&["--allow-local", "127.0.0.1", "/a.t42"])), Ok(Launch::Plain), "no marker: as ever");
    }

    /// What a crafted `%1` becomes on the command line: Rust's reading of a
    /// quote gives more than one argument, and each is refused whole.
    #[test]
    fn anything_beside_the_one_argument_refuses_the_launch() {
        for list in [
            &[OPEN_LINK][..],
            &[OPEN_LINK, LINK, "--uninstall"],
            &[OPEN_LINK, "neoteletext://https://a.example/", "--allow-local", "127.0.0.1"],
            &["--no-keychain", OPEN_LINK, LINK],
            &[OPEN_LINK, LINK, LINK],
            &[OPEN_FILE, "/a.tti", "/b.tti"],
            &["--keychain", OPEN_FILE, "/a.tti"],
            &[OPEN_LINK, OPEN_FILE, "/a.tti"],
            &[LINK, "--input-trace"],
            &["--input-trace", LINK],
        ] {
            assert_eq!(read(&arguments(list)), Err(Refused::NotOneArgument), "{list:?}");
        }
    }

    #[test]
    fn the_one_argument_must_pass_its_own_rules() {
        for bad in ["--uninstall", "https://a.example/", "neoteletext://home", "neoteletext://file:///etc/passwd", "neoteletext://javascript://x"] {
            assert_eq!(read(&arguments(&[OPEN_LINK, bad])), Err(Refused::BadLink), "{bad}");
        }
        assert_eq!(read(&arguments(&[OPEN_FILE, "--uninstall"])), Err(Refused::BadFile));
        assert_eq!(read(&arguments(&[OPEN_FILE, "https://a.example/"])), Err(Refused::BadFile));
    }
}
