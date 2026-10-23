# Installation and updates on macOS, Windows and Linux

| | |
|---|---|
| **Status** | Adopted 22 October 2026 (decision 54), with the owner's answers to section 12 recorded there. Built 23 October 2026 on every desktop while the owner was away, as the owner instructed (decisions 73 to 82); what building changed is section 14, what it proved section 15. What only the owner can give (certificates, hardware keys, the update host) is in `PENDINGTASKSFORANDREW.md`. |
| **Code** | `native/update` (shared: versions, platforms, manifest, state, gzip, signatures, version mark, the updater's messages, safe writes, acceptance, profiles, fault points); `native/release-identity` (the identity built in); `native/launcher`; `native/stub` and `native/installer` (Windows); `native/os-install` (the unsafe calls); `native/tuf` (a workspace of its own: `updater`, `release`, and their fuzz targets); `confine::Role::Updater`; `core/src/updates.rs`; `scripts/updates-e2e.sh`, `scripts/updates-e2e.ps1`, `scripts/release/`. |
| **Decisions it follows** | 15 (no update can leave the application unusable), 16 (The Update Framework), 17 and 25 (`update.neoteletext.com`), 24 items 10, 11 and 13 (`tough`, one launcher scheme on every desktop, three root keys), 52 (the sandboxes). |
| **Requirements** | 13.1 to 13.13 (the earlier design's, stated in full at the tag `prototypes-final`, `prototypes/docs/requirements.md` section 13); section 11 proposes 13.14 to 13.20 and amendments. |
| **Security analysis** | [`installation-and-updates-security.md`](installation-and-updates-security.md). |

## 1. What is wanted

- **Installation** that any user can do without an administrator, on every supported version of each system, that puts the browser where the system expects an application and leaves nothing behind when removed.
- **Updates** that cannot leave the user without a working browser, that run nothing that was not verified twice by keys held apart, and that never restart the browser under the user.
- **The packaging work the sandboxes left** (`TECH-DEBT.md`): the Windows program folder readable by `ALL APPLICATION PACKAGES`, and a place for the trusted core's own confinement.

## 2. Measured, 22 October 2026

| Fact | Value | How |
|---|---|---|
| The browser's five programs, macOS, Apple silicon | 11.3 MB; gzip −9 5.4 MB; xz −9 3.1 MB | `target/release`, `tar` then `gzip -9` and `xz -9` |
| The same on Windows | 11.5 MB | the virtual machine's release build |
| SHA-256 of all five | 0.06 s of processor time | `shasum -a 256` |
| macOS binaries' minimum system | macOS 11.0, arm64 only | `otool -l`, `lipo -archs` |
| Linux binaries' newest C library symbol | `GLIBC_2.43` for the interface, 2.39 for the core, 2.34 for the others | `objdump -T` on Ubuntu 26.04 |
| Linux libraries the interface needs to start | `libasound.so.2`, `libfontconfig.so.1` (and the C library) | `readelf -d` |
| Atomic exchange of two directories | works on APFS (`renamex_np`, `RENAME_SWAP`) and ext4 (`renameat2`, `RENAME_EXCHANGE`) | a test of each, swapping two folders |
| `/Applications` | `drwxrwxr-x root:admin`: an administrator can write it without elevating | `ls -ld` |
| Code-signing identities on the owner's Mac | none | `security find-identity -v -p codesigning` |
| `tough` 0.24.0 (the decided TUF client) | an async API on `tokio`; cryptography by `aws-lc-rs`, a C library built with `cmake`; 136 crates with no default features, against the browser's 247 | `cargo info`, `cargo tree` |
| `fontique` 0.12 (fonts) | offers `fontconfig-dlopen`, loading fontconfig at run time | its `Cargo.toml` |

Three of these change the design. The Linux build on the owner's host would not start on Ubuntu 24.04 (C library 2.39) or Debian 12 (2.36), so Linux releases must be built on an old baseline. A Linux machine without ALSA's library cannot start the interface at all, though requirement 16.6 says a browser with no sound device runs silent. And `tough` is now a large, async, C-backed dependency, which section 8.2 weighs.

## 3. The model, the same on every desktop

```
 installed once, changes almost never        replaced by every update
 ┌─────────────────────────────┐            ┌───────────────────────────────┐
 │ launcher                    │  reads ──▶ │ state: current, previous,     │
 │ (macOS: the bundle's main   │            │ trial and its failed starts,  │
 │  executable; Windows: a     │            │ versions known bad            │
 │  fixed stub and two         │            └───────────────────────────────┘
 │  launcher copies; Linux:    │  starts ─▶ ┌───────────────────────────────┐
 │  bin/neoteletext)           │            │ version N: the trusted core   │
 │ checks the version's signer │            │ and its helpers, complete     │
 └─────────────────────────────┘            └───────────────────────────────┘
                                                      │ starts, confined
                                            ┌───────────────────────────────┐
                                            │ updater (a helper): downloads │
                                            │ and verifies version N+1 into │
                                            │ staging; can write nowhere else│
                                            └───────────────────────────────┘
```

**Three components, each with one job:**

- **The launcher** is what the user starts. It reads the state, chooses a version, checks that version's signer (section 5), counts a trial version's failed starts, rolls back, and starts the trusted core. It parses no network data: its only input is a state file of a few lines that it writes itself, and the version's own signed files. It is small enough to read in one sitting, and it changes only in a launcher release (section 7).
- **The updater** is a new helper process (`neoteletext-updater`), started by the trusted core and confined like the others (`confine::Role::Updater`). It may reach the update host on port 443 and write only into the staging folder. It fetches The Update Framework's metadata, verifies it, downloads the release files it names, checks their hashes and sizes, and writes them into `staging/<version>`. It never switches anything.
- **The trusted core** tells the updater when to look (at start, then every 24 hours while it runs). When the updater reports a newer version, the core offers it in the strip and, only once the user accepts, tells the updater to download and stage it. It parses none of the updater's formats: it learns only "version N is available" and "staged version N, complete" over the protocol.

### 3.1 The life of an update

1. **Check.** The updater fetches `timestamp.json`. If it names a new snapshot, the updater walks The Update Framework's client workflow (its specification, section 5: root rotation, timestamp, snapshot, targets) and learns of version N+1 for this platform and architecture.
2. **Offer, then download.** The core offers version N+1 in the strip; on the user's acceptance, each release file is a target of its own, compressed one by one (section 6). It goes into `staging/<N+1>.partial/`, written and flushed to disk, then has its size and hash checked against the targets metadata. A file that fails is deleted and fetched again, at most three times, and then the update waits for the next check.
3. **Stage.** When every file is there and checked, the updater writes the targets metadata it verified beside them, flushes, renames `<N+1>.partial` to `<N+1>`, and tells the core "staged N+1".
4. **Accept.** At the next start the launcher sees a staged version newer than `current` and not marked bad. It copies it into a folder only it writes (`versions/<N+1>.copying` on Windows and Linux, `accepting/<N+1>/` on macOS), checks the copy's files and signer (section 5), renames the copy into place (`versions/<N+1>`; on macOS the copy is what the bundle exchange takes), and writes the state with `trial = N+1, failed starts = 0`. The staged folder itself is never checked and moved: the confined updater wrote it, and could otherwise change a version after its check, through a hard link, a descriptor kept open, or a staged folder that is a link, which is refused (decision 87). Under `staging/metadata/` the launcher keeps only the updater's own files. A staged version that is refused is removed and not marked bad: "bad" means only a trial that failed (step 6), so a folder the updater made cannot stop that version being accepted later (finding F-08). One start examines at most three staged versions, newest first; the next start goes on. The state is written to a new file, flushed, then renamed over the old one, which is the atomic switch of requirement 13.4.
5. **Trial.** Before starting a trial version the launcher counts the start as one that has not started yet, and flushes the state. The broker says `started` just before it starts the trusted core: the launcher then clears that count and marks the trial running (decision 93). The new version's trusted core, once healthy (section 3.2), asks the launcher's state to *commit*: `previous = old current, current = N+1, trial = none`. A start that said `started` and then ended without being healthy, or missed its health deadline, is a failed start; one that said `started` and ended successfully before its health was known (the user quit) is not counted; a trial still marked running at the next start (the launcher itself ended) counts as failed.
6. **Roll back.** If a trial version reaches two failed starts, or three starts that never said `started`, without committing, the launcher writes `trial = none`, adds N+1 to the bad versions (the updater never stages it again), and starts `current`, which was never touched. The strip says so once.
7. **Clean.** After a commit, versions older than `previous` are deleted, and staging is emptied. A file still in use, which Windows refuses to delete, is left for next time.

Every step leaves the state naming a complete version: the old one until step 5's commit, the new one after. Power lost at any point is the same as the process being killed at that point, which is what requirement 13.12's tests do.

### 3.2 The health check

A trial version is healthy when, within 30 seconds of its start, all of these have happened:

- The interface has drawn its first frame.
- Every helper started has reported its confinement, every layer present (`confine::Report`; a process missing one does not start, decision 58).
- The settings were read (or found absent).

The core then commits. A version that crashes, hangs or fails to confine its helpers never commits, so it is rolled back after two starts.

**The launcher's deadlines** (decision 93, amending decision 80's 45 seconds from the broker's start): the broker writes `started` before it starts the trusted core, and the launcher waits for it up to a start deadline of 5 minutes, for a first run's checks by the system (macOS's assessment of a new program, an antivirus scan on Windows). From `started` it waits up to a health deadline of 47 seconds for `healthy`: the core's 30-second window and 2 seconds' settling, defined once in `update::layout` for both, and 15 seconds for the broker to start the core. `healthy` says `started` too. After every broker's end, the launcher also ends whatever is left in its process group on macOS and Linux, so a broker that crashed leaves no helper running (finding F-34; confine gives each Linux helper a parent-death signal as well), and it notices the broker's end even while a helper still holds its output open. A trial that misses its start deadline is ended and not counted as a failed start; on Windows and Linux the current version runs for that start, and on macOS, whose installed bundle is the trial's, the start ends. A slow computer therefore no longer rolls back a good version. The check is deliberately what the user would notice: a window that draws, with sandboxes that hold.

A decoder or fetcher that ends before it first speaks, unasked, fails the trial even when its pipe closed cleanly, as one that cannot confine itself does; one that ends after speaking is not counted, so a hostile server that crashes it cannot force a rollback (finding F-12; `a_decoder_that_ends_before_it_speaks_fails_the_version`, `a_fetcher_that_ends_before_it_speaks_fails_the_version`).

### 3.3 The user's data

The profile is `settings.txt`, which carries its format version on its first line (`neoteletext-settings 1`, requirement 19.2). Requirement 13.6 becomes concrete:

- A version that writes a new format writes `settings-<format>.txt`, first copying from the newest older format it finds.
- The old file is never written by the new version.
- A rolled-back version finds its own format's file as it left it.
- The change in the user's settings made under the newer version is lost on rollback. That is the price of never migrating in place, and the strip says so when it happens.

### 3.4 What the user sees

- **Updates are suggested, never imposed** (the owner's answer to section 12, item 9). The updater checks at start and every 24 hours. When a newer version exists the strip offers it once ("NeoTeletext 0.3 is available: Install"), and the settings panel keeps offering it. Nothing is downloaded or staged until the user accepts. It then starts at the next start, and the browser is never restarted under the user.
- **A setting, "Check for updates", on by default.** "Off" stops the updater from starting at all. The settings panel then shows "Not checking for updates", and expired metadata is never seen.
- **Expired or unreachable metadata** shows "Updates unavailable" in the settings panel after 7 days without a successful check (section 9.3), never as a dialog.
- **The version and its state** (current, previous, "rolled back from 0.3") are on the start page's help pages and in the settings panel.

## 4. Per platform

### 4.1 macOS

**Layout: one bundle, swapped whole.**

| Path | What |
|---|---|
| `/Applications/NeoTeletext.app` (or `~/Applications/` for a user who is not an administrator) | The running version, a complete signed and notarised bundle. Its main executable is the launcher; the trusted core and every helper are in `Contents/MacOS/`. |
| `~/Library/Application Support/NeoTeletext/` | `state`, `staging/`, `accepting/`, `previous/NeoTeletext.app`, the settings |

The switch is `renamex_np(accepted, installed, RENAME_SWAP)`, which exchanges the two bundles atomically (measured on APFS): `accepted` is the launcher's own copy of the staged bundle in `accepting/<version>/`, checked there (section 3.1, step 4; decision 87), and the old bundle moves to its path and is then moved under `previous/`, from the trial's own accepted folder alone. Before either exchange, a rollback's included, the bundle about to run is checked again by the pinned signer; one refused is not exchanged in, and nothing runs (finding F-19). **Why one bundle rather than versioned bundles started by a launcher bundle** (the earlier design's arrangement): the system keeps the Dock, Launchpad, Spotlight, login items and privacy permissions by the bundle's path and identity. With versioned bundles, the running app would be the versioned bundle, so a Dock icon pinned to it would point at a version that the next update deletes. One bundle swapped whole keeps all of that. The swap needs the bundle's folder to be writable by the user, which `/Applications` is for an administrator (measured), and `~/Applications` always is.

**The launcher inside the bundle.** Because the launcher is swapped along with the version, a broken new launcher would be its own judge. So the release build checks that the launcher's executable is byte-identical to the last launcher release, unless this release is one. A trial version therefore always runs the same reviewed launcher code that can roll it back. The previous bundle is complete, so rolling back is the same swap the other way.

**Signing and first install.**

- A Developer ID Application certificate, with the hardened runtime on every executable (notarisation requires it).
- The bundle submitted with `notarytool`, and the ticket stapled to the bundle.
- First install is a notarised and stapled disk image: drag to Applications.
- macOS 15 removed the Control-click way past Gatekeeper, so an unnotarised build cannot be opened by an ordinary user.
- **Translocation:** a bundle opened from the disk image, or from Downloads with its quarantine attribute, is run from a random read-only path. The launcher detects this (its own path is not writable) and offers to move itself to Applications, doing nothing else until it has.

**What the updater writes** is a complete bundle with its stapled ticket. Gatekeeper accepts it whether or not the files carry a quarantine attribute, and the launcher's own check (section 5) does not depend on the attribute.

**Architectures and systems.**

- Today's builds are arm64 only, with a minimum of macOS 11 (measured).
- The disk image is universal, with arm64 and x86-64 in one bundle.
- Updates are per architecture: two sets of targets, each half the size.
- The minimum system stays 11, Rust's floor, unless a dependency raises it.

**The trusted core's own sandbox** (requirement 3.8) is not settled by packaging alone. Under the App Sandbox a child process inherits its parent's sandbox (`com.apple.security.inherit`) and may not narrow it, and macOS refuses a nested `sandbox-exec` (measured, 21 October 2026). So confining the core with the App Sandbox would widen every helper to the core's sandbox. That is weaker than today's per-helper profiles. Narrower helpers under the App Sandbox means XPC services, each with its own entitlements, started by `launchd`, with the protocol's pipes passed over XPC as descriptors. That is a design of its own (section 12, decision 6). The first release keeps today's arrangement: the helpers confined by `sandbox-exec`, the core unconfined.

### 4.2 Windows

**Layout: a fixed stub, two launcher copies, versioned folders, all per user.**

| Path | What |
|---|---|
| `%LOCALAPPDATA%\Programs\NeoTeletext\NeoTeletext.exe` | The stub, under 200 KB: it reads `launcher` (one byte: `a` or `b`) and starts `launcher-a.exe` or `launcher-b.exe`. It is never replaced, and Start, the taskbar and the uninstaller all point at it. |
| `…\launcher-a.exe`, `…\launcher-b.exe` | Two launcher copies. A launcher release writes the other copy and switches the byte (requirement 13.7). |
| `…\versions\<version>\` | Each version's programs, side by side. A running program cannot be overwritten on Windows, so a new version never has to be. |
| `…\state`, `…\staging\` | As section 3 |
| `%APPDATA%\NeoTeletext\` | The settings |

The state file is replaced with `std::fs::rename`, which on Windows 10 1607 and later renames with replace in one call on NTFS. Flushing before it is `File::sync_all`, which is `FlushFileBuffers`. No unsafe code is needed for the switch.

**Per user, no administrator,** as Chrome's, Visual Studio Code's and Squirrel's per-user installs are. A machine-wide install under Program Files would need an elevated service to update. That would be a SYSTEM process exposed to the update path: the most powerful process on the machine, doing the most dangerous thing.

**The installer** (`NeoTeletext-Setup.exe`, our own small Rust program, signed) does this:

- Copies the stub, one launcher and the first version.
- Grants `ALL APPLICATION PACKAGES` read and execute on `versions\` (`icacls`-equivalent through the security calls `confine` already makes), which closes `TECH-DEBT.md`'s item. Without that grant the decoder and fetcher run without their AppContainer.
- Creates the decoder's and fetcher's AppContainer profiles.
- Writes the Start menu shortcut (a Shell `IShellLinkW` call, unsafe, in a crate allowed it).
- Registers itself under `HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall\NeoTeletext`, so Settings › Apps lists and removes it.

**Not MSI:** Windows Installer keeps its own record of the installed files, so its repair after an update puts the old files back. **Not MSIX:** it installs and updates by itself, trusting only the package signature, which decision 24 refuses.

**Signing:**

- Authenticode on every executable, and the installer.
- Code-signing keys must sit in a hardware module since June 2023 (the CA/Browser Forum's rule), so the key is a hardware token or a cloud signing service.
- SmartScreen builds a reputation per file and certificate. An extended-validation certificate no longer skips it (Microsoft's change of 2024), so the first releases will be warned about until they have been downloaded enough.

**Systems.**

- Windows 10 version 1703 or later: the less privileged AppContainer needs 1703, and `std::fs::rename`'s replace semantics need 1607.
- Windows 11.
- x86-64 first, arm64 when there is a machine to test on.

### 4.3 Linux

**Layout: in the user's data folder, no root.**

| Path | What |
|---|---|
| `~/.local/share/neoteletext/bin/neoteletext` | The launcher |
| `~/.local/share/neoteletext/versions/<version>/` | Each version's programs |
| `~/.local/share/neoteletext/state`, `staging/` | As section 3 |
| `~/.local/bin/neoteletext` | A symbolic link to the launcher, on the path in most distributions |
| `~/.local/share/applications/neoteletext.desktop`, `~/.local/share/icons/hicolor/…` | The menu entry and its icon |
| `~/.config/neoteletext/` | The settings (`XDG_CONFIG_HOME` honoured) |

A launcher release replaces `bin/` by `renameat2(…, RENAME_EXCHANGE)`, measured on ext4. Btrfs, XFS and tmpfs support it too. On a file system that refuses it (the call says `EINVAL`), the launcher keeps the old copy and the release waits.

**First install** is a signed tarball containing the launcher, run once as `neoteletext --install`: it copies itself and the first version into place and writes the menu entry. There is no `curl | sh`: the tarball is verified first, as the download page says, by its detached signature (section 5).

- **Not AppImage:** it needs FUSE 2, which Ubuntu stopped installing in 22.04.
- **Not Flatpak:** it would be the update path, against decision 24.
- **Not a distribution's packages:** they would be another update path, through the distribution's own signatures.

**A build that starts on every distribution.** Measured: a build on the owner's Ubuntu 26.04 needs the C library 2.43. So releases are built in a container with C library 2.28 (AlmaLinux 8, with `podman` on `foundintelquiet4060`). Those binaries run on every distribution since 2018: RHEL 8, Debian 10, Ubuntu 18.10 and later.

- **fontconfig** is loaded at run time (`fontique`'s `fontconfig-dlopen`), so a machine without it still starts.
- **ALSA's library** is linked by `cpal` and cannot be loaded later, so it is linked only by the sound helper, `neoteletext-sound` (decision 55). The interface starts without it, and where it is missing the sound helper fails to start and the browser runs silent (requirement 16.6).

**Signing:** Linux has no platform signature. Each release file carries a detached Ed25519 signature by the release key (section 5), which the launcher checks with `ring`, already in the browser.

**Systems.** Any x86-64 distribution with the C library 2.28 or later and a display (X11 or Wayland). arm64 when there is a machine to build and test on. Every sandbox layer is required (decision 58): Linux 6.10 or later with Landlock, and unprivileged user namespaces allowed for the browser, which a default Ubuntu from 24.04 allows only through an AppArmor profile installed with root; how a per-user install meets that is open (`sandboxing.md` section 8.3).

## 5. Two signatures, by keys held apart

A version runs only if two independent checks pass:

1. **The Update Framework, in the updater.** It covers freshness, rollback protection, key rotation, and a network attacker or a compromised host. Its keys are the root (three hardware keys, threshold two), the targets key (offline), and the snapshot and timestamp keys (section 9).
2. **A pinned signer, in the launcher**, at acceptance, before a version is first started:
   - **macOS:** the bundle's code signature is valid, and its designated requirement names the project's Team ID, which is built into the launcher.
   - **Windows:** the manifest's detached Ed25519 signature (`manifest.sig`) verifies against the release key built into the launcher, checked first and by the code Linux uses, so programs each signed for a different release cannot be put together under a new version's number (decision 92); then every executable's Authenticode signature is valid (`WinVerifyTrust`), and the signer's certificate is the project's, pinned by its public key.
   - **Linux:** every file's detached Ed25519 signature verifies against the release public key built into the launcher.

A compromised updater, which is the process that parses network data, cannot get a version run: it holds no platform signing key. A stolen platform signing key cannot ship an update either: the updater stages nothing the targets metadata does not name. Shipping malware needs both, from two places. The launcher also checks that the version it accepts is newer than `current` and not marked bad. On macOS and Windows that version is read from inside the signed files; on Linux it is in the signed manifest. The only rollback is the launcher's own, to `previous`.

Both checks are of the launcher's own copy of the staged version, made where only the launcher writes, so what was checked is what runs (decision 87). The platform signature is checked once, at acceptance, not at every start. On macOS the kernel kills a signed program whose pages no longer match its signature. A corrupt file anywhere then shows as a failed start, which the trial counts like any other.

## 6. The release files

**No archive format.** Each program, and on macOS each file of the bundle, is a target of its own: `<version>/<platform>-<arch>/<path>.gz`. The updater decompresses with `miniz_oxide`, already in the browser through the PNG reader. That leaves no archive reader to fuzz and no path inside an archive to escape with. A target's path is checked against the platform's list of the files a version has, and anything else is refused.

**Full files, no deltas.** A whole release is 5.4 MB gzip-compressed (3.1 MB with xz, which would need a new crate): less than the browser's own channels fetch in a few minutes. Delta updates would add a patch format, and a reader for it, to the most dangerous process, to save under 4 MB a release.

## 7. Updating the launcher

The launcher is part of every release but changes only in a **launcher release**, which the release notes call out. That release also passes the release build's check that the launcher is unchanged against the last launcher release; section 4.1 explains why that matters on macOS.

- **macOS:** the launcher is inside the bundle, so it is swapped with the version.
- **Windows:** the running launcher writes the other copy (`launcher-b.exe` if it is `a`), flushes, checks its signature, and switches the one-byte `launcher` file.
- **Linux:** the new launcher is written beside `bin/`, then the two folders are exchanged.

In each case the old launcher stays until the new one has started a version that committed.

## 8. The updater's confinement and its client

### 8.1 Confinement

`confine::Role::Updater` would be a fourth role in decision 52's crate:

| | macOS (`sandbox-exec`) | Linux | Windows |
|---|---|---|---|
| Network | outbound TCP to port 443 only | Landlock: TCP connect to port 443 only (ABI 4); seccomp: the internet families only | AppContainer with `internetClient` |
| Files | read and write `staging/` only, and the resolver's files | Landlock: `staging/` read and write; the resolver's paths read | AppContainer: `staging\` granted to its SID by the installer; nothing else |
| Programs | none | none (seccomp) | none (the child-process policy) |

`sandbox-exec` cannot name a host, only addresses and ports. The host is pinned instead by TLS: the updater accepts only `update.neoteletext.com`'s certificate, through `rustls` with Mozilla's roots. It also needs no secret of the user's: it receives nothing but the channel of its own pipe.

### 8.2 The Update Framework client

Decision 24 named `tough`. It has since become async and C-backed (section 2). The choices:

| | Cost | Risk |
|---|---|---|
| **`tough` 0.24, in the confined updater** (recommended) | 136 crates, `tokio` with one thread (`block_on`), `aws-lc-rs` built with `cmake` (and `nasm` on Windows, or its prebuilt objects), `serde` | Maintained by Amazon for Bottlerocket's updates; used in production. Its weight sits in one confined process that holds no platform key (section 5), so a fault in it alone ships nothing. |
| Our own client: the specification's client workflow for one repository with no delegations, with `ring` and the engine's bounded JSON reader | Perhaps 1,500 lines of the most security-sensitive code in the project, plus a canonical JSON writer | Ours to get right. The TUF project's conformance suite (`tuf-conformance`) would test it against the specification's attacks. |
| `tuf` (rust-tuf) | A pre-release (0.3.0 beta) | No stable release; its activity is to be checked before it is considered |

The recommendation keeps decision 24. The second check of section 5 is what makes its weight acceptable.

**The root's floor** (decision 91): `tough` verifies the chain of roots from the root built into the updater at every check, so a host that stopped serving the newer root files would take the updater back to keys a rotation retired. The updater therefore keeps the highest root version it has reached in `staging/metadata/root-version`, refuses a root below it ("the update host offered an older root"), and raises it after each check. The built-in root stays the trust anchor: the floor only refuses going back. Requirement 13.8's protection against rollback thus covers the root's versions as well as the other roles'.

**The clock** (finding F-49): `tough` keeps the latest time it has seen (`staging/metadata/latest_known_time.json`) and refuses to go on when the clock is behind it, so a clock that once jumped forward and was put right left "Updates unavailable" for good. The updater now says that case in its own words ("this computer's clock … is behind the time of an earlier check"), and forgets a recorded time more than a day ahead of the clock, which a wrong clock made, and checks again.

## 9. The repository

### 9.1 Roles and keys

| Role | Signs | Expires | Key |
|---|---|---|---|
| Root | the keys of every role | 1 year | three hardware keys held by three people, threshold two (requirement 13.9) |
| Targets | each release file's length and hashes, and the per-platform release manifests | 3 months | a hardware key, offline, used only at a release |
| Snapshot | the version of the targets metadata | 3 months, re-signed with targets | with targets |
| Timestamp | the latest snapshot | 7 days | online, on the machine that re-signs it daily |

Each platform's own signing key (Apple's Developer ID, the Windows certificate's hardware module, the Linux release key) is separate from all four. On Linux the release key is a fifth hardware key, offline.

### 9.2 Hosting

`update.neoteletext.com` serves static files only, over TLS (decisions 17 and 25): the four metadata files, each consistent-snapshot version of them, and the targets. It is not trusted. The worst it can do is refuse service or serve old metadata, which expiry turns into "Updates unavailable" after 7 days.

### 9.3 The timestamp's 7 days

The timestamp is re-signed every day by a scheduled job on a machine the owner controls, with the timestamp key alone. It can say "nothing new"; it cannot name a release. Seven days is how long an attacker holding the host could keep users from learning of a release before they see "Updates unavailable". A shorter expiry narrows that window. The cost is that one missed run of the job shows every user the warning.

## 10. The release procedure

A script per step, run by the owner, as the gates are (no continuous integration):

1. **Build at the release commit**, after the full gate:
   - macOS on the owner's Mac, both architectures, and `lipo` for the disk image.
   - Windows in the virtual machine.
   - Linux in the C library 2.28 container on `foundintelquiet4060`.
2. **Platform-sign:**
   - macOS: `codesign` with the hardened runtime, `notarytool submit --wait`, `stapler staple`.
   - Windows: `signtool` with the hardware module, then the release key signs the manifest (`neoteletext-release sign-windows`, decision 92).
   - Linux: the Ed25519 release key signs each file and the manifest.
3. **Check what the launcher will check:** run each platform's acceptance check (section 5) on the signed files, and the launcher-unchanged check (section 7). The check also reads the identity the launcher was built with, from its identity mark (`release_identity::IDENTITY_MARK`), and refuses a test identity unless `--test-identity` says this is a test; every release script refuses to run while `NEOTELETEXT_RELEASE_IDENTITY` is set unless its first argument is `--test` (`-Test` on Windows), an empty variable is no identity, `sign-linux`, `sign-windows`, `publish.sh` and `timestamp.sh` refuse a key in a file (`file:`) for a production release, and `publish.sh` runs the check on every platform's version first (finding F-24).
4. **Sign the release:** the targets key adds the files (gzip-compressed) and the manifests to the targets metadata, then snapshot and timestamp are signed. The keys are plugged in for this step only.
5. **Upload:** targets first, then the snapshot, then the timestamp, so a client never sees metadata naming files that are not there yet.
6. **Record:** the release's version, hashes and the signing step's log, in `docs/releases.md`.

## 11. Proposed requirements

13.1 to 13.13 stand. The amendments:

- **13.7:** on macOS the launcher is inside the bundle, swapped with it, and byte-identical across releases except launcher releases (section 4.1).
- **13.8:** the client stays `tough` (section 8.2), in the confined updater.
- **13.10:** "the launcher scheme" is section 3's model on every desktop.

New:

- **13.14 Two independent signatures.** A version runs only after the updater verified it against The Update Framework and the launcher verified its pinned signer (section 5).
- **13.15 Installation without an administrator,** in the user's own folders, on every desktop: per user on Windows and Linux; in `/Applications` for an administrator on macOS, else `~/Applications`.
- **13.16 Supported systems:** macOS 11 and later; Windows 10 version 1703 and later; Linux with the C library 2.28 and later. Releases are built to start on the oldest.
- **13.17 No forced restart:** an update starts at the browser's next start, announced once in the strip.
- **13.18 The updater is confined** (section 8.1): the update host's port, the staging folder, no programs.
- **13.19 The health check** of section 3.2 decides commit and rollback.
- **13.20 Uninstall** removes the program and the menu entries, and asks before removing the settings and bookmarks.

## 12. The owner's answers (22 October 2026)

1. **Adopted** as decision 54, with requirements 13.7, 13.8 and 13.10 amended and 13.14 to 13.20 added.
2. **`tough`** kept, in the confined updater.
3. **macOS: one bundle swapped whole**, proved first by a spike on macOS 11 and 15.
4. **ALSA on Linux:** sound moved into a helper process of its own (decision 55), so the interface starts without the library and the browser runs silent where it is missing.
5. **Architectures** as recommended: universal on macOS, x86-64 first elsewhere.
6. **The trusted core confined now, properly** (decision 56): not the App Sandbox with XPC services, which cannot give each channel its own decoder without private API; instead a small broker that starts the core confined and every helper for it, on all three platforms.
7. **Accounts, keys and machines:** not yet; listed in `PENDINGTASKSFORANDREW.md`.
8. **The timestamp expires after 7 days.**
9. **Updates are suggested to the user and installed only when accepted** (section 3.4).

## 13. Proving it

Requirement 13.12's tests, made concrete, run on every desktop in the full gate (Windows and Linux on their hosts):

- **Every step killed:** a test build of the updater and the launcher takes `NEOTELETEXT_FAULT=<step>` and exits at that step, for every step of section 3.1 and between every file write. After each, the launcher starts a complete version, and the next run finishes the update.
- **Writes cut short:** a fault point truncates a staged file. The updater refuses it, deletes it and fetches it again.
- **Bad releases:** every negative case of the conformance suite's repositories, served by a local test host: corrupt, truncated, wrongly signed, expired, older, below threshold, replaced root, mismatched length or hash. Then the second check alone: a targets-valid release with no platform signature or the wrong signer, which the launcher refuses.
- **A full disk:** a small RAM disk (`hdiutil` on macOS, `tmpfs` on Linux, a small virtual disk on Windows). The update fails cleanly and the current version runs.
- **A broken version** that crashes at start, one that hangs, and one whose helpers do not confine: each is rolled back after two starts and marked bad.
- **The profile:** migrate to `settings-2.txt`, roll back, and find `settings.txt` as it was.
- **A launcher release,** killed at every step, on each platform.

## 14. What building changed (23 October 2026)

Built while the owner was away, as the owner instructed ("do everything, do not stop, make the decisions you need"). Every departure below is in decisions 73 to 82 (`architecture.md` section 2) and a row of `IMPLEMENTATION-LOG.md`'s section "Installer and updater", for the owner to confirm.

1. **The updater and the release tool are a workspace of their own, `native/tuf`.** `tough` depends on `rustls` with its default features, which turn on the `aws-lc-rs` provider; in the browser's workspace Cargo would unify that into every crate's `rustls`, and `ClientConfig::builder()` (the fetcher's, and the engine's qrkeyboard client's) panics when two providers are built in. The workspace has its own lockfile, `deny.toml` and fuzz targets, and the gates run it beside the browser's. `tough` built on all three desktops with no `cmake` or `nasm`: `aws-lc-sys` 0.46 builds with the C compiler alone.
2. **The release's identity is built in** (`native/release-identity`): the update host and port, a test host's certificate root, The Update Framework's first root, the Linux release key, the macOS designated requirement and the Windows signer's key. They are read at build time from `production/`, or from the folder `NEOTELETEXT_RELEASE_IDENTITY` names for a test build; no program reads them at run time, so nothing on a user's machine changes what a build trusts (decision 58). Until the key ceremony the production identity names only `update.neoteletext.com:443`: a production build checks nothing, stages nothing and accepts nothing, and says "This build carries no update keys".
3. **macOS keeps the update files apart from the settings:** `~/Library/Application Support/NeoTeletext Updates/` holds `state`, `staging/` and `previous/`, since the confined trusted core may write its settings folder (`sandboxing.md` section 7.3) and must not write the state.
4. **The launcher stays the broker's parent.** It starts the broker with `NEOTELETEXT_LAUNCHED=1` and reads its standard output: once the trusted core has found the version healthy it asks the broker, which writes `healthy`, and the launcher commits. Only the launcher writes the state. A trial that ends badly is counted and the choice made again in the same start, so a broken version is rolled back without the user starting the browser three times; one that ends successfully (the user quit before its health was known) is counted and not retried. A trial that says nothing in 45 seconds (section 3.2's 30 and 15 for the core to start) is ended with its process group (since 24 October 2026 a start deadline, then a health deadline from the broker's `started`: section 3.2, decision 93) on macOS and Linux, and on Windows with the broker, whose death ends every helper's job. An exclusive lock on `state.lock` guards every change; a second start shares it and runs the version the first chose. An unreadable state is rebuilt from what is installed (the bundle's mark on macOS, the newest complete version elsewhere). Since 24 October 2026 (finding F-80) a state too long to be one is rebuilt likewise, and so is one whose current version is not installed; on Windows and Linux each installed version is accepted again by the pinned signer, and the newest becomes the trial with the next newer one current, since nothing says the newest was ever found healthy.
5. **The health check, as built:** the interface says once that its first frame reached the screen (`ToCore::Drawn`), the settings were read or found absent, no decoder ended with an error and no decoder or fetcher ended before it first spoke (finding F-12), all within 30 seconds, and 2 more seconds have passed for a helper that cannot confine itself to end. A window that draws is what the user would notice; a page of a channel is not required, since a restored web channel may be offline.
6. **The version a launcher accepts is read from a mark in the broker's bytes** (`update::mark`) on macOS and Windows, inside what the platform signed, not from `Info.plist` or a version resource: one bounded search, the same on both, and no property-list or resource reader.
7. **Each release has a manifest** (`update::manifest`): its version, platform, and every file with its length and SHA-256, itself a target of The Update Framework. The updater stages only what it names and checks each file once decompressed; the launcher accepts a folder only when it holds exactly those files. On Linux the manifest and every file carry the release key's detached signature; the version is the signed manifest's. The updater keeps The Update Framework's trusted metadata in `staging/metadata/` (its datastore, which stops the host rolling the repository back) and does not write the targets metadata beside a staged version: the manifest is what the launcher reads.
8. **Updates in the browser:** "Check for updates" is the line `updates on|off` of `settings.txt`, on by default; the settings panel shows the version, the trusted core's line about updates and, when there is one, "Install <version>". The strip says the offer once, as a notice ("NeoTeletext 0.3 is available: Install it in Settings"), rather than with a button of its own: the strip's address field stays the channel's. The updater goes through the user's proxy when one is set, and its sandbox's one port is then the proxy's.
9. **Removal:** on Linux and macOS `--uninstall` on the terminal, asking before the settings go; on macOS a bundle moved to the Bin is removed the system's way, which leaves the settings, as every macOS application's are. On Windows Settings › Apps runs `NeoTeletext.exe --uninstall`, which asks with message boxes and hands the removal to a copy of the launcher in the temporary folder, since a running program cannot remove itself; that copy stays there until Windows cleans the folder. The uninstaller also removes the helpers' `AppContainer` profiles.
10. **Windows:** the installer grants `ALL APPLICATION PACKAGES` read and execute on `versions\`, with no administrator (the user owns the folder), and the launcher copies each version it accepts into `versions\` and checks the copy, rather than renaming it, since what the updater wrote carries its container's full control and the low integrity label an `AppContainer` gives its files; the containers' profiles are made at each helper's first start, as before. The Start menu shortcut is written as the file it is, by the Shell Link Binary File Format, with no COM call. A fetcher's loopback exemption (`CheckNetIsolation LoopbackExempt`) needs an administrator, so a per-user installation cannot give it: a channel on this computer needs the owner's one command, which `docs/cookies.md` section 15 item 11 records. There is no icon of the browser's own on any desktop yet: the art is the owner's.
11. **Linux and AppArmor** (`sandboxing.md` section 8.3, the owner's question): every Linux version carries `neoteletext.apparmor`, which lets the browser's programs where a per-user installation keeps them make the user namespace their sandbox needs, and changes nothing else; `neoteletext --install` says the two administrator commands that install it where the restriction is on. Since 24 October 2026 (decision 95, finding F-23) the profile names only the four programs that make namespaces (`neoteletext`, `neoteletext-decoder`, `neoteletext-interface`, `neoteletext-sound`), not a pattern for every program, and `--install` writes it to `<root>/neoteletext.apparmor` with the installation's real root, so an installation under `XDG_DATA_HOME` or reached through a link is named as it is, and prints it with one sentence saying what it allows; the release's own copy names the default root. Verified on the Linux host with the restriction on: a program at one of the four names made a user namespace, one at `neoteletext-fetcher` was refused. The browser still refuses to run without its namespace. No system-wide package: it would need root and a package per distribution, a second way into the system beside decision 24's one update path, for a file an administrator installs once.
12. **Keys at signing:** the release tool names a key by its source, a PKCS #8 file (the test keys) or a program that signs (`command:<public key>:<program>`, a PKCS #11 or OpenPGP card tool for the hardware keys), or by its public half alone (`public:`) for a root key whose holder is away. The same steps sign with the test keys now and the hardware keys later.
13. **The conformance suite** (`tuf-conformance`) is a Python program and was not run (no interpreter in the tree, requirement 1.3, and no download beyond crates). Its negative cases are tests of the browser's own instead (section 15).
14. **Fault points** exist in debug builds only; a release build reads no `NEOTELETEXT_FAULT`.
15. **A Windows launcher release** writes the other copy and switches the byte; a new launcher that cannot start a version is not switched back by the stub, which stays as small as it is. The old copy stays in the other slot, so the byte can be switched back by hand.
16. **Linux release builds on the C library 2.28** are a script (`scripts/release/build-linux.sh`) that has not been run: it needs an AlmaLinux 8 image, which would have to be downloaded. Since 24 October 2026 (finding F-73) everything it takes is pinned: the image by its digest, each package by its version, rustup-init by its version and SHA-256 (no `curl | sh`), and the engine by its commit from a copy of its history named by `NEOTELETEXT_ENGINE_GIT`; it, and `build-windows.ps1`, refuse a tree with changes.

17. **The stub is 233,864 bytes**, not a few kilobytes: Rust's standard library, even at `opt-level = "z"`. Its code reads one byte and starts one program; a smaller one would need unsafe Win32 calls without the standard library.
18. **A Windows version is copied into `versions\`, not renamed** (item 10 above, and `installation-and-updates-security.md` section 4a): what the updater wrote carries its container's full control and the low integrity label; the copy takes `versions\`'s access list and is checked again before it becomes the trial. Since 24 October 2026 every desktop accepts a copy (decision 87, section 3.1 step 4), and `neoteletext --install` on Linux copies the tarball's version first and accepts the copy.
19. **The Start menu shortcut carries the target as the shell's item list** as well as the link information: Windows resolves a link with only the latter to nothing. The item list names each folder in ASCII, a folder with other characters as `?`.
20. **The macOS end-to-end test signs ad hoc** with an identifier as its pinned requirement, since a self-signed certificate would need the login keychain's trust; the launcher's check is the same `codesign --verify --deep --strict -R=` a Developer ID requirement will use.

## 15. What building proved (23 October 2026)

Section 13's proofs, run on 23 October 2026 with the real programs, test keys made under `tmp/` (ignored), and a local HTTPS test host serving a repository the release tool published. Nothing was published and no external service was contacted. Requirement section 13's State lines cite this section.

**Tests on every desktop** (macOS arm64, Linux x86-64, Windows x86-64): the launcher's (`native/launcher/tests/launcher.rs`: commit, rollback of a version that crashes, hangs or ends quietly, refusal of one unsigned or not whole, a lost state rebuilt, and the launcher killed at every fault point of its platform), the updater's and the release tool's against the test host (`native/tuf/release/tests/updates.rs`: staged whole, a target not matching its metadata, expired metadata, the wrong key, an older repository after a newer one, a root rotated only by a threshold of old and new keys, a changed launcher needing a launcher release, and the updater killed at every step), and the confined updater's probe (`the_updater_reaches_one_port_and_writes_only_its_staging_folder`, Linux). The fuzz targets of every new reader ran clean (`native/fuzz`, `native/tuf/fuzz`).

**End to end** (`scripts/updates-e2e.sh`, `scripts/updates-e2e.ps1`), each in a home or user folder of the test's own, with a window opened for every start:

| Step | macOS 15.6, arm64 | Linux: Ubuntu 26.04, kernel 7.0, under Xvfb | Windows 11 (build 26200) in the virtual machine |
|---|---|---|---|
| First install | The bundle copied to an Applications folder, its signature verified against the pinned requirement; the disk image made | The signed tarball checked by its release key's signature, then `neoteletext --install`: on the path and in the menu | `NeoTeletext-Setup.exe` (Authenticode-signed with a test certificate, the stub and launcher in it) in a limited desktop session: the programs, the grant to `ALL APPLICATION PACKAGES`, the state, the Start menu shortcut (resolved by `WScript.Shell` to the stub) and the uninstall entry |
| A healthy first start | Yes | Yes | Yes |
| An update checked and staged by the confined updater, accepted by the pinned signer, tried and committed, the old version kept | `sandbox-exec` profile | Landlock, seccomp, its port rule | `AppContainer`, job and mitigation policies, let reach the test host on this computer by an administrator's loopback exemption |
| A release whose broker crashes, rolled back after two starts and marked bad | Yes | Yes | Yes |
| A release not signed by the pinned signer, refused and marked bad, the current version still running | Signed as another identifier | Signed by another key | Not signed (`0x800B0100`) |
| Expired metadata refused | Yes | Yes | By the release tool's tests |
| A full disk for the staging folder: failed cleanly, the current version runs | A small disk image | A small `tmpfs` | Not run |
| A launcher release | The bundle is the launcher's: swapped whole | The launcher's folder exchanged, the old one kept until the new one's healthy start | By the launcher's tests (both copies and the byte) |
| Removal | `--uninstall`, asking before the settings go | `--uninstall`, asking before the settings go | The uninstaller's copy: the folder, the shortcut, the uninstall entry and every container's profile removed |

**AppArmor** (decision 77), on the Linux host with `kernel.apparmor_restrict_unprivileged_userns=1`, as Ubuntu from 24.04 sets it: without the profile the browser refused to run ("cannot make an empty network namespace: setgroups: Permission denied"); with `neoteletext.apparmor` installed every helper started confined and the version was healthy. The host was put back as it was.

**Found by these runs and fixed:** the confined updater lacked `HOME` (macOS); `codesign` refuses a program with bytes after its last segment (the test broker's slot); a shortcut with no item list resolves to nothing; `CheckNetIsolation` takes a container's SID, not its name; and on Windows a version renamed from `staging\` kept the updater's full control and the low integrity label, so it could not start its helpers and the updater could have changed it after the check: the launcher now copies it in (`IMPLEMENTATION-LOG.md` rows 25 to 30).

**Not proven here:** the owner's signers (a Developer ID requirement and notarisation, the hardware module's certificate, the hardware root keys); macOS 11 and Windows 10 1703; the Linux build on the C library 2.28; the macOS offer to move a translocated bundle and the Windows uninstaller's message boxes, which need a hand.
