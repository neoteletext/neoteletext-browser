//! Which files the broker reads for the trusted core (DEC-1551 items 1 and
//! 3, REQ-1698): those granted, those named to the broker itself, and those
//! the interface declared on its own channel a moment ago. This is what stops
//! a compromised core reading a file nobody gave it, and it needs two
//! parties: the core's word alone is never enough, and the interface's only
//! for [`DECLARED_FOR`].

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use confine::FileIdentity;
use confine::broker::{Declaration, MAX_FILES_AT_ONCE};
use neoteletext::address::typed_path;
use neoteletext::grants::{Grant, Grants};

/// How long a declaration allows a read: the core reads what it is told to
/// open at once, so this need only outlast several files of up to 64 MB read
/// in turn, and is short so that a declaration not followed by its read
/// leaves nothing standing.
const DECLARED_FOR: Duration = Duration::from_secs(10);
/// The most declarations waiting: a few drops' worth. The oldest go first.
const MAX_WAITING: usize = 4 * MAX_FILES_AT_ONCE;

/// What the interface declared, waiting for its read.
#[derive(Debug)]
struct Declared {
    path: PathBuf,
    at: Instant,
    keep: bool,
}

/// May this file be read for the core, and on what terms.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Admission {
    /// Nobody gave it.
    Refused,
    /// Its grant stands, while it is this file.
    Granted(FileIdentity),
    /// Given just now: a read makes it a grant, kept or for the run.
    Fresh {
        /// The grant is kept.
        keep: bool,
    },
}

/// The broker's table.
#[derive(Debug, Default)]
pub(super) struct Access {
    /// Kept between runs.
    kept: Grants,
    /// For the run alone: from a private window (DEC-1549 item 7).
    run: Grants,
    /// Named on the command line, by a second start or in the chooser.
    named: Vec<(PathBuf, bool)>,
    declared: Vec<Declared>,
    /// Where `kept` is written.
    folder: Option<PathBuf>,
}

impl Access {
    /// The table kept in `folder`.
    pub(super) fn load(folder: Option<PathBuf>) -> Self {
        let kept = folder.as_deref().map(|folder| Grants::load(folder).0).unwrap_or_default();
        Self { kept, folder, ..Self::default() }
    }

    /// The broker itself was told of `path`: on its command line, by a second
    /// start, or in the chooser. It stays allowed until it is read.
    pub(super) fn name(&mut self, path: &Path, keep: bool) {
        if let Ok(canonical) = path.canonicalize()
            && !self.named.iter().any(|(named, _)| *named == canonical)
        {
            self.named.push((canonical, keep));
            let excess = self.named.len().saturating_sub(MAX_WAITING);
            self.named.drain(..excess);
        }
    }

    /// What the interface declared at `now`, each path written as an address
    /// or a path, read by the rules typed paths are read by. A path that
    /// does not exist has nothing to allow.
    pub(super) fn declare(&mut self, declaration: &Declaration, home: Option<&Path>, now: Instant) {
        for text in &declaration.paths {
            let path = typed_path(text, home).map(PathBuf::from).and_then(|path| path.canonicalize().ok());
            if let Some(path) = path {
                self.declared.push(Declared { path, at: now, keep: declaration.keep });
            }
        }
        let excess = self.declared.len().saturating_sub(MAX_WAITING);
        self.declared.drain(..excess);
    }

    /// Whether `canonical` may be read at `now`. What was given just now
    /// comes before a grant, so that choosing a file again renews a grant
    /// whose file was replaced.
    pub(super) fn admit(&mut self, canonical: &Path, now: Instant) -> Admission {
        self.declared.retain(|declared| now.saturating_duration_since(declared.at) < DECLARED_FOR);
        if let Some((_, keep)) = self.named.iter().find(|(named, _)| named == canonical) {
            return Admission::Fresh { keep: *keep };
        }
        if let Some(declared) = self.declared.iter().find(|declared| declared.path == canonical) {
            return Admission::Fresh { keep: declared.keep };
        }
        match self.run.find(canonical).or_else(|| self.kept.find(canonical)) {
            Some(grant) => Admission::Granted(grant.identity),
            None => Admission::Refused,
        }
    }

    /// `canonical` was read as `identity`: it is granted, and what allowed
    /// it is spent.
    pub(super) fn read(&mut self, canonical: &Path, identity: FileIdentity, keep: bool) {
        self.named.retain(|(named, _)| named != canonical);
        self.declared.retain(|declared| declared.path != canonical);
        let grant = Grant { path: canonical.to_path_buf(), identity };
        if keep {
            self.kept.record(grant);
            self.write();
        } else {
            self.run.record(grant);
        }
    }

    /// Where the chooser starts.
    pub(super) fn last_folder(&self) -> Option<PathBuf> {
        self.kept.last_folder.clone()
    }

    /// The chooser chose in `folder`, not from a private window.
    pub(super) fn set_last_folder(&mut self, folder: Option<&Path>) {
        self.kept.last_folder = folder.map(Path::to_path_buf);
        self.write();
    }

    /// Writes the kept grants; a failure leaves them for the run, and is
    /// said.
    fn write(&self) {
        if let Some(folder) = &self.folder
            && let Err(error) = self.kept.save(folder)
        {
            eprintln!("neoteletext: the grants could not be kept: {error}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn folder(name: &str) -> PathBuf {
        let folder = std::env::temp_dir().join(format!("neoteletext-access-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        std::fs::create_dir_all(&folder).unwrap();
        folder.canonicalize().unwrap()
    }

    fn file(folder: &Path, name: &str) -> PathBuf {
        let path = folder.join(name);
        std::fs::write(&path, b"PN,10000\n").unwrap();
        path
    }

    const IDENTITY: FileIdentity = FileIdentity { volume: 1, file: 2 };

    fn declaration(path: &Path, keep: bool) -> Declaration {
        Declaration { paths: vec![path.display().to_string()], keep }
    }

    /// REQ-1698: the core's word alone reads nothing.
    #[test]
    fn nobody_gave_it_so_it_is_refused() {
        let home = folder("refused");
        let path = file(&home, "a.tti");
        assert_eq!(Access::default().admit(&path, Instant::now()), Admission::Refused);
        std::fs::remove_dir_all(&home).unwrap();
    }

    /// REQ-1698 item 3: a declaration allows a read for `DECLARED_FOR`, and
    /// a typed `~` is the home folder.
    #[test]
    fn a_declaration_lasts_a_moment() {
        let home = folder("declared");
        let path = file(&home, "a.tti");
        let now = Instant::now();
        let mut access = Access::default();
        access.declare(&declaration(&path, true), None, now);
        assert_eq!(access.admit(&path, now + DECLARED_FOR.saturating_sub(Duration::from_millis(1))), Admission::Fresh { keep: true });
        assert_eq!(access.admit(&path, now + DECLARED_FOR), Admission::Refused);

        let mut access = Access::default();
        access.declare(&Declaration { paths: vec!["~/a.tti".into()], keep: true }, Some(&home), now);
        assert_eq!(access.admit(&path, now), Admission::Fresh { keep: true });
        std::fs::remove_dir_all(&home).unwrap();
    }

    /// REQ-1698: a read leaves a grant a later run honours; a private
    /// window's stays for the run, and is in no file.
    #[test]
    fn a_read_leaves_a_grant_that_a_private_window_does_not_keep() {
        let home = folder("kept");
        let (kept, private) = (file(&home, "kept.tti"), file(&home, "private.tti"));
        let store = home.join("broker");
        let now = Instant::now();
        let mut access = Access::load(Some(store.clone()));
        access.declare(&declaration(&kept, true), None, now);
        access.declare(&declaration(&private, false), None, now);
        access.read(&kept, IDENTITY, true);
        access.read(&private, IDENTITY, false);
        let later = now + 2 * DECLARED_FOR;
        assert_eq!(access.admit(&kept, later), Admission::Granted(IDENTITY));
        assert_eq!(access.admit(&private, later), Admission::Granted(IDENTITY), "for the run");
        let mut next_run = Access::load(Some(store.clone()));
        assert_eq!(next_run.admit(&kept, later), Admission::Granted(IDENTITY));
        assert_eq!(next_run.admit(&private, later), Admission::Refused);
        assert_eq!(std::fs::read_to_string(store.join("grants.txt")).unwrap().lines().count(), 2, "its first line, and the one grant kept");
        std::fs::remove_dir_all(&home).unwrap();
    }

    /// REQ-1698 item 2: what the broker itself was told allows a read with
    /// nothing declared, and is spent by it.
    #[test]
    fn a_path_named_to_the_broker_needs_no_declaration() {
        let home = folder("named");
        let path = file(&home, "a.tti");
        let now = Instant::now();
        let mut access = Access::default();
        access.name(&path, true);
        assert_eq!(access.admit(&path, now + 2 * DECLARED_FOR), Admission::Fresh { keep: true });
        access.read(&path, IDENTITY, true);
        assert_eq!(access.admit(&path, now), Admission::Granted(IDENTITY));
        std::fs::remove_dir_all(&home).unwrap();
    }

    /// Choosing a file again renews a grant whose file was replaced.
    #[test]
    fn what_was_given_just_now_comes_before_a_grant() {
        let home = folder("renew");
        let path = file(&home, "a.tti");
        let now = Instant::now();
        let mut access = Access::default();
        access.read(&path, IDENTITY, false);
        access.name(&path, true);
        assert_eq!(access.admit(&path, now), Admission::Fresh { keep: true });
        std::fs::remove_dir_all(&home).unwrap();
    }

    /// What waits is bounded, whatever the interface sends.
    #[test]
    fn declarations_waiting_are_bounded() {
        let home = folder("bound");
        let path = file(&home, "a.tti");
        let now = Instant::now();
        let mut access = Access::default();
        for _ in 0..=MAX_WAITING {
            access.declare(&declaration(&path, true), None, now);
        }
        assert_eq!(access.declared.len(), MAX_WAITING);
        std::fs::remove_dir_all(&home).unwrap();
    }
}
