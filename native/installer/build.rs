//! The release the installer carries (`installation-and-updates.md`
//! section 4.2): the folder `NEOTELETEXT_INSTALLER_PAYLOAD` names at build
//! time holds `NeoTeletext.exe` (the stub), `launcher.exe`, and one
//! version's folder, named by its version, as the release tool laid it
//! out, signed and listed. Each file is built into the installer, so the
//! one signed program carries the whole first install; without the
//! variable the installer carries nothing and installs nothing.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

const VARIABLE: &str = "NEOTELETEXT_INSTALLER_PAYLOAD";

/// Every file under `folder`, by its path there with `/` between parts.
fn files(base: &Path, folder: &Path, out: &mut Vec<(String, PathBuf)>) {
    for entry in std::fs::read_dir(folder).expect("the payload's folders can be read") {
        let path = entry.expect("the payload's folders can be read").path();
        if path.is_dir() {
            files(base, &path, out);
        } else {
            let relative = path.strip_prefix(base).expect("under the payload").to_str().expect("a name that is text").replace('\\', "/");
            out.push((relative, path.canonicalize().expect("a file of the payload")));
        }
    }
}

fn main() {
    println!("cargo:rerun-if-env-changed={VARIABLE}");
    let mut out = String::from("/// The release's files, by their paths in the payload.\npub const FILES: &[(&str, &[u8])] = &[\n");
    if let Some(folder) = std::env::var_os(VARIABLE) {
        let folder = PathBuf::from(folder);
        println!("cargo:rerun-if-changed={}", folder.display());
        let mut found = Vec::new();
        files(&folder, &folder, &mut found);
        found.sort();
        for (name, path) in found {
            println!("cargo:rerun-if-changed={}", path.display());
            let _ = writeln!(out, "    (\"{name}\", include_bytes!(r\"{}\")),", path.display());
        }
    }
    out.push_str("];\n");
    let target = PathBuf::from(std::env::var_os("OUT_DIR").expect("Cargo sets OUT_DIR")).join("payload.rs");
    std::fs::write(target, out).expect("the payload's list can be written");
}
