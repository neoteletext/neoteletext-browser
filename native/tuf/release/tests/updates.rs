//! The release tool and the updater together (`installation-and-updates.md`
//! sections 3.1, 9, 10 and 13), on the platform the tests run on: a test
//! identity; a release laid out, signed for Linux, listed, compressed and
//! published; a local HTTPS host serving the repository; the updater's
//! client checking it and staging the release whole. Then every bad
//! repository the design names is refused: a target that does not match
//! its metadata, expired metadata, metadata signed by the wrong key, an
//! older repository after a newer one (rollback), a root below threshold
//! and a root replaced by keys the old root never signed. A root rotated
//! by both old and new keys is accepted.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing, reason = "a test")]

use std::io::{BufRead as _, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

use update::platform::{BUNDLE, Family, PROGRAMS, Platform};
use update::version::Version;
use updater::{Config, Https};

const VERSION: Version = Version::new(0, 2, 0);

fn release(arguments: &[&str]) {
    let output = Command::new(env!("CARGO_BIN_EXE_neoteletext-release")).args(arguments).output().unwrap();
    assert!(output.status.success(), "neoteletext-release {arguments:?}: {}", String::from_utf8_lossy(&output.stderr));
}

fn release_fails(arguments: &[&str]) -> String {
    let output = Command::new(env!("CARGO_BIN_EXE_neoteletext-release")).args(arguments).output().unwrap();
    assert!(!output.status.success(), "neoteletext-release {arguments:?} succeeded");
    String::from_utf8_lossy(&output.stderr).into_owned()
}

/// The test's folder, empty.
fn scratch(name: &str) -> PathBuf {
    let folder = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("updates-{name}"));
    let _ = std::fs::remove_dir_all(&folder);
    std::fs::create_dir_all(&folder).unwrap();
    folder
}

fn text(path: &Path) -> String {
    path.to_str().unwrap().to_owned()
}

/// A free port on this computer.
fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
}

/// The host, ended when dropped.
struct Host(Child);

impl Drop for Host {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn serve(repository: &Path, identity: &Path, port: u16) -> Host {
    let mut child = Command::new(env!("CARGO_BIN_EXE_neoteletext-test-host"))
        .args([text(repository), text(&identity.join("host.pem")), text(&identity.join("host.key.pem")), format!("localhost:{port}")])
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut line = String::new();
    BufReader::new(child.stdout.take().unwrap()).read_line(&mut line).unwrap();
    assert!(line.starts_with("listening"), "{line}");
    Host(child)
}

/// Everything a test works with.
struct Fixture {
    folder: PathBuf,
    identity: PathBuf,
    repository: PathBuf,
    port: u16,
    _host: Host,
}

fn key(fixture: &Fixture, name: &str) -> String {
    format!("file:{}", text(&fixture.identity.join(format!("keys/{name}.pk8"))))
}

impl Fixture {
    /// A test identity, release `VERSION` published, and a host serving it.
    fn new(name: &str) -> Self {
        let folder = scratch(name);
        let identity = folder.join("identity");
        let port = free_port();
        release(&["test-identity", &text(&identity), "--host", "localhost", "--port", &port.to_string()]);
        let repository = folder.join("repository");
        let fixture =
            Self { folder: folder.clone(), identity, repository: repository.clone(), port, _host: serve(&repository, &folder.join("identity"), port) };
        fixture.publish(VERSION, &[]);
        fixture
    }

    /// Lays out, signs, lists, compresses and publishes `version`.
    fn publish(&self, version: Version, extra: &[&str]) {
        let platform = Platform::this().unwrap();
        let programs = self.folder.join(format!("programs-{version}"));
        std::fs::create_dir_all(&programs).unwrap();
        for program in PROGRAMS {
            std::fs::write(programs.join(platform.program_file(program)), format!("{program} {version}")).unwrap();
        }
        let built = self.folder.join(format!("built/{version}"));
        release(&["layout", platform.name(), &version.to_string(), &text(&programs), &text(&built)]);
        match platform.family() {
            Family::Linux => release(&["sign-linux", &text(&built), &version.to_string(), &key(self, "linux-release")]),
            Family::Macos => {
                // The bundle's seal, which codesign writes; the updater
                // checks only the manifest's hashes.
                let seal = built.join(BUNDLE).join("Contents/_CodeSignature/CodeResources");
                std::fs::create_dir_all(seal.parent().unwrap()).unwrap();
                std::fs::write(seal, "<plist/>").unwrap();
                release(&["manifest", &text(&built), platform.name(), &version.to_string()]);
            }
            Family::Windows => release(&["manifest", &text(&built), platform.name(), &version.to_string()]),
        }
        let targets = self.folder.join(format!("targets-{version}"));
        let _ = std::fs::remove_dir_all(&targets);
        release(&["targets", &text(&built), platform.name(), &text(&targets)]);
        let mut arguments = vec![
            String::from("publish"),
            text(&self.repository),
            text(&self.identity.join("root.json")),
            text(&targets),
            String::from("--targets"),
            key(self, "targets"),
            String::from("--snapshot"),
            key(self, "snapshot"),
            String::from("--timestamp"),
            key(self, "timestamp"),
        ];
        arguments.extend(extra.iter().map(|argument| (*argument).to_owned()));
        release(&arguments.iter().map(String::as_str).collect::<Vec<_>>());
    }

    /// The updater's client, with its own staging folder.
    fn client(&self, staging: &str) -> Config {
        let tls_root = std::fs::read(self.identity.join("tls-root.der")).unwrap();
        let staging = self.folder.join(staging);
        std::fs::create_dir_all(&staging).unwrap();
        Config {
            https: Https::new("localhost", self.port, Some(&tls_root), None).unwrap(),
            host: String::from("localhost"),
            port: self.port,
            root: std::fs::read(self.identity.join("root.json")).unwrap(),
            staging,
            platform: Platform::this().unwrap(),
        }
    }

    fn metadata(&self) -> PathBuf {
        self.repository.join("metadata")
    }
}

fn block<T>(future: impl Future<Output = T>) -> T {
    tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(future)
}

/// Section 3.1: the newest version is learnt, then staged whole.
#[test]
fn a_published_release_is_checked_and_staged_whole() {
    let fixture = Fixture::new("staged");
    let client = fixture.client("staging");
    assert_eq!(block(client.check()), Ok(Some(VERSION)));
    assert!(client.last_success().is_some());
    block(client.download(VERSION)).unwrap();
    let staged = client.staging.join(VERSION.to_string());
    let manifest = update::accept::complete(&staged, Platform::this().unwrap()).unwrap();
    assert_eq!(manifest.version, VERSION);
    assert!(!client.staging.join(format!("{VERSION}{}", updater::PARTIAL)).exists());
    // A second publish adds a newer version, and the check learns it.
    fixture.publish(Version::new(0, 3, 0), &[]);
    assert_eq!(block(client.check()), Ok(Some(Version::new(0, 3, 0))));
}

/// Section 13, "bad releases": a release file that is not the one the
/// targets metadata names is refused, fetched again three times, and
/// nothing is staged.
#[test]
fn a_target_that_does_not_match_its_metadata_is_refused() {
    let fixture = Fixture::new("corrupt");
    let program = Platform::this().unwrap().program_path("neoteletext-decoder");
    let name = Path::new(&program).file_name().unwrap().to_str().unwrap().to_owned();
    let mut corrupted = 0;
    for path in update::files::list_files(&fixture.repository.join("targets"), usize::MAX).unwrap() {
        // Consistent snapshots: `<hash>.<version>/<platform>/<file>.gz`.
        if path.ends_with(&format!("/{name}.gz")) {
            let file = fixture.repository.join("targets").join(&path);
            let mut bytes = std::fs::read(&file).unwrap();
            let last = bytes.len() - 1;
            bytes[last] ^= 0xff;
            std::fs::write(&file, bytes).unwrap();
            corrupted += 1;
        }
    }
    assert_eq!(corrupted, 1);
    let client = fixture.client("staging");
    let refused = block(client.download(VERSION)).unwrap_err();
    assert!(refused.contains("hash") || refused.contains("digest") || refused.contains("match"), "{refused}");
    assert!(!client.staging.join(VERSION.to_string()).exists(), "nothing staged");
}

/// Metadata past its expiry is refused (requirement 13.8): a frozen host
/// shows "Updates unavailable" after the timestamp's 7 days.
#[test]
fn expired_metadata_is_refused() {
    let fixture = Fixture::new("expired");
    fixture.publish(Version::new(0, 3, 0), &["--timestamp-days", "-1"]);
    let refused = block(fixture.client("staging").check()).unwrap_err();
    assert!(refused.to_lowercase().contains("expire"), "{refused}");
}

/// Metadata whose signature is not its role's key's is refused; the
/// release tool itself refuses to sign a role with another role's key.
#[test]
fn metadata_signed_by_the_wrong_key_is_refused() {
    let fixture = Fixture::new("wrong-key");
    let wrong = key(&fixture, "snapshot");
    let said = release_fails(&["timestamp", &text(&fixture.repository), &text(&fixture.identity.join("root.json")), "--timestamp", &wrong]);
    assert!(said.contains("signing keys"), "{said}");
    // A signature changed by one digit: no longer the timestamp key's.
    let stamp = fixture.metadata().join("timestamp.json");
    let original = std::fs::read_to_string(&stamp).unwrap();
    let at = original.find("\"sig\": \"").unwrap() + "\"sig\": \"".len();
    let digit = &original[at..=at];
    let flipped = if digit == "0" { "1" } else { "0" };
    std::fs::write(&stamp, format!("{}{flipped}{}", &original[..at], &original[at + 1..])).unwrap();
    let refused = block(fixture.client("staging").check()).unwrap_err();
    assert!(refused.contains("signature") || refused.contains("threshold") || refused.contains("verif"), "{refused}");
}

/// Versions only increase: once the client has seen a newer repository, an
/// older one served again is refused (requirement 13.8).
#[test]
fn an_older_repository_after_a_newer_one_is_refused() {
    let fixture = Fixture::new("rollback");
    let client = fixture.client("staging");
    let older = fixture.folder.join("older-metadata");
    copy(&fixture.metadata(), &older);
    // The repository's versions are its signing time in seconds.
    std::thread::sleep(std::time::Duration::from_secs(1));
    fixture.publish(Version::new(0, 3, 0), &[]);
    assert_eq!(block(client.check()), Ok(Some(Version::new(0, 3, 0))));
    std::fs::remove_dir_all(fixture.metadata()).unwrap();
    copy(&older, &fixture.metadata());
    let refused = block(client.check()).unwrap_err();
    assert!(refused.to_lowercase().contains("version") || refused.to_lowercase().contains("older") || refused.contains("rollback"), "{refused}");
}

fn copy(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        std::fs::copy(entry.path(), to.join(entry.file_name())).unwrap();
    }
}

/// The specification's section 5.3: a new root must be signed by a
/// threshold of the old root's keys and of its own. One below the old
/// threshold, and one signed by new keys alone, are refused; one signed
/// by both is accepted.
#[test]
fn a_root_is_rotated_only_by_a_threshold_of_old_and_new_keys() {
    let fixture = Fixture::new("root");
    let client = fixture.client("staging");
    assert!(block(client.check()).is_ok());
    let roles = |root: [&str; 3]| -> Vec<String> {
        let mut arguments = Vec::new();
        for key in root {
            arguments.extend([String::from("--root"), key.to_owned()]);
        }
        for role in ["targets", "snapshot", "timestamp"] {
            arguments.extend([format!("--{role}"), key(&fixture, role)]);
        }
        arguments
    };
    let new_root = fixture.metadata().join("2.root.json");
    let public = |name: &str| -> String {
        let output = Command::new(env!("CARGO_BIN_EXE_neoteletext-release")).args(["public", &key(&fixture, name)]).output().unwrap();
        format!("public:{}", String::from_utf8(output.stdout).unwrap().trim())
    };
    // Below threshold: one of the root's keys signs.
    let one = key(&fixture, "root-1");
    let (two, three) = (public("root-2"), public("root-3"));
    let mut arguments = vec![String::from("root"), text(&new_root), String::from("2")];
    arguments.extend(roles([&one, &two, &three]));
    release(&arguments.iter().map(String::as_str).collect::<Vec<_>>());
    let refused = block(client.check()).unwrap_err();
    assert!(refused.contains("root"), "{refused}");
    // Replaced: three new keys sign a root of their own.
    for name in ["root-4", "root-5", "root-6"] {
        release(&["key", &text(&fixture.identity.join(format!("keys/{name}.pk8")))]);
    }
    let (four, five, six) = (key(&fixture, "root-4"), key(&fixture, "root-5"), key(&fixture, "root-6"));
    let mut arguments = vec![String::from("root"), text(&new_root), String::from("2")];
    arguments.extend(roles([&four, &five, &six]));
    release(&arguments.iter().map(String::as_str).collect::<Vec<_>>());
    assert!(block(client.check()).is_err(), "a root the old keys never signed");
    // Rotated: the new root signed by its keys and two of the old.
    let mut arguments = vec![String::from("root"), text(&new_root), String::from("2")];
    arguments.extend(roles([&four, &five, &six]));
    arguments.extend([String::from("--old-root"), key(&fixture, "root-1"), String::from("--old-root"), key(&fixture, "root-2")]);
    release(&arguments.iter().map(String::as_str).collect::<Vec<_>>());
    assert_eq!(block(client.check()), Ok(Some(VERSION)), "rotated");
}

/// The launcher-unchanged check (section 7): a launcher that differs from
/// the last launcher release's is refused unless the release says it is
/// one.
#[test]
fn a_changed_launcher_needs_a_launcher_release() {
    let folder = scratch("launcher-check");
    let platform = Platform::this().unwrap();
    let launcher = folder.join(platform.program_path("neoteletext-launcher"));
    std::fs::create_dir_all(launcher.parent().unwrap()).unwrap();
    std::fs::write(&launcher, "this release's launcher").unwrap();
    let last = folder.join("last-launcher");
    std::fs::write(&last, "an older launcher").unwrap();
    let said = release_fails(&["check", &text(&folder), platform.name(), "--last-launcher", &text(&last)]);
    assert!(said.contains("launcher differs"), "{said}");
    let said = release_fails(&["check", &text(&folder), platform.name(), "--last-launcher", &text(&last), "--launcher-release"]);
    assert!(!said.contains("launcher differs"), "a launcher release may change it: {said}");
    std::fs::write(&last, "this release's launcher").unwrap();
    let said = release_fails(&["check", &text(&folder), platform.name(), "--last-launcher", &text(&last)]);
    assert!(!said.contains("launcher differs"), "unchanged: {said}");
}

/// The fault child's variable: the fixture's folder and port it downloads
/// from.
const CHILD: &str = "UPDATES_TEST_FAULT_CHILD";

/// Not a test of its own: a download a fault test runs in a process of
/// its own, ended at its fault point.
#[test]
fn fault_child() {
    let Some(given) = std::env::var_os(CHILD) else { return };
    let given = given.into_string().unwrap();
    let (folder, port) = given.rsplit_once('|').unwrap();
    let folder = PathBuf::from(folder);
    let identity = folder.join("identity");
    let tls_root = std::fs::read(identity.join("tls-root.der")).unwrap();
    let port: u16 = port.parse().unwrap();
    let client = Config {
        https: Https::new("localhost", port, Some(&tls_root), None).unwrap(),
        host: String::from("localhost"),
        port,
        root: std::fs::read(identity.join("root.json")).unwrap(),
        staging: folder.join("staging"),
        platform: Platform::this().unwrap(),
    };
    let _ = block(client.download(VERSION));
}

/// Section 13, "every step killed" and "writes cut short": the updater
/// ended after each file it writes, with a file written half, and before
/// and after its rename; the next download finishes the staging, and a
/// staged version is only ever whole.
#[cfg(debug_assertions)]
#[test]
fn the_updater_killed_at_every_step_still_stages_whole() {
    let fixture = Fixture::new("faults");
    let files = Platform::this().unwrap().files().into_iter().filter(|file| file.required).count();
    let mut points: Vec<String> = (0..files).map(|index| format!("download-file-written:{index}")).collect();
    points.extend(["download-truncated", "stage-before-rename", "stage-renamed"].map(String::from));
    for point in points {
        let client = fixture.client("staging");
        let _ = std::fs::remove_dir_all(&client.staging);
        std::fs::create_dir_all(&client.staging).unwrap();
        let status = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "fault_child", "--nocapture", "--test-threads=1"])
            .env(CHILD, format!("{}|{}", text(&fixture.folder), fixture.port))
            .env(update::fault::VARIABLE, &point)
            .status()
            .unwrap();
        assert_eq!(status.code(), Some(update::fault::EXIT_CODE), "{point}: ended at its fault point");
        let staged = client.staging.join(VERSION.to_string());
        if staged.exists() {
            assert!(update::accept::complete(&staged, client.platform).is_ok(), "{point}: a staged version is whole");
        }
        block(client.download(VERSION)).unwrap_or_else(|error| panic!("{point}: {error}"));
        assert!(update::accept::complete(&staged, client.platform).is_ok(), "{point}: staged whole after");
    }
}
