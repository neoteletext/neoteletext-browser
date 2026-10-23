# Installation and updates: security analysis

The analysis of [`installation-and-updates.md`](installation-and-updates.md), as `DEVELOPMENT.md` section 2.3 asks of a proposal that adds inputs, outputs and dependencies. It extends `security-analysis.md` section 8, which analysed the earlier design's update mechanism, and replaces its "not built" summary once adopted.

## 1. Why this is the most powerful part of the browser

The updater decides what code the user runs next. Everything else in `security-analysis.md` protects one channel, one process or the user's files. A fault here can replace all of it at once, on every user's machine, silently. So the design asks for more than one check and more than one key everywhere.

## 2. Threats

| # | Threat | Where it comes from |
|---|---|---|
| 1 | A malicious build served to users | A compromised update host, a network attacker, a stolen key |
| 2 | An old, vulnerable build served (rollback), or new builds withheld (freeze) | A compromised host |
| 3 | A stolen signing key | The owner's machines, a hardware key lost, a cloud signing service |
| 4 | The updater compromised by the data it parses | A crafted metadata file or release file |
| 5 | An update that leaves the user without a browser | A failed download, a full disk, a power cut, a broken release |
| 6 | An update that loses the user's settings | A profile migrated in place |
| 7 | A fault in the launcher, which recovers from every other fault | Our own code |
| 8 | Same-user malware replacing the installed programs | Any program the user runs |
| 9 | The first install itself tampered with | A fake download page, a modified tarball |
| 10 | The installer or updater given more power than it needs | An elevated service; a Windows Installer repair |

## 3. Mitigations

| # | Mitigation | Section |
|---|---|---|
| 1 | Two checks by keys held apart. The Update Framework's targets key, offline, must name the release. The platform's signer, pinned in the launcher, must have signed it. A host or network attacker has neither key. | 5 |
| 2 | Versions only increase in The Update Framework's metadata and in the launcher's acceptance; the launcher's own rollback is only to `previous`. A frozen host shows "Updates unavailable" after the timestamp's 7 days. | 5, 9.3 |
| 3 | The root needs two of three hardware keys held by three people, and can replace any other key. The targets key and the Linux release key are offline hardware keys. Each platform signing key alone can ship nothing (threat 1's two checks). | 9.1 |
| 4 | The updater is confined: the update host's port, the staging folder, no programs. It holds no platform key, so a compromised updater can stage files but not get them run. There is no archive reader, and target paths are checked against a fixed list. | 6, 8.1 |
| 5 | Side by side, an atomic switch of the state file, a trial with a health check, two failed starts roll back, the previous version kept. Every failure point is a test. | 3.1, 3.2, 13 |
| 6 | Each settings format is its own file; a new version copies, never rewrites. | 3.3 |
| 7 | The launcher parses only its own state file and signed files, is small, changes only in launcher releases (byte-identical otherwise), and is updated by atomic exchange (macOS, Linux) or the fixed stub's two copies (Windows). | 3, 7 |
| 8 | Not stopped by design: see section 4. Low-integrity and AppContainer processes, including every helper on Windows, cannot write the install folder. Helpers on Linux and macOS cannot write anywhere. | 4 |
| 9 | macOS: Gatekeeper with notarisation. Windows: Authenticode and SmartScreen. Linux: a detached signature by the release key, whose public key and fingerprint are published on the download page and in the repository. | 4 |
| 10 | Per user, no elevation, no service. No MSI, so no repair puts old files back. | 4.2 |

## 4. What remains

- **Same-user malware (threat 8).** A per-user install is writable by the user, so a program running as the user can replace it. That program could equally change the user's shell, startup items or browser, so the install folder adds nothing to its reach. A machine-wide install would stop it, at the price of an elevated updater (threat 10), which is the larger risk. **Medium, accepted.**
- **The threshold of root keys stolen together.** Two of three hardware keys held by different people. **Low.**
- **The first download.** Users who never check the Linux signature trust the download page's TLS alone, as for any software. **Low to medium**, the industry's norm.
- **`tough`'s 136 crates** (section 8.2 of the design). They run in a confined process that holds no platform key, so a fault there alone ships nothing (threat 4). Their supply chain is checked by `cargo deny`, as every dependency is. **Low.**
- **The macOS trusted core stays unconfined** until the XPC design. **Medium**, as `security-analysis.md` section 16 rank 3 already records.
- **A fault in the launcher (threat 7).** It is small, and section 13's tests try every failure point. **Low.**
- **The timestamp key online.** It can only keep users on the current release for 7 days. **Low.**

## 4a. As built (23 October 2026)

Reviewed against the build of decisions 68 to 77 of the installer work (`installation-and-updates.md` section 14), while the owner was away.

**What each process now reads, and its bound:**

| Process | Input | Who can write it | Bound and check |
|---|---|---|---|
| Launcher (unconfined) | `state` | Anything running as the user | 2 KiB, read strictly (`update::state`), fuzzed (`launcher_state`); an unreadable one is rebuilt from what is installed, never trusted in part |
| Launcher | A staged version's `manifest` and files | The updater, and anything running as the user | 16 KiB manifest read strictly (`update::manifest`, fuzzed `release_manifest`); each file at most 64 MiB, its length and SHA-256 checked; the folder must hold exactly the files named, every path one of the platform's fixed list; then the pinned signer |
| Launcher | The broker's version mark | As above, inside what the platform signed | A bounded search, exactly one mark (`update::mark`, fuzzed `version_mark`) |
| Launcher | The broker's standard output | The version being tried | 64-byte lines; only `healthy` means anything, and only for the version it started as a trial |
| Updater (confined) | HTTP responses | The update host or the network, under TLS | The engine's head and body readers, a body at most 64 MiB (`read_response`, fuzzed `http_response`) |
| Updater | The Update Framework's metadata | As above | `tough`'s parser and limits (1 MiB root, timestamp, snapshot; 10 MiB targets), fuzzed (`tuf_metadata`) |
| Updater | Each release file | As above | `tough` checks the length and hash the targets key signed, then the strict gzip reader decompresses to exactly the manifest's length (`update::gzip`, fuzzed `gzip_member`) |
| Trusted core (confined) | The updater's messages | The updater | Frames of at most 512 bytes, read strictly (`update::wire`, fuzzed `updater_messages`); the reason text cleaned before the panel shows it |
| Trusted core | The launcher's state, through the broker | Anything running as the user | At most 4 KiB, read strictly as the launcher reads it |
| Broker | The core's requests `Healthy` and `UpdateState`, and the updater's spawn | The confined core | No payload, or the spawn's bounded arguments; the updater's folder comes from where the broker is installed, never from the core |

**New weaknesses, ranked:**

- **The AppArmor profile lets any program at the browser's path make a user namespace** (decision 72). On a system that restricts unprivileged user namespaces, a program running as the user that writes itself to `~/.local/share/neoteletext/versions/<v>/neoteletext-x` gains the namespaces the restriction denies, and with them more of the kernel to attack. It gains nothing else: the profile is otherwise unconfined, as the user's programs already are. The alternative, a system-wide install, needs an elevated updater (threat 10). An administrator chooses to install the profile. **Medium-low, accepted.**
- **A compromised trusted core chooses the updater's one port** by naming a proxy. With a compromised updater as well it reaches one other port; it still stages nothing the targets key did not sign and nothing the launcher's pinned signer did not sign. **Low.**
- **A trial version's broker says when it is healthy.** A release that lies commits; it was signed by both keys to be tried at all. **Low**, as the design's threat 1.
- **Windows: a staged version is copied into `versions\`, not renamed** (found end to end, 23 October 2026). Every file the updater writes carries its container's full control, inherited from `staging\`, and the low integrity label an `AppContainer` gives what it writes; a renamed folder keeps both, so the confined updater could have changed a version after the launcher checked it. The launcher copies the folder under `versions\`, whose access list the copy takes, checks the copy again, and only then names it the trial. **Closed.**
- **Windows: every container can read the programs' folder** (`ALL APPLICATION PACKAGES`, as Program Files grants). The programs are the public release; nothing secret is there. **Low.**
- **A test identity built into a release** would trust the test keys. It is chosen only at build time, its name is built in (`release_identity::NAME`), and the release tool's `check` prints it; the release scripts build the production identity unless the variable is set. **Low**, a step of the release procedure to watch.
- **Windows: the uninstaller's temporary copy stays** in `%TEMP%`: a signed copy of the launcher, which does nothing without its arguments. **Low.**

## 5. Checks that keep these properties

- The release tool's `check` refuses a launcher that differs from the last launcher release, unless the release is marked as one (`--launcher-release`); `a_changed_launcher_needs_a_launcher_release` tests it.
- The commit and full gates run the launcher's and the updater's tests of section 13 on macOS and, in the full gate's Linux stage, on Linux; on Windows they run by hand in the virtual machine (`DEVELOPMENT.md` section 12.1), and `scripts/updates-e2e.sh` and `scripts/updates-e2e.ps1` run the whole life of an update with the real programs, by hand.
- `cargo deny` covers the updater's workspace (`native/tuf/deny.toml`) as the browser's.
- The fuzz targets cover every reader above: `launcher_state`, `release_manifest`, `gzip_member`, `updater_messages` and `version_mark` in `native/fuzz`; `tuf_metadata` and `http_response` in `native/tuf/fuzz`.
- The key ceremony is recorded in `docs/releases.md` at each root or targets key change.
