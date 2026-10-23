//! macOS: one bundle, swapped whole (`installation-and-updates.md`
//! section 4.1). The installed bundle is the running version; the state
//! says which it should be. The bundle's own bytes say which it is (its
//! broker's version mark), so after a power cut at any step the launcher
//! finds the bundles where they are and finishes or undoes the swap:
//!
//! - The trial's bundle is the launcher's own copy of what was staged,
//!   accepted in `accepting/<trial>/NeoTeletext.app` (decision 87), and
//!   exchanged with the installed one (`renamex_np`, `RENAME_SWAP`); the
//!   old current bundle is then in that folder, and is moved to
//!   `previous/`, from where a rollback exchanges it back.
//! - The state is written before each exchange, so it names what the
//!   installed bundle is to be, and [`reconcile`] makes it so.

use std::path::Path;

use update::fault;
use update::files;
use update::manifest::{MAX_FILE_BYTES, Manifest};
use update::platform::{BUNDLE, MANIFEST, MARKED_PROGRAM};
use update::state::State;
use update::version::Version;

use crate::{Launcher, io_text};

mod translocation;

pub use translocation::{must_move, offer_move};

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
/// it with the previous bundle or the accepted copy of that version, or,
/// when that version is nowhere, makes the state name what is installed.
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
    let accepted = launcher.layout.accepted(wanted).join(BUNDLE);
    let source = [previous, accepted].into_iter().find(|candidate| version_of(candidate).ok() == Some(wanted));
    if let Some(source) = source {
        // The bundle about to run is checked by the pinned signer first:
        // `previous/` has been out of the launcher's sight since it was
        // moved there (finding F-19). Refused, nothing is exchanged and
        // nothing runs; the state keeps what it says.
        let folder = source.parent().ok_or("a bundle with no folder")?;
        let manifest = Manifest { version: wanted, platform: launcher.platform, files: Vec::new() };
        if let Err(why) = launcher.signer.check(folder, &manifest) {
            eprintln!("neoteletext: the bundle of version {wanted} at {} is refused, and not exchanged in: {why}", source.display());
            return Err(format!("version {wanted}, to be run, is not signed by the pinned signer; install NeoTeletext again"));
        }
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

/// After a trial's exchange, the trial's accepted folder holds the old
/// current bundle: it goes to `previous/`, replacing what was there. Only
/// that folder is looked at, which only the launcher writes.
fn move_old_current(launcher: &Launcher, state: &State) {
    let Some(trial) = state.trial else { return };
    let folder = launcher.layout.accepted(trial.version);
    if version_of(&folder.join(BUNDLE)).ok() != Some(state.current) {
        return;
    }
    let previous = launcher.layout.previous();
    let discard = files::with_suffix(&previous, ".discard");
    let _ = files::remove_tree(&discard);
    if previous.exists() && std::fs::rename(&previous, &discard).is_err() {
        return;
    }
    fault::point("previous-discarded");
    if std::fs::rename(&folder, &previous).is_ok() {
        fault::point("previous-moved");
        let _ = std::fs::remove_file(previous.join(MANIFEST));
        let _ = files::sync_folder(&launcher.layout.root);
    }
    let _ = files::remove_tree(&discard);
}

/// The launcher's copies no longer needed: all but the trial's, while its
/// bundle is still to be exchanged in or the old current bundle still to
/// be moved out.
pub fn tidy_accepting(launcher: &Launcher, state: &State) {
    let Ok(entries) = std::fs::read_dir(launcher.layout.accepting()) else { return };
    let kept = state.trial.map(|trial| launcher.layout.accepted(trial.version));
    for entry in entries.filter_map(Result::ok) {
        if Some(entry.path()) != kept {
            let _ = files::remove_tree(&entry.path());
        }
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
