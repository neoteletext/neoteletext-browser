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

## 5. Checks that keep these properties

- The release build refuses a launcher that differs from the last launcher release, unless the release is marked as one.
- The full gate runs section 13 of the design on every desktop.
- `cargo deny` covers the updater's dependencies.
- The fuzz targets cover the updater's readers (The Update Framework's metadata through `tough`'s parser, the state file, the gzip reader).
- The key ceremony is recorded in `docs/releases.md` at each root or targets key change.
