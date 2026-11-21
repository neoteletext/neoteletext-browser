//! On Windows, `neoteletext.exe` is both the broker and the trusted core
//! (decision 56). Only the broker opens web pages, through the shell's
//! `ShellExecuteW`, and `shell32.dll` loads `user32.dll` and `gdi32.dll`,
//! whose start fails in a process denied Win32k, as the core is: it ended
//! at once with `STATUS_DLL_INIT_FAILED` (0xC0000142) when imported
//! directly. Delay-loaded, `shell32.dll` loads at the broker's first hand-off
//! and never in the core.
//!
//! The broker carries its version in its bytes, which the launcher reads
//! from the signed program (`installation-and-updates.md` section 5). A
//! release, or a test of an update, is built as the version
//! `NEOTELETEXT_RELEASE_VERSION` names. Every other compile is a
//! development build (REQ-7047): its version is the workspace's first two
//! numbers and, as the third, a count one higher than the last compile's,
//! so that after a rebuild and a restart the version shown says which
//! build is running. The count is kept in `native/build-number`, which
//! git ignores, so that a compile changes no tracked file; it is beside
//! the workspace and not in `target/`, so that `cargo clean`, another
//! profile or another target folder does not start it again.
//!
//! The program carries the browser's icon (decision 124): the art as a
//! resource file (`icon::windows`), which the MSVC linker takes as an
//! input, so Explorer, the Start menu and the taskbar show it.

use std::io::Read as _;
use std::path::{Path, PathBuf};

/// The development builds' count, beside the workspace (`native/`).
const COUNT_FILE: &str = "build-number";
/// The longest count file read: the five digits of 65,535, the largest
/// third number of a version (`update::version::Version`), and a line end.
const MAX_COUNT_FILE: usize = 6;
/// A file in this script's output folder that is never made: Cargo runs a
/// build script again whenever a file it names is missing, so naming this
/// one runs the script on every compile.
const NEVER_MADE: &str = "never-made";

/// The count file's place: the workspace's folder, this crate's parent.
fn count_file() -> PathBuf {
    let manifest = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").expect("Cargo sets CARGO_MANIFEST_DIR"));
    manifest.parent().expect("the crate is in the workspace's folder").join(COUNT_FILE)
}

/// The count this compile takes, one more than the file holds, written
/// back before it is used. After 65,535 it starts again at 1. The new
/// count replaces the old by a rename, so a compile interrupted, or two at
/// once, never leave a file that is not a count.
fn next_count(file: &Path) -> u16 {
    let last = match std::fs::File::open(file) {
        Ok(opened) => {
            // One byte more than a count has is read, and no more, to tell
            // a longer file from a count.
            let mut bytes = Vec::new();
            opened.take(MAX_COUNT_FILE as u64 + 1).read_to_end(&mut bytes).unwrap_or_else(|error| panic!("{}: {error}", file.display()));
            assert!(bytes.len() <= MAX_COUNT_FILE, "{} is not a build count; delete it to start again at 1", file.display());
            let text = std::str::from_utf8(&bytes).ok().map(str::trim_ascii_end);
            let count = text.filter(|text| text.bytes().all(|byte| byte.is_ascii_digit())).and_then(|text| text.parse::<u16>().ok());
            count.unwrap_or_else(|| panic!("{} is not a build count; delete it to start again at 1", file.display()))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => 0,
        Err(error) => panic!("{}: {error}", file.display()),
    };
    let next = last.checked_add(1).unwrap_or_else(|| {
        println!("cargo::warning=the build count passed {}, the largest a version holds, and starts again at 1", u16::MAX);
        1
    });
    let writing = file.with_extension(format!("{}.writing", std::process::id()));
    std::fs::write(&writing, format!("{next}\n")).unwrap_or_else(|error| panic!("{}: {error}", writing.display()));
    std::fs::rename(&writing, file).unwrap_or_else(|error| panic!("{}: {error}", file.display()));
    next
}

/// One of the workspace version's numbers, as Cargo gives it.
fn workspace_number(variable: &str) -> u16 {
    std::env::var(variable).ok().and_then(|number| number.parse().ok()).unwrap_or_else(|| panic!("{variable} is not a version's number"))
}

/// The version this compile is built as: a release's as named, or a
/// development build's with its count.
fn version() -> String {
    println!("cargo::rerun-if-env-changed=NEOTELETEXT_RELEASE_VERSION");
    if let Ok(release) = std::env::var("NEOTELETEXT_RELEASE_VERSION") {
        return release;
    }
    let out = PathBuf::from(std::env::var_os("OUT_DIR").expect("Cargo sets OUT_DIR"));
    println!("cargo::rerun-if-changed={}", out.join(NEVER_MADE).display());
    let (major, minor) = (workspace_number("CARGO_PKG_VERSION_MAJOR"), workspace_number("CARGO_PKG_VERSION_MINOR"));
    let version = format!("{major}.{minor}.{}", next_count(&count_file()));
    // The fetcher's user agent sends the workspace's first two numbers
    // (`CARGO_PKG_VERSION`, REQ-1015): the version shown starts with them.
    let workspace = std::env::var("CARGO_PKG_VERSION").unwrap_or_default();
    assert!(workspace.starts_with(&format!("{major}.{minor}.")), "the development version {version} is not of the workspace's {workspace}");
    version
}

fn main() {
    let version = version();
    assert!(!version.is_empty() && version.bytes().all(|byte| byte.is_ascii_digit() || byte == b'.'), "a release version is three numbers");
    println!("cargo::rustc-env=NEOTELETEXT_VERSION={version}");
    let windows = std::env::var("CARGO_CFG_TARGET_OS").is_ok_and(|os| os == "windows");
    let microsoft_linker = std::env::var("CARGO_CFG_TARGET_ENV").is_ok_and(|environment| environment == "msvc");
    if windows && microsoft_linker {
        println!("cargo::rustc-link-arg-bin=neoteletext=/DELAYLOAD:shell32.dll");
        // The delay-load helper the linker's /DELAYLOAD calls on.
        println!("cargo::rustc-link-arg-bin=neoteletext=delayimp.lib");
        // The program's icon (decision 124): the art as a resource file,
        // which the MSVC linker takes as an input.
        let out = std::env::var_os("OUT_DIR").expect("Cargo sets OUT_DIR");
        let resource = icon::windows::write_resource_file(Path::new(&out)).expect("the icon's resource file is written");
        println!("cargo::rustc-link-arg-bin=neoteletext={}", resource.display());
    }
}
