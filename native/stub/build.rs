//! Windows: the program's imported libraries are loaded from System32
//! alone, never from the folder it was started from, where a planted
//! `wintrust.dll` beside a download would otherwise be loaded first
//! (finding F-25, CWE-427). `/DEPENDENTLOADFLAG:0x800` is the MSVC
//! linker's flag (Microsoft's linker documentation, `/DEPENDENTLOADFLAG`),
//! and 0x800 is `LOAD_LIBRARY_SEARCH_SYSTEM32` (`LoadLibraryExW`'s flags,
//! `libloaderapi.h`); a build script has no Windows bindings to name it by.

/// `LOAD_LIBRARY_SEARCH_SYSTEM32`, as the linker flag takes it.
const SEARCH_SYSTEM32: &str = "0x800";

fn main() {
    let windows = std::env::var("CARGO_CFG_TARGET_OS").is_ok_and(|os| os == "windows");
    let msvc = std::env::var("CARGO_CFG_TARGET_ENV").is_ok_and(|env| env == "msvc");
    if windows && msvc {
        println!("cargo:rustc-link-arg-bins=/DEPENDENTLOADFLAG:{SEARCH_SYSTEM32}");
    }
}
