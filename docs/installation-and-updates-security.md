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
- **The macOS trusted core is confined** behind the broker on every platform since decision 56, which refused the XPC design; what stays unconfined is the broker and the launcher (`security-analysis.md` section 16 rank 3).
- **A fault in the launcher (threat 7).** It is small, and section 13's tests try every failure point. **Low.**
- **The timestamp key online.** It can only keep users on the current release for 7 days. **Low.**

## 4a. As built (23 October 2026)

Reviewed against the build of decisions 73 to 82 (`installation-and-updates.md` section 14), while the owner was away.

**What each process now reads, and its bound:**

| Process | Input | Who can write it | Bound and check |
|---|---|---|---|
| Launcher (unconfined) | `state` | Anything running as the user | 2 KiB, read strictly (`update::state`), fuzzed (`launcher_state`); an unreadable one is rebuilt from what is installed, never trusted in part |
| Launcher | A staged version's `manifest` and files | The updater, and anything running as the user | 16 KiB manifest read strictly (`update::manifest`, fuzzed `release_manifest`); the launcher copies only the files it names, each opened without following a link at its name and read to at most its manifest length (finding F-50), and checks the copy: exactly the files named, every path one of the platform's fixed list, each of its length and SHA-256; then the pinned signer |
| Launcher | The broker's version mark | As above, inside what the platform signed | A bounded search, exactly one mark (`update::mark`, fuzzed `version_mark`) |
| Launcher | The broker's standard output | The version being tried | Read as bytes, 64 at a time; a line counts only when the read before it ended one, so `healthy` after other bytes on one line is not the line `healthy` (finding F-47); only `started` and `healthy` mean anything, and only for the version it started as a trial; each is held to its deadline, also when the broker closes its output and goes on running (decision 93) |
| Updater (confined) | HTTP responses | The update host or the network, under TLS | The engine's head and body readers (`read_response`, fuzzed `http_response`); a body at most `tough`'s 10 MiB for metadata, and for a release file its compressed bound from the manifest's length; each request done within 10 minutes in all (finding F-81) |
| Updater | The Update Framework's metadata | As above | `tough`'s parser and limits (1 MiB root, timestamp, snapshot; 10 MiB targets), fuzzed (`tuf_metadata`) |
| Updater | Each release file | As above | `tough` checks the length and hash the targets key signed, then the strict gzip reader decompresses to exactly the manifest's length (`update::gzip`, fuzzed `gzip_member`) |
| Trusted core (confined) | The updater's messages | The updater | Frames of at most 512 bytes, read strictly (`update::wire`, fuzzed `updater_messages`); the reason text cleaned before the panel shows it |
| Trusted core | The launcher's state, through the broker | Anything running as the user | At most 4 KiB, read strictly as the launcher reads it |
| Broker | The core's requests `Healthy` and `UpdateState`, and the updater's spawn | The confined core | No payload, or the spawn's bounded arguments; the updater's folder comes from where the broker is installed, never from the core |

**New weaknesses, ranked:**

- **The AppArmor profile lets any program at the browser's path make a user namespace** (decision 77; narrowed by decision 95, finding F-23). On a system that restricts unprivileged user namespaces, a program running as the user that writes itself to `<root>/versions/<v>/` under one of the four names the profile now lists (`neoteletext`, `neoteletext-decoder`, `neoteletext-interface`, `neoteletext-sound`; before, any `neoteletext-*`) gains the namespaces the restriction denies, and with them more of the kernel to attack; the profile names the installation's real root, so it no longer misses one under `XDG_DATA_HOME`. An installation owned by root, with its profile, is the design that would close it, not built since it breaks per-user updates (decision 54). It gains nothing else: the profile is otherwise unconfined, as the user's programs already are. The alternative, a system-wide install, needs an elevated updater (threat 10). An administrator chooses to install the profile. **Medium-low, accepted.**
- **The root could be rolled back** (finding F-06, decision 91): the updater verified the chain of roots from its built-in root at every check and kept no record of the highest it had reached, so a host withholding the newer root files could take it back to rotated-away keys. It now keeps that version in `staging/metadata/root-version` and refuses an older root; a compromised updater could delete the file, but it can already stage whatever it likes and still gets nothing run (section 5 of the design). Test: `a_root_older_than_one_reached_is_refused`. **Closed.**
- **Windows: no signature covered the manifest** (finding F-07, decision 92), so programs Authenticode-signed for different releases could be put together under a new version's number. The Windows release now carries `manifest.sig`, the release key's Ed25519 signature of its manifest, which the launcher checks before Authenticode with the code Linux uses; the updater stages it with the release. Tests: `a_windows_manifest_must_be_signed_by_the_release_key`, and end to end through the release tool and the updater, `a_windows_release_is_staged_with_its_signed_manifest`; on Windows itself, by hand in the virtual machine. **Closed.**
- **A compromised trusted core chooses the updater's one port** by naming a proxy. With a compromised updater as well it reaches one other port; it still stages nothing the targets key did not sign and nothing the launcher's pinned signer did not sign. **Low.**
- **A trial version's broker says when it is healthy.** A release that lies commits; it was signed by both keys to be tried at all. **Low**, as the design's threat 1.
- **A staged version is copied, and the copy accepted, on every desktop** (found end to end on Windows, 23 October 2026; on Linux and macOS by the security review of 24 October 2026, finding F-02, decision 87). On Windows every file the updater writes carries its container's full control, inherited from `staging\`, and the low integrity label an `AppContainer` gives what it writes; a renamed folder keeps both. On Linux and macOS the launcher checked the staged files where the updater wrote them and then renamed or exchanged those same files into place, so a confined updater could still change them afterwards, through a hard link it made in `staging/metadata`, a write descriptor it kept open, or a staged folder that was a link, and gain a persistent, unconfined broker. Now the launcher copies the version into a folder only it writes (`versions/<v>.copying`; on macOS `accepting/<v>/`), checks the copy, and renames or exchanges the copy; a staged folder that is a link is refused; and only the updater's own files are kept under `staging/metadata`. `neoteletext --install` copies the tarball's version and accepts the copy the same way (finding F-69). Tests: `a_hard_link_into_staging_cannot_change_an_accepted_version`, `a_staged_root_that_is_a_link_is_refused`, `an_open_write_descriptor_cannot_change_an_accepted_version`, `a_file_changed_after_acceptance_is_not_installed`. **Closed.**
- **Windows: every container can read the programs' folder** (`ALL APPLICATION PACKAGES`, as Program Files grants). The programs are the public release; nothing secret is there. **Low.**
- **A test identity built into a release** would trust the test keys, which sign updates for whoever holds them. It is chosen only at build time and an empty variable is none; the launcher carries an identity mark, and the release tool's `check` refuses a launcher built with a test identity unless `--test-identity` is given; the release scripts refuse to run with the variable set unless `--test` is, and refuse keys in files for a production release (finding F-24). What remains: an owner who passes `--test` to a real release. **Low.**
- **Windows: a library planted beside the installer** (finding F-25, CWE-427): a `wintrust.dll` in Downloads beside `NeoTeletext-Setup.exe` would have been loaded by it. The installer, the stub and the launcher are linked with `/DEPENDENTLOADFLAG:0x800` (`LOAD_LIBRARY_SEARCH_SYSTEM32`), so what they import comes from System32 alone, and the installer and the launcher call `SetDefaultDllDirectories(LOAD_LIBRARY_SEARCH_SYSTEM32)` first in `main`, for what they load later; the stub, which depends on nothing, has the linker flag alone. With the flag, every library a program imports must be in System32, where the Visual C++ runtime (`VCRUNTIME140.dll`) is only once installed, so every Windows program links the C runtime statically (`.cargo/config.toml`, `+crt-static` for every MSVC target): measured in the virtual machine on 24 October 2026, before, every program imported `VCRUNTIME140.dll` and six `api-ms-win-crt` libraries; after, none, the three programs still carrying `0800 Dependent Load Flag`, and each started alone in a folder beside planted `VCRUNTIME140.dll`, `wintrust.dll`, `crypt32.dll` and `userenv.dll` without loading any of them. Implemented; verified by hand in the virtual machine (`dumpbin /loadconfig`, Process Monitor). **Closed when verified.**
- **Windows: the uninstaller's temporary copy stays** in `%TEMP%`: a signed copy of the launcher, which does nothing without its arguments. **Low.**

## 4b. The icon (28 October 2026)

Reviewed against decision 124's build (`installation-and-updates.md` section 16), on 28 October 2026, without the owner.

**What changes on the release's list.** The list of the files a version may have is the boundary that keeps a release from naming anything else (section 6 of the design). It gains data files only: on macOS `NeoTeletext.app/Contents/Resources/NeoTeletext.icns`, on Linux `neoteletext-16.png` to `neoteletext-256.png` and their signatures. Each is required, never executable, at a fixed name with no folder a release chooses, so no path read from a release can escape the version's folder or name a program. A version without them is incomplete and refused; no version has been published, so none in the field lacks them. Windows adds no file: the icon is a resource inside each program Authenticode signs. `MAX_FILES` rises from 32 to 48, still within the manifest's 16 KiB bound at the longest path (an assertion compiled into `update::manifest`).

**Who reads the icon files.** Not the browser: no program of NeoTeletext's decodes them at run time; the Linux launcher only copies them. The desktop does, with its own image readers (Launch Services and ImageIO on macOS, the shell on Windows, the icon theme's loader on Linux), when it shows the menu, the Dock or Explorer. A crafted image aimed at one of those readers would have to be in a release, which takes the targets key and the platform's signer, as any program in the release would (threat 1); with both, an attacker would ship a program rather than an image. **Low.**

**Where the launcher writes.** `--install` on Linux writes outside the installation, into `~/.local/share/icons/hicolor/<side>x<side>/apps/neoteletext.png`, as it already writes the menu entry and the link on the path: the names are the launcher's own, from the sizes it was built with, and the bytes are the accepted copy's (decision 87), checked by the release key's signature. `--uninstall` removes exactly those seven names. A same-user program can replace the icon files, as it can the menu entry (threat 8). **Low, accepted.**

**Building the icon.** The art's reader (`icon::png`) runs in build scripts and the release tool, on the committed art and on what `iconutil` wrote; it is bounded (1024 pixels a side, 16 MiB a file, the image data inflated to exactly its size) and has the no-panic lints and a fuzz target, `icon_png`. The release tool runs `iconutil` by its fixed path, `/usr/bin/iconutil`, never from the search path, and refuses a release whose icon file is not the art pixel for pixel. The Windows resource file is written by the same code from the same art, with no resource compiler.

**The windows.** The interface's icon is the art's pixels, compiled into the program; the confined interface reads no file for it. On macOS, outside a bundle, it hands AppKit a PNG made from the art at build time. On Linux the windows' class becomes `neoteletext`, a name, not a capability.

## 5. Checks that keep these properties

- The release tool's `check` refuses a launcher that differs from the last launcher release, unless the release is marked as one (`--launcher-release`); `a_changed_launcher_needs_a_launcher_release` tests it.
- The commit and full gates run the launcher's and the updater's tests of section 13 on macOS and, in the full gate's Linux stage, on Linux; on Windows they run by hand in the virtual machine (`DEVELOPMENT.md` section 12.1), and `scripts/updates-e2e.sh` and `scripts/updates-e2e.ps1` run the whole life of an update with the real programs, by hand.
- `cargo deny` covers the updater's workspace (`native/tuf/deny.toml`) as the browser's.
- The fuzz targets cover every reader above: `launcher_state`, `release_manifest`, `gzip_member`, `updater_messages`, `version_mark` and, for the release tool's reading of the icon, `icon_png` in `native/fuzz`; `tuf_metadata` and `http_response` in `native/tuf/fuzz`.
- The key ceremony is recorded in `docs/releases.md` at each root or targets key change.
