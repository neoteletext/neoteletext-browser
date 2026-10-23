//! Builds the release's identity into the programs that pin it
//! (`installation-and-updates.md` section 14): the update host, its port
//! and, for a test, its certificate's root; The Update Framework's first
//! root metadata; and the platform signers the launcher accepts. They are
//! read at build time from the folder `NEOTELETEXT_RELEASE_IDENTITY`
//! names, else from `production/` beside this file, and become constants:
//! no program reads them at run time, so nothing a user's machine holds
//! can change what a build trusts (decision 58).
//!
//! A file that is absent leaves its constant `None`, and the program
//! that needs it refuses to work: until the key ceremony
//! (`PENDINGTASKSFORANDREW.md`), the production identity names only the
//! host, so a production build stages and accepts nothing.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

/// The variable naming a test identity's folder.
const VARIABLE: &str = "NEOTELETEXT_RELEASE_IDENTITY";

/// A file of the identity, if present, as text without its line end.
fn text(folder: &Path, name: &str) -> Option<String> {
    let path = folder.join(name);
    println!("cargo:rerun-if-changed={}", path.display());
    let text = std::fs::read_to_string(&path).ok()?;
    let text = text.strip_suffix('\n').unwrap_or(&text);
    // One line, written into a raw string: a designated requirement quotes
    // its values (`certificate leaf = H"..."`).
    assert!(!text.contains('\n') && !text.contains("\"#"), "{} must be one line", path.display());
    Some(text.to_owned())
}

/// A file of the identity, if present, by its absolute path.
fn file(folder: &Path, name: &str) -> Option<PathBuf> {
    let path = folder.join(name);
    println!("cargo:rerun-if-changed={}", path.display());
    path.is_file().then(|| path.canonicalize().unwrap_or(path))
}

/// Thirty-two bytes written in lower-case hexadecimal, as an array.
fn key(folder: &Path, name: &str) -> Option<String> {
    let text = text(folder, name)?;
    assert!(text.len() == 64 && text.bytes().all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f')), "{name} must be 64 lower-case hexadecimal digits");
    let bytes: Vec<String> = (0..32).map(|at| format!("0x{}", &text[at * 2..at * 2 + 2])).collect();
    Some(format!("[{}]", bytes.join(", ")))
}

fn main() {
    println!("cargo:rerun-if-env-changed={VARIABLE}");
    let manifest = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").expect("Cargo sets CARGO_MANIFEST_DIR"));
    // An empty variable is no identity named, so the production one, as a
    // release script that passes the variable on unset makes it (F-24).
    let (folder, name) = match std::env::var_os(VARIABLE).filter(|folder| !folder.is_empty()) {
        Some(folder) => {
            let folder = PathBuf::from(folder);
            let name = format!("test identity {}", folder.display());
            (folder, name)
        }
        None => (manifest.join("production"), String::from("production")),
    };
    let host = text(&folder, "host").expect("an identity names its update host");
    assert!(!host.is_empty() && host.bytes().all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'.'), "the host must be a plain DNS name");
    let port: u16 = text(&folder, "port").map_or(443, |port| port.parse().expect("the port must be a number"));
    let optional_bytes = |path: Option<PathBuf>| path.map_or_else(|| String::from("None"), |path| format!("Some(include_bytes!(r\"{}\"))", path.display()));
    let optional_key = |key: Option<String>| key.map_or_else(|| String::from("None"), |key| format!("Some({key})"));
    let mut out = String::new();
    let _ = writeln!(out, "/// Which identity this build carries.\npub const NAME: &str = r\"{name}\";");
    let kind = if name == "production" { "production" } else { "test" };
    let _ = writeln!(
        out,
        "/// The identity mark (`update::identity_mark!`) a program carries in its bytes, read by the release tool's `check`.\npub const IDENTITY_MARK: &str = \"\\u{{1}}neoteletext-identity\\u{{1}}{kind}\\u{{1}}\";"
    );
    let _ = writeln!(out, "/// The update host (decisions 17 and 25).\npub const HOST: &str = \"{host}\";");
    let _ = writeln!(out, "/// Its port.\npub const PORT: u16 = {port};");
    let _ = writeln!(
        out,
        "/// A test host's certificate root, DER; Mozilla's roots otherwise.\npub const TLS_ROOT: Option<&[u8]> = {};",
        optional_bytes(file(&folder, "tls-root.der"))
    );
    let _ = writeln!(
        out,
        "/// The Update Framework's first trusted root metadata.\npub const TUF_ROOT: Option<&[u8]> = {};",
        optional_bytes(file(&folder, "root.json"))
    );
    // The file keeps the name it had when the key signed Linux's files
    // alone; it signs the Windows manifest too (decision 92).
    let _ = writeln!(
        out,
        "/// The release key: Linux's files and the Linux and Windows manifests.\npub const RELEASE_KEY: Option<[u8; 32]> = {};",
        optional_key(key(&folder, "linux-release.pub"))
    );
    let requirement = text(&folder, "macos-requirement").map_or_else(|| String::from("None"), |requirement| format!("Some(r#\"{requirement}\"#)"));
    let _ = writeln!(out, "/// The macOS designated requirement.\npub const MACOS_REQUIREMENT: Option<&str> = {requirement};");
    let _ = writeln!(
        out,
        "/// SHA-256 of the Windows signer's public key.\npub const WINDOWS_SIGNER: Option<[u8; 32]> = {};",
        optional_key(key(&folder, "windows-signer"))
    );
    let target = PathBuf::from(std::env::var_os("OUT_DIR").expect("Cargo sets OUT_DIR")).join("identity.rs");
    std::fs::write(target, out).expect("the identity can be written");
}
