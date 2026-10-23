//! Hands a web address to the system's web browser (`hyperlinks.md`
//! section 12). The trusted core calls it only after the user confirmed
//! the web-browser prompt in the trusted strip, with an address it parsed
//! and rebuilt itself; this crate checks the scheme again and passes the
//! address as one argument, never through a shell:
//!
//! - macOS: `/usr/bin/open -u`, which opens it as an address even if it
//!   names a file;
//! - Linux: `xdg-open`, until a D-Bus crate is approved for the XDG
//!   Desktop Portal (decision 41, the hyperlink decision 10);
//! - Windows: the shell's `ShellExecuteW`, which opens an `https` or
//!   `http` address in the default browser. That call is `unsafe`, so this
//!   is the second crate of the browser allowed it, on Windows only
//!   (decision 41, the hyperlink decision 7).

// No panicking constructs: the address is untrusted page data, however
// well checked.
#![deny(
    clippy::indexing_slicing,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::arithmetic_side_effects,
    clippy::unreachable,
    clippy::todo,
    clippy::unimplemented
)]

#[cfg(windows)]
mod windows;

/// The only schemes handed over.
const SCHEMES: [&str; 2] = ["https://", "http://"];

/// Why the address was not handed over.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OpenError {
    /// Not an `https` or `http` address, or holding a character no
    /// address holds.
    Refused,
    /// The system's opener could not be started, or refused it.
    Failed(String),
}

impl std::fmt::Display for OpenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Refused => write!(f, "not an https or http address"),
            Self::Failed(reason) => write!(f, "the web browser could not be opened: {reason}"),
        }
    }
}

impl std::error::Error for OpenError {}

/// Whether `address` may be handed over: `https` or `http`, printable
/// ASCII with no spaces, quotes or backslashes. The trusted core has
/// already rebuilt it from its parsed parts; this is a second wall.
#[must_use]
pub fn acceptable(address: &str) -> bool {
    SCHEMES.iter().any(|scheme| address.starts_with(scheme))
        && address.bytes().all(|byte| byte.is_ascii_graphic() && !matches!(byte, b'"' | b'\\' | b'<' | b'>' | b'`' | b'{' | b'}' | b'|' | b'^'))
}

/// Opens `address` in the system's web browser. Returns once the opener is
/// started; it is not waited for, so a broken opener cannot hang the
/// trusted core (`hyperlinks-security.md` section 12).
///
/// # Errors
/// When the address is refused, or the opener cannot be started.
pub fn open(address: &str) -> Result<(), OpenError> {
    if !acceptable(address) {
        return Err(OpenError::Refused);
    }
    platform(address)
}

#[cfg(windows)]
fn platform(address: &str) -> Result<(), OpenError> {
    windows::open(address)
}

#[cfg(not(windows))]
fn platform(address: &str) -> Result<(), OpenError> {
    use std::process::Command;
    let mut command = if cfg!(target_os = "macos") {
        let mut command = Command::new("/usr/bin/open");
        command.arg("-u");
        command
    } else {
        Command::new("/usr/bin/xdg-open")
    };
    let mut child = detached(command.arg(address)).spawn().map_err(|error| OpenError::Failed(error.to_string()))?;
    // Reaped in the background, so it leaves no zombie and is never waited on.
    std::thread::spawn(move || child.wait());
    Ok(())
}

/// The opener's command, with its standard streams closed and in a
/// process group of its own (group 0: the child's own pid, POSIX
/// `setpgid`). The web browser the opener may start then stays out of the
/// broker's group, which the launcher kills whole after a broker crash
/// (finding F-34); a browser the user opened outlives the crash.
#[cfg(not(windows))]
fn detached(command: &mut std::process::Command) -> &mut std::process::Command {
    use std::os::unix::process::CommandExt;
    use std::process::Stdio;
    command.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).process_group(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_plain_https_and_http_addresses_are_handed_over() {
        assert!(acceptable("https://www.example.invalid/story?x=1#top"));
        assert!(acceptable("http://example.invalid/"));
        for address in [
            "",
            "file:///etc/passwd",
            "javascript:alert(1)",
            "ms-settings:display",
            "-https://example.invalid/",
            "HTTPS://example.invalid/",
            "https://example.invalid/ x",
            "https://example.invalid/\"x",
            "https://example.invalid/\\x",
            "https://example.invalid/é",
            "https://example.invalid/\n",
            "\\\\server\\share",
            "/usr/bin/true",
        ] {
            assert!(!acceptable(address), "{address:?} must be refused");
            assert_eq!(open(address), Err(OpenError::Refused), "{address:?} never reaches an opener");
        }
    }

    /// The process group of `pid`, as `ps` reports it.
    #[cfg(unix)]
    fn process_group(pid: u32) -> String {
        let output = std::process::Command::new("/bin/ps").args(["-o", "pgid=", "-p", &pid.to_string()]).output();
        String::from_utf8_lossy(&output.map(|output| output.stdout).unwrap_or_default()).trim().to_owned()
    }

    /// The opener, and so the web browser it may start, is in a process
    /// group of its own, so the launcher's kill of the broker's group after
    /// a crash (finding F-34) never reaches a browser the user opened.
    #[cfg(unix)]
    #[test]
    fn the_opener_is_in_a_process_group_of_its_own() {
        let mut child = detached(std::process::Command::new("/bin/sleep").arg("30")).spawn().map_err(|error| error.to_string());
        let child = child.as_mut().map(|child| (child.id(), child));
        let Ok((pid, child)) = child else { panic!("sleep did not start: {child:?}") };
        let (own, its) = (process_group(std::process::id()), process_group(pid));
        let _ = child.kill();
        let _ = child.wait();
        assert!(!own.is_empty() && !its.is_empty(), "ps reported {own:?} and {its:?}");
        assert_ne!(own, its, "the opener shares this process's group");
        assert_eq!(its, pid.to_string(), "it leads its own group");
    }

    /// Opens a page in the system's default web browser: run by hand on
    /// each platform (`cargo test -p web-opener -- --ignored`).
    #[test]
    #[ignore = "opens the system's web browser"]
    fn an_address_opens_in_the_default_browser() {
        assert_eq!(open("https://www.example.com/"), Ok(()));
    }
}
