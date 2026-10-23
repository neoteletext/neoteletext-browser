//! macOS: one bundle, swapped whole (`installation-and-updates.md`
//! section 4.1). The installed bundle is the running version; the state
//! says which it should be. The bundle's own bytes say which it is (its
//! broker's version mark), so after a power cut at any step the launcher
//! finds the bundles where they are and finishes or undoes the swap:
//!
//! - The trial's bundle is staged in `staging/<trial>/NeoTeletext.app`
//!   and exchanged with the installed one (`renamex_np`, `RENAME_SWAP`);
//!   the old current bundle is then in the staging folder, and is moved
//!   to `previous/`, from where a rollback exchanges it back.
//! - The state is written before each exchange, so it names what the
//!   installed bundle is to be, and [`reconcile`] makes it so.

use std::path::{Path, PathBuf};

use update::fault;
use update::files;
use update::manifest::MAX_FILE_BYTES;
use update::platform::{BUNDLE, MANIFEST, MARKED_PROGRAM};
use update::state::State;
use update::version::Version;

use crate::{Launcher, io_text};

/// The version a bundle is, by its broker's mark.
pub fn version_of(bundle: &Path) -> Result<Version, String> {
    let program = bundle.join("Contents/MacOS").join(MARKED_PROGRAM);
    let bytes = files::read_bounded(&program, usize::try_from(MAX_FILE_BYTES).unwrap_or(usize::MAX)).map_err(|error| io_text(&program, &error))?;
    update::mark::find(&bytes).map_err(|refused| format!("{}: {refused}", program.display()))
}

fn bundle(launcher: &Launcher) -> Result<&Path, String> {
    launcher.layout.bundle.as_deref().ok_or_else(|| String::from("no installed bundle"))
}

/// The installed bundle's version.
pub fn installed(launcher: &Launcher) -> Result<Version, String> {
    version_of(bundle(launcher)?)
}

/// Makes the installed bundle the version the state runs, by exchanging
/// it with the previous or the staged bundle of that version, or, when
/// that version is nowhere, makes the state name what is installed.
/// Whether the state changed and is to be written.
pub fn reconcile(launcher: &Launcher, state: &mut State) -> Result<bool, String> {
    let installed_bundle = bundle(launcher)?.to_path_buf();
    let installed = version_of(&installed_bundle)?;
    let wanted = state.running();
    if installed == wanted {
        move_old_current(launcher, state);
        return Ok(false);
    }
    let previous = launcher.layout.previous_bundle();
    let staged = launcher.layout.staged(wanted).join(BUNDLE);
    let source = [previous, staged].into_iter().find(|candidate| version_of(candidate).ok() == Some(wanted));
    if let Some(source) = source {
        files::exchange(&source, &installed_bundle)
            .map_err(|error| format!("cannot exchange {} with {}: {error}", source.display(), installed_bundle.display()))?;
        fault::point("bundle-exchanged");
        eprintln!("neoteletext: the installed bundle is now version {wanted}");
        move_old_current(launcher, state);
        return Ok(false);
    }
    // Neither bundle is there: run what is installed, and say so.
    eprintln!("neoteletext: version {wanted} is not here; running the installed {installed}");
    if state.trial.is_some_and(|trial| trial.version == wanted) {
        state.trial = None;
    }
    if state.running() != installed {
        state.current = installed;
        state.trial = None;
    }
    Ok(true)
}

/// Whether `folder`, in the staging folder, holds the old current bundle
/// after a trial's exchange, to be moved to `previous/`.
pub fn holds_old_current(launcher: &Launcher, state: &State, folder: &Path) -> bool {
    state.running() != state.current
        && version_of(&folder.join(BUNDLE)).ok() == Some(state.current)
        && launcher.layout.previous_bundle().as_path() != folder.join(BUNDLE)
}

/// After a trial's exchange, its staging folder holds the old current
/// bundle: it goes to `previous/`, replacing what was there.
fn move_old_current(launcher: &Launcher, state: &State) {
    let Ok(entries) = std::fs::read_dir(launcher.layout.staging()) else { return };
    let holding: Vec<PathBuf> = entries.filter_map(Result::ok).map(|entry| entry.path()).filter(|path| holds_old_current(launcher, state, path)).collect();
    for folder in holding {
        let previous = launcher.layout.previous();
        let discard = files::with_suffix(&previous, ".discard");
        let _ = files::remove_tree(&discard);
        if previous.exists() && std::fs::rename(&previous, &discard).is_err() {
            continue;
        }
        fault::point("previous-discarded");
        if std::fs::rename(&folder, &previous).is_ok() {
            fault::point("previous-moved");
            let _ = std::fs::remove_file(previous.join(MANIFEST));
            let _ = files::sync_folder(&launcher.layout.root);
        }
        let _ = files::remove_tree(&discard);
    }
}

/// `previous/` keeps only the bundle a rollback would run: the current
/// version's during a trial, the previous version's after a commit.
pub fn tidy_previous(launcher: &Launcher, state: &State) {
    let previous = launcher.layout.previous_bundle();
    let Ok(held) = version_of(&previous) else {
        let _ = files::remove_tree(&launcher.layout.previous());
        return;
    };
    let installed = installed(launcher).ok();
    let useful = Some(held) != installed && (Some(held) == state.previous || (state.trial.is_some() && held == state.current));
    if !useful {
        let _ = files::remove_tree(&launcher.layout.previous());
    }
    let _ = files::remove_tree(&files::with_suffix(&launcher.layout.previous(), ".discard"));
}

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
