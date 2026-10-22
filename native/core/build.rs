//! On Windows, `neoteletext.exe` is both the broker and the trusted core
//! (decision 56). Only the broker opens web pages, through the shell's
//! `ShellExecuteW`, and `shell32.dll` loads `user32.dll` and `gdi32.dll`,
//! whose start fails in a process denied Win32k, as the core is: it ended
//! at once with `STATUS_DLL_INIT_FAILED` (0xC0000142) when imported
//! directly. Delay-loaded, `shell32.dll` loads at the broker's first hand-off
//! and never in the core.

fn main() {
    let windows = std::env::var("CARGO_CFG_TARGET_OS").is_ok_and(|os| os == "windows");
    let microsoft_linker = std::env::var("CARGO_CFG_TARGET_ENV").is_ok_and(|environment| environment == "msvc");
    if windows && microsoft_linker {
        println!("cargo::rustc-link-arg-bin=neoteletext=/DELAYLOAD:shell32.dll");
        // The delay-load helper the linker's /DELAYLOAD calls on.
        println!("cargo::rustc-link-arg-bin=neoteletext=delayimp.lib");
    }
}
