//! The launcher's life of an update (`installation-and-updates.md`
//! sections 3.1, 4 and 13), on the platform the tests run on, with fake
//! versions whose broker is `launcher-test-broker`: acceptance, trial,
//! commit, rollback of a version that crashes, hangs or says nothing,
//! refusal of a wrongly signed one, and every fault point of the launcher
//! killed in turn, after which the next start runs a complete version and
//! finishes the update.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing, reason = "a test")]

use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use launcher::{Chosen, HEALTH_DEADLINE, Launcher};
use ring::signature::{Ed25519KeyPair, KeyPair as _};
use update::accept::{LinuxSigner, Signer};
use update::layout::Layout;
use update::manifest::{Entry, Manifest};
use update::platform::{BUNDLE, Family, MANIFEST, MANIFEST_SIGNATURE, MARKED_PROGRAM, Platform, SIGNATURE_SUFFIX};
use update::state::State;
use update::version::Version;

const OLD: Version = Version::new(0, 1, 0);
const NEW: Version = Version::new(0, 2, 0);
/// What the test signer looks for in a broker.
const SIGNED: &[u8] = b"\x02signed=test\x02";
/// A trial that hangs is given this long, not the real deadline: long
/// enough for a loaded machine to start a program it has not run before,
/// which macOS first checks (measured 23 October 2026: over 3 seconds while
/// the workspace's tests ran beside it).
const SHORT_DEADLINE: Duration = Duration::from_secs(15);
/// A deadline no test waits out: a version that is not to hang is given
/// this long, so a loaded machine starting it slowly is not taken for one
/// that hangs.
const NO_DEADLINE_NEEDED: Duration = Duration::from_secs(5 * 60);
/// The fault child's variable: the fixture it runs.
const CHILD: &str = "LAUNCHER_TEST_FAULT_CHILD";

/// The platform's signer for tests: on Linux the real one with a test
/// key; elsewhere a check of the test broker's appended signature, since
/// the real check (`codesign`, Authenticode) is the platform's and is
/// exercised end to end with test certificates.
struct TestSigner;

impl Signer for TestSigner {
    fn check(&self, folder: &Path, _manifest: &Manifest) -> Result<(), String> {
        let broker = fs::read(folder.join(platform().program_path(MARKED_PROGRAM))).map_err(|error| error.to_string())?;
        if broker.windows(SIGNED.len()).any(|window| window == SIGNED) { Ok(()) } else { Err(String::from("not signed by the test signer")) }
    }
}

fn platform() -> Platform {
    Platform::this().unwrap()
}

/// The test's installation, under the test's own folder.
struct Fixture {
    folder: PathBuf,
    layout: Layout,
    pair: Ed25519KeyPair,
}

/// The test key, the same in a fault child as in its parent: made from
/// a fixed seed (RFC 8032 section 7.1's first test secret key).
fn test_pair() -> Ed25519KeyPair {
    let seed = update::from_hex::<32>("9d61b19deffd5a60ba844af492ec2cc44449c5697b326919703bac031cae7f60").unwrap();
    let public = update::from_hex::<32>("d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a").unwrap();
    Ed25519KeyPair::from_seed_and_public_key(&seed, &public).unwrap()
}

impl Fixture {
    fn new(name: &str) -> Self {
        let folder = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("launcher-{name}"));
        let _ = fs::remove_dir_all(&folder);
        fs::create_dir_all(&folder).unwrap();
        Self::at(folder)
    }

    fn at(folder: PathBuf) -> Self {
        let family = platform().family();
        let root = folder.join("installation");
        let bundle = (family == Family::Macos).then(|| folder.join("Applications").join(BUNDLE));
        let layout = Layout { family, root, bundle };
        Self { folder, layout, pair: test_pair() }
    }

    fn launcher(&self, deadline: Duration) -> Launcher {
        let signer: Box<dyn Signer> = match platform().family() {
            Family::Linux => Box::new(LinuxSigner { key: Some(self.pair.public_key().as_ref().try_into().unwrap()) }),
            Family::Macos | Family::Windows => Box::new(TestSigner),
        };
        let own = match platform().family() {
            Family::Linux => self.layout.launcher_folder().join("neoteletext"),
            Family::Windows => self.layout.root.join("launcher-a.exe"),
            Family::Macos => self.layout.bundle.clone().unwrap().join("Contents/MacOS/neoteletext-launcher"),
        };
        Launcher { platform: platform(), layout: self.layout.clone(), signer, own, health_deadline: deadline }
    }

    /// A complete version in `folder`, its broker behaving as `behaviour`.
    fn version(&self, folder: &Path, version: Version, behaviour: &str, signed: bool) {
        let platform = platform();
        let mut entries = Vec::new();
        for file in platform.files().into_iter().filter(|file| file.required && !file.path.ends_with(SIGNATURE_SUFFIX)) {
            let path = folder.join(&file.path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            let bytes = if file.path == platform.program_path(MARKED_PROGRAM) {
                let mut bytes = fs::read(env!("CARGO_BIN_EXE_launcher-test-broker")).unwrap();
                bytes.extend_from_slice(update::version_mark!("VERSION").replace("VERSION", &version.to_string()).as_bytes());
                bytes.extend_from_slice(format!("\x02behaviour={behaviour}\x02").as_bytes());
                if signed {
                    bytes.extend_from_slice(SIGNED);
                }
                bytes
            } else if file.path.ends_with("neoteletext-launcher") || file.path.ends_with("neoteletext-launcher.exe") {
                b"the launcher of this release".to_vec()
            } else {
                format!("{} of {version}", file.path).into_bytes()
            };
            update::files::write_synced(&path, &bytes, file.executable).unwrap();
            entries.push(Entry { path: file.path.clone(), length: bytes.len() as u64, sha256: update::sha256(&bytes) });
            if platform.family() == Family::Linux {
                let signature = if signed { self.pair.sign(&bytes).as_ref().to_vec() } else { vec![0; 64] };
                fs::write(folder.join(format!("{}{SIGNATURE_SUFFIX}", file.path)), &signature).unwrap();
                entries.push(Entry { path: format!("{}{SIGNATURE_SUFFIX}", file.path), length: 64, sha256: update::sha256(&signature) });
            }
        }
        entries.sort_by(|a, b| a.path.cmp(&b.path));
        let manifest = Manifest { version, platform, files: entries }.to_text();
        fs::write(folder.join(MANIFEST), &manifest).unwrap();
        if platform.family() == Family::Linux {
            fs::write(folder.join(MANIFEST_SIGNATURE), self.pair.sign(manifest.as_bytes()).as_ref()).unwrap();
        }
    }

    /// `version` installed and current.
    fn install(&self, version: Version, behaviour: &str) {
        match platform().family() {
            Family::Macos => {
                let made = self.folder.join("made");
                self.version(&made, version, behaviour, true);
                let bundle = self.layout.bundle.clone().unwrap();
                fs::create_dir_all(bundle.parent().unwrap()).unwrap();
                fs::rename(made.join(BUNDLE), &bundle).unwrap();
                fs::remove_dir_all(made).unwrap();
            }
            Family::Linux | Family::Windows => {
                self.version(&self.layout.version(version), version, behaviour, true);
                let own = self.launcher(HEALTH_DEADLINE).own;
                fs::create_dir_all(own.parent().unwrap()).unwrap();
                fs::write(&own, b"the launcher running now").unwrap();
                if platform().family() == Family::Windows {
                    fs::write(self.layout.root.join("launcher"), b"a").unwrap();
                }
            }
        }
        fs::create_dir_all(&self.layout.root).unwrap();
        fs::write(self.layout.state(), State::installed(version).to_text()).unwrap();
    }

    fn stage(&self, version: Version, behaviour: &str, signed: bool) {
        self.version(&self.layout.staged(version), version, behaviour, signed);
    }

    fn state(&self) -> State {
        State::parse(&fs::read(self.layout.state()).unwrap()).unwrap()
    }

    fn log(&self) -> String {
        fs::read_to_string(self.folder.join("log")).unwrap_or_default()
    }

    /// Runs the launcher; the test broker is given its log's path as the
    /// browser's argument.
    fn run(&self, deadline: Duration) -> Option<i32> {
        self.launcher(deadline).run(&[OsString::from(self.folder.join("log"))]).unwrap()
    }

    /// The bytes of the broker that runs as `version` once it is the
    /// trial: the installed bundle's on macOS, the version's folder's
    /// elsewhere.
    fn broker_of(&self, version: Version) -> Vec<u8> {
        let program = match platform().family() {
            Family::Macos => self.layout.bundle.clone().unwrap().join("Contents/MacOS").join(MARKED_PROGRAM),
            Family::Linux | Family::Windows => self.layout.version(version).join(platform().program_file(MARKED_PROGRAM)),
        };
        fs::read(program).unwrap()
    }

    /// The staged broker of `version`.
    fn staged_broker(&self, version: Version) -> PathBuf {
        self.layout.staged(version).join(platform().program_path(MARKED_PROGRAM))
    }

    /// The installed version, by its broker's mark.
    fn installed(&self) -> Version {
        let program = match platform().family() {
            Family::Macos => self.layout.bundle.clone().unwrap().join("Contents/MacOS").join(MARKED_PROGRAM),
            Family::Linux | Family::Windows => self.layout.version(self.state().running()).join(platform().program_file(MARKED_PROGRAM)),
        };
        update::mark::find(&fs::read(program).unwrap()).unwrap()
    }
}

/// Section 3.1: a staged version newer than the current one is accepted,
/// tried, found healthy and committed; the old one is kept as previous.
#[test]
fn a_staged_version_is_accepted_tried_and_committed() {
    let fixture = Fixture::new("commit");
    fixture.install(OLD, "healthy");
    assert_eq!(fixture.run(HEALTH_DEADLINE), Some(0));
    assert_eq!(fixture.state(), State::installed(OLD), "nothing to update");
    fixture.stage(NEW, "healthy", true);
    assert_eq!(fixture.run(HEALTH_DEADLINE), Some(0));
    let state = fixture.state();
    assert_eq!((state.current, state.previous, state.trial), (NEW, Some(OLD), None));
    assert_eq!(fixture.installed(), NEW);
    assert_eq!(fixture.log(), "0.1.0 healthy\n0.2.0 healthy\n");
    assert!(!fixture.layout.staged(NEW).exists(), "staging emptied of it");
    if platform().family() == Family::Macos {
        assert_eq!(launcher_previous(&fixture), Some(OLD), "the previous bundle is kept for a rollback");
    }
}

fn launcher_previous(fixture: &Fixture) -> Option<Version> {
    let program = fixture.layout.previous_bundle().join("Contents/MacOS").join(MARKED_PROGRAM);
    fs::read(program).ok().and_then(|bytes| update::mark::find(&bytes).ok())
}

/// Requirement 13.5 and section 13: a broken version, one that crashes,
/// hangs or ends saying nothing, is rolled back, marked bad, and the
/// current one runs; it is never tried again. Only a version that hangs
/// waits out its deadline, so only it is given a short one: the others end
/// of themselves, and a deadline they never need keeps a loaded machine,
/// slow to start a program it has not run before, from being taken for a
/// hang (decision 93). What is asserted is the launcher's own state; the
/// broken version is started at least once, and the starts needed to roll
/// it back, which differ by how it fails, are bounded.
#[test]
fn a_broken_version_is_rolled_back_after_two_starts() {
    /// The launcher's starts, at most, before the test gives up on a
    /// rollback: a version that ends successfully is not retried within a
    /// start, so each of its failed starts is a start of its own.
    const MOST_STARTS: usize = 5;
    for behaviour in ["crash", "hang", "quiet"] {
        let fixture = Fixture::new(&format!("rollback-{behaviour}"));
        fixture.install(OLD, "healthy");
        fixture.stage(NEW, behaviour, true);
        let deadline = if behaviour == "hang" { SHORT_DEADLINE } else { NO_DEADLINE_NEEDED };
        for _ in 0..MOST_STARTS {
            fixture.run(deadline);
            if fixture.state().trial.is_none() {
                break;
            }
        }
        let state = fixture.state();
        assert_eq!(state.current, OLD, "{behaviour}");
        assert_eq!(state.trial, None, "{behaviour}");
        assert!(state.is_bad(NEW), "{behaviour}");
        assert_eq!(state.rolled_back, Some(NEW), "{behaviour}: said once");
        assert_eq!(fixture.installed(), OLD, "{behaviour}");
        let log = fixture.log();
        let tried = log.matches(&format!("0.2.0 {behaviour}")).count();
        assert!(tried >= 1, "{behaviour}: the broken version was started: {log}");
        assert!(log.ends_with("0.1.0 healthy\n"), "{behaviour}: then the current one: {log}");
        // The rollback is said in one start only, and the bad version is
        // never tried again.
        fixture.stage(NEW, behaviour, true);
        fixture.run(deadline);
        assert_eq!(fixture.state().rolled_back, None);
        assert_eq!(fixture.log().matches("0.2.0").count(), tried, "{behaviour}");
    }
}

/// Section 5: a version without the pinned signer's signature, and one
/// whose files are not the ones its manifest names, never run.
#[test]
fn a_version_not_signed_or_not_whole_is_refused() {
    let fixture = Fixture::new("refused");
    fixture.install(OLD, "healthy");
    fixture.stage(NEW, "healthy", false);
    fixture.run(HEALTH_DEADLINE);
    assert_eq!(fixture.state().current, OLD);
    assert!(fixture.state().is_bad(NEW));
    assert!(!fixture.log().contains("0.2.0"), "never started");

    let fixture = Fixture::new("truncated");
    fixture.install(OLD, "healthy");
    let newer = Version::new(0, 3, 0);
    fixture.stage(newer, "healthy", true);
    let broker = fixture.layout.staged(newer).join(platform().program_path(MARKED_PROGRAM));
    let bytes = fs::read(&broker).unwrap();
    fs::write(&broker, &bytes[..bytes.len() / 2]).unwrap();
    fixture.run(HEALTH_DEADLINE);
    assert_eq!(fixture.state().current, OLD);
    assert!(!fixture.log().contains("0.3.0"), "never started");
}

/// Decision 87 (finding F-02): what the launcher accepts is its own copy,
/// so a hard link the confined updater made into the staged version
/// before acceptance cannot change the version afterwards, and the
/// launcher keeps nothing under `staging/metadata` but the updater's own
/// files.
#[cfg(unix)]
#[test]
fn a_hard_link_into_staging_cannot_change_an_accepted_version() {
    let fixture = Fixture::new("hard-link");
    fixture.install(OLD, "healthy");
    fixture.stage(NEW, "healthy", true);
    let staged = fixture.staged_broker(NEW);
    let accepted = fs::read(&staged).unwrap();
    let metadata = fixture.layout.staging().join(update::layout::UPDATER_METADATA);
    fs::create_dir_all(&metadata).unwrap();
    let link = metadata.join("kept");
    fs::hard_link(&staged, &link).unwrap();
    assert_eq!(fixture.launcher(HEALTH_DEADLINE).prepare().unwrap(), Chosen::Trial(NEW));
    if link.exists() {
        fs::write(&link, b"#!/bin/sh\necho changed after acceptance\n").unwrap();
    }
    assert!(fixture.broker_of(NEW) == accepted, "the accepted version changed through a hard link");
    assert!(!link.exists(), "a file that is not the updater's own is removed from staging/metadata");
}

/// Decision 87 (finding F-02): a staged version whose folder is a link is
/// refused, and what the link names is left alone.
#[cfg(unix)]
#[test]
fn a_staged_root_that_is_a_link_is_refused() {
    let fixture = Fixture::new("root-link");
    fixture.install(OLD, "healthy");
    let elsewhere = fixture.folder.join("elsewhere").join(NEW.to_string());
    fixture.version(&elsewhere, NEW, "healthy", true);
    fs::create_dir_all(fixture.layout.staging()).unwrap();
    std::os::unix::fs::symlink(&elsewhere, fixture.layout.staged(NEW)).unwrap();
    assert_eq!(fixture.launcher(HEALTH_DEADLINE).prepare().unwrap(), Chosen::Run(OLD));
    let state = fixture.state();
    assert_eq!((state.current, state.trial), (OLD, None));
    assert_eq!(fixture.installed(), OLD);
    assert!(fs::symlink_metadata(fixture.layout.staged(NEW)).is_err(), "the link is removed");
    assert!(elsewhere.join(MANIFEST).is_file(), "what it named is left alone");
}

/// Decision 87 (finding F-02): a write descriptor the updater kept open on
/// a staged file cannot change the version once it is accepted.
#[test]
fn an_open_write_descriptor_cannot_change_an_accepted_version() {
    use std::io::Write as _;
    let fixture = Fixture::new("descriptor");
    fixture.install(OLD, "healthy");
    fixture.stage(NEW, "healthy", true);
    let staged = fixture.staged_broker(NEW);
    let accepted = fs::read(&staged).unwrap();
    let mut kept = fs::OpenOptions::new().write(true).open(&staged).unwrap();
    assert_eq!(fixture.launcher(HEALTH_DEADLINE).prepare().unwrap(), Chosen::Trial(NEW));
    kept.write_all(b"changed after acceptance").unwrap();
    kept.sync_all().unwrap();
    drop(kept);
    assert!(fixture.broker_of(NEW) == accepted, "the accepted version changed through a descriptor kept open");
}

/// A missing or unreadable state file is rebuilt from what is installed.
#[test]
fn a_lost_state_is_rebuilt_from_what_is_installed() {
    let fixture = Fixture::new("lost");
    fixture.install(OLD, "healthy");
    fs::write(fixture.layout.state(), b"not a state").unwrap();
    assert_eq!(fixture.run(HEALTH_DEADLINE), Some(0));
    assert_eq!(fixture.state(), State::installed(OLD));
}

/// The launcher's fault points in the life of an update.
fn fault_points() -> Vec<&'static str> {
    let mut points = vec!["state-written-before-rename", "state-renamed", "accept-written", "trial-counted", "commit-before-write", "commit-written"];
    match platform().family() {
        Family::Macos => points.extend(["accept-copied", "bundle-exchanged", "previous-discarded", "previous-moved"]),
        Family::Linux => points.extend(["accept-copied", "accept-moved", "launcher-written", "launcher-exchanged"]),
        Family::Windows => points.extend(["accept-copied", "accept-moved"]),
    }
    points
}

/// Not a test of its own: the launcher a fault test runs in a process of
/// its own, ended at its fault point.
#[test]
fn fault_child() {
    let Some(folder) = std::env::var_os(CHILD) else { return };
    let fixture = Fixture::at(PathBuf::from(folder));
    fixture.run(HEALTH_DEADLINE);
}

/// Section 13, "every step killed": the launcher ended at each of its
/// fault points during an update; the next start runs a complete version
/// and finishes the update.
#[cfg(debug_assertions)]
#[test]
fn the_launcher_killed_at_every_step_still_starts_and_finishes_the_update() {
    for point in fault_points() {
        let fixture = Fixture::new(&format!("fault-{point}"));
        fixture.install(OLD, "healthy");
        fixture.stage(NEW, "healthy", true);
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "fault_child", "--nocapture", "--test-threads=1"])
            .env(CHILD, &fixture.folder)
            .env(update::fault::VARIABLE, point)
            .status()
            .unwrap();
        assert_eq!(status.code(), Some(update::fault::EXIT_CODE), "{point}: the child ended at its fault point");
        let installed = fixture.installed();
        assert!(installed == OLD || installed == NEW, "{point}: a complete version is installed");
        assert_eq!(fixture.run(HEALTH_DEADLINE), Some(0), "{point}: the next start runs");
        let state = fixture.state();
        assert_eq!((state.current, state.trial), (NEW, None), "{point}: the update finished");
        assert!(!state.is_bad(NEW), "{point}");
        assert_eq!(fixture.installed(), NEW, "{point}");
    }
}

/// Section 7 on Linux: a release whose launcher differs replaces it by
/// exchanging the folders, and keeps the old one until the new one has
/// started a healthy version.
#[cfg(target_os = "linux")]
#[test]
fn a_launcher_release_exchanges_the_launcher_folder() {
    let fixture = Fixture::new("launcher-release");
    fixture.install(OLD, "healthy");
    fixture.stage(NEW, "healthy", true);
    fixture.run(HEALTH_DEADLINE);
    let launcher = fixture.layout.launcher_folder().join("neoteletext");
    assert_eq!(fs::read(&launcher).unwrap(), b"the launcher of this release");
    assert_eq!(fs::read(fixture.layout.root.join("bin.old/neoteletext")).unwrap(), b"the launcher running now");
    fixture.run(HEALTH_DEADLINE);
    assert!(!fixture.layout.root.join("bin.old").exists(), "forgotten once the new launcher started a healthy version");
}
