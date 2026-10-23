//! On Windows, `neoteletext.exe` is both the broker and the trusted core
//! (decision 56). Only the broker opens web pages, through the shell's
//! `ShellExecuteW`, and `shell32.dll` loads `user32.dll` and `gdi32.dll`,
//! whose start fails in a process denied Win32k, as the core is: it ended
//! at once with `STATUS_DLL_INIT_FAILED` (0xC0000142) when imported
//! directly. Delay-loaded, `shell32.dll` loads at the broker's first hand-off
//! and never in the core.
//!
//! The broker carries the release's version in its bytes, which the
//! launcher reads from the signed program (`installation-and-updates.md`
//! section 5): the workspace's version, or for a test of an update the one
//! `NEOTELETEXT_RELEASE_VERSION` names at build time.

fn main() {
    println!("cargo::rerun-if-env-changed=NEOTELETEXT_RELEASE_VERSION");
    let version = std::env::var("NEOTELETEXT_RELEASE_VERSION").unwrap_or_else(|_| std::env::var("CARGO_PKG_VERSION").unwrap_or_default());
    assert!(!version.is_empty() && version.bytes().all(|byte| byte.is_ascii_digit() || byte == b'.'), "a release version is three numbers");
    println!("cargo::rustc-env=NEOTELETEXT_VERSION={version}");
    let windows = std::env::var("CARGO_CFG_TARGET_OS").is_ok_and(|os| os == "windows");
    let microsoft_linker = std::env::var("CARGO_CFG_TARGET_ENV").is_ok_and(|environment| environment == "msvc");
    if windows && microsoft_linker {
        println!("cargo::rustc-link-arg-bin=neoteletext=/DELAYLOAD:shell32.dll");
        // The delay-load helper the linker's /DELAYLOAD calls on.
        println!("cargo::rustc-link-arg-bin=neoteletext=delayimp.lib");
    }
}
