//! macOS translocation (section 4.1): a bundle opened from a disk image,
//! with its quarantine attribute, or from a folder the user cannot write
//! is moved to Applications before it runs.

use std::path::{Path, PathBuf};

use update::files;
use update::platform::BUNDLE;

use crate::io_text;

/// Whether a bundle at `bundle` must be moved before it is run: macOS
/// translocates a bundle opened from a disk image or with its quarantine
/// attribute to a random read-only path (section 4.1), and a bundle whose
/// folder the user cannot write cannot be swapped.
#[must_use]
pub fn must_move(bundle: &Path) -> bool {
    let translocated = bundle.components().any(|part| part.as_os_str() == "AppTranslocation");
    let writable = bundle.parent().is_some_and(writable_folder);
    translocated || !writable
}

/// Whether this user may make a file in `folder`.
fn writable_folder(folder: &Path) -> bool {
    let probe = folder.join(format!(".neoteletext-probe-{}", std::process::id()));
    match std::fs::File::create(&probe) {
        Ok(_) => {
            let _ = std::fs::remove_file(&probe);
            true
        }
        Err(_) => false,
    }
}

/// Where a moved bundle goes: `/Applications` for an administrator, who
/// may write it without elevating, else `~/Applications` (requirement
/// 13.15).
#[must_use]
pub fn applications() -> Option<PathBuf> {
    let system = PathBuf::from("/Applications");
    if writable_folder(&system) {
        return Some(system);
    }
    let home = PathBuf::from(std::env::var_os("HOME")?).join("Applications");
    std::fs::create_dir_all(&home).ok()?;
    Some(home)
}

/// Offers to move the translocated or unwritable bundle at `from` to
/// Applications, and does nothing else until it has (section 4.1): the
/// copy is then opened and this start ends.
///
/// # Errors
/// When the user declines, or the copy fails.
pub fn offer_move(from: &Path) -> Result<(), String> {
    let asked = os_install::ask(
        "NeoTeletext",
        "NeoTeletext must be in your Applications folder to keep itself up to date. Move it there now?",
        "Move to Applications",
        "Quit",
    );
    if !asked {
        return Err(String::from("NeoTeletext was not moved to Applications, so it does not start"));
    }
    let folder = applications().ok_or("no Applications folder can be written")?;
    let to = folder.join(BUNDLE);
    if to.exists() {
        os_install::tell("NeoTeletext", &format!("NeoTeletext is already in {}. Open that one.", folder.display()));
        return Err(format!("{} exists", to.display()));
    }
    let partial = files::with_suffix(&to, ".partial");
    let _ = files::remove_tree(&partial);
    files::copy_tree(from, &partial).map_err(|error| format!("cannot copy to {}: {error}", partial.display()))?;
    std::fs::rename(&partial, &to).map_err(|error| io_text(&to, &error))?;
    std::process::Command::new("/usr/bin/open").arg(&to).status().map_err(|error| format!("cannot open {}: {error}", to.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_translocated_bundle_must_move() {
        assert!(must_move(Path::new("/private/var/folders/xy/T/AppTranslocation/1234/d/NeoTeletext.app")));
        assert!(must_move(Path::new("/System/NeoTeletext.app")), "a folder the user cannot write");
        assert!(!must_move(&std::env::temp_dir().join("NeoTeletext.app")));
    }
}
