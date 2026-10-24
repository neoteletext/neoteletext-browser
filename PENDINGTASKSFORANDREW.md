# Pending tasks for Andrew

What only Andrew can do: accounts, money, keys, people, machines, and checks that need a hand on a real window or phone. Each item says what it unblocks. Remove an item here when it is done.

## Accounts and certificates (unblock signed releases, `docs/installation-and-updates.md` sections 4 and 10)

1. **Join the Apple Developer Program** (paid yearly), then create a **Developer ID Application** certificate on this Mac. It signs and notarises the macOS bundle. Without it macOS 15 will not open the browser for an ordinary user. Checked 22 October 2026: this Mac has no code-signing identity. A signed build also lets the Keychain trust the browser across updates: an unsigned build may be asked for permission to read the cookie file's key after each rebuild (`docs/cookies.md` section 15 item 5).
2. **Get a Windows code-signing certificate** whose key sits in a hardware module (the CA/Browser Forum rule since June 2023): a hardware token from a certificate authority, or a cloud signing service such as Microsoft's. It signs every Windows executable and the installer.
3. **Choose the update host** for `update.neoteletext.com` (decisions 17 and 25). It must serve static files over TLS and host nothing else, and it need not be trusted.

## Keys and people (unblock The Update Framework repository, section 9)

4. **Buy five hardware security keys**:
   - three for the root role;
   - one for the targets role;
   - one for the release key, which signs the Linux release's files and, since decision 92, the Windows manifest (its file is still named `linux-release.pub`).
5. **Name the three people who each hold one root key** (requirement 13.9), one of them keeping theirs in a safe. Two of the three are needed to sign a new root.
6. **Hold the first key ceremony.** It creates the root, targets and Linux release keys, signs the first root, and is recorded in `docs/releases.md`.

## Machines

7. **Pick the machine that re-signs the timestamp daily** (section 9.3). It holds the timestamp key only, and if it misses seven days, every user sees "Updates unavailable".

## Checks that need a hand

8. **Real phones against qrkeyboard.com** (protocol 3), as `DEPLOY.md` step 5 of qrkeyboard asks.
9. **The settings panel by hand**: press each control and listen to the volume. Posted clicks do not reach the window.
10. **The phone keyboard through a proxy**: its relay reached through the proxy setting.
11. **Typing into a channel and paste on macOS under the sandboxes**: the text-input services and the pasteboard, which the profile allows but no test drove.

12. **Listen on Windows and Linux with real speakers.** The sound helper played confined on both (`docs/sandboxing.md` section 7.5), but into the virtual machine's device and the Linux host's null sink: nobody heard it.

## Decisions

13. **Confirm decisions 52 and 53** (`docs/architecture.md` section 2): every helper sandboxed on every platform, and the efficiency work. Both are built and marked "owner to confirm".
14. **Confirm decision 57**, history and bookmarks, built while you were away: the 17 departures are in `docs/history-and-bookmarks.md` section 14.
15. **Decision 58 on Linux, decided while you were away (decision 77):** a stated requirement, not a system-wide package. Every Linux version ships `neoteletext.apparmor`, and `neoteletext --install` prints the two commands an administrator runs once. Confirm, or ask for a profile-only package instead (`docs/installation-and-updates.md` section 14 item 11).
16. **Decisions 10 and 44 under decision 58:** plain `http` channels and plain `ws://` push are allowed, marked "Not secure". They are not downgrades (nothing falls back to them), but they are weaker modes a user can choose. Say whether they stand.

## Housekeeping

17. **The prototypes backup**, `~/devel/neoteletext-browser.prototypes.bak-2026-10-21-141115` (33 GB): delete it when you no longer want it.
18. **teletext-rag** (branch `redesign`) has no remote: say where it should be pushed.
19. **Push** the browser, the engine's `implementation` branch and `neoteletext-common` when you want them on GitHub.

## The installer and the updater (23 October 2026)

20. **Confirm decisions 73 to 82** (`docs/architecture.md` section 2): the installer and the updater built on every desktop, the libraries (`tough` and its tree in a workspace of its own, `miniz_oxide`, `rustix`, `rcgen` for test certificates), the crate `os-install` with unsafe code (requirement 10.12 amended), the release identity built in, Windows packaging, the macOS update folder, the launcher's contract with the broker and the health check, the offer in the browser, and removal. The departures are `docs/installation-and-updates.md` section 14; the questions met are `IMPLEMENTATION-LOG.md`'s section "Installer and updater".
21. **At the key ceremony** (item 6), with the release tool (`native/tuf/release`): each hardware key is named `command:<public key>:<program>`, its program writing the raw Ed25519 signature of its input (a PKCS #11 or OpenPGP card tool); `neoteletext-release root native/release-identity/production/root.json 1 --root … --root … --root public:… --targets … --snapshot … --timestamp …` signs the first root with two of the three; then write `linux-release.pub`, `macos-requirement` (`anchor apple generic and certificate leaf[subject.OU] = "<team>"`) and `windows-signer` (`neoteletext-release signer-key <a signed program>`) into `native/release-identity/production/`, and commit them. Until then a production build updates nothing.
22. **The Linux release build on the C library 2.28:** `scripts/release/build-linux.sh` needs the AlmaLinux 8 image. Since 24 October 2026 the script pins the image by digest and every package by version (finding F-73), and the image is pulled on `foundintelquiet4060` (`podman rmi docker.io/library/almalinux:8` removes it); the container build itself has never run end to end.
23. **An icon of the browser's own**, for the bundle, the Windows programs and the Linux menu entry: the art is yours.
24. **By hand:** the macOS "Move to Applications" alert of a bundle opened from the disk image (a dialog nobody could press), and the Windows uninstaller's two questions from Settings › Apps.
25. **A channel on this computer on Windows** needs an administrator's one command, which no per-user installer may run: `CheckNetIsolation.exe LoopbackExempt -a -p=<SID>`, where the SID is the fetcher's container's, the key under `HKCU\Software\Classes\Local Settings\Software\Microsoft\Windows\CurrentVersion\AppContainer\Mappings` whose moniker is `neoteletext.fetcher` (the tool refuses the container's name; decision 78 of the installer work).

## The to-do list's round (23 October 2026)

26. **Confirm decisions 66 to 72** (`docs/architecture.md` section 2), each taken while you were away and recorded with its reasons in `IMPLEMENTATION-LOG.md`'s first section: no fonts from channels (66); Fastext, reveal, hold, mix and sub-pages (67); the remote control (68); dragging tabs and their overflow panel (69); the console's Log (70); bookmarks from other browsers, Move…, and new tabs from Command-click and the middle button (71); multiple windows (72). Two choices in them change nothing you had chosen but may not be what you want: Command-Shift-N stays "New private tab", with "New private window" in the menu only (72); and Command-Shift-R shows the remote (68).
27. **Device Bound Session Credentials:** the engine has them (its E.110); the browser needs a key store on each desktop, whose libraries are yours to approve by name: `security-framework` (macOS), `windows` (Windows), `secret-service` or `oo7` with `p256`'s `ecdsa` feature (Linux).
28. **A file chooser for importing bookmarks:** the path is typed today (`docs/history-and-bookmarks.md` section 3.7); a chooser needs the broker to show the platform's own, which is a new power for the broker.
29. **The engine's decisions E.106 to E.111**, taken while you were away (the engine's own records): the qrkeyboard waker, the proxy's answer read in chunks, no new renderer for flash, links and row 24 in the formats crate, channel log lines in TTI (`LG`), Device Bound Session Credentials, and fonts named by channels withdrawn (E.111). The engine's full gate on them is recorded in `IMPLEMENTATION-LOG.md`'s status section.
30. **`CLAUDE.md` of this repository** has two facts behind the code, which are yours to change: the processes (it lists five; there are also the broker, the sound helper and the updater, with the launcher before them), and unsafe code (it names `dock-menu` and `web-opener`). Requirement 10.12, amended 24 October 2026, names every module that may hold unsafe code; the line could read: "**Unsafe code** only in `dock-menu` (`src/mac.rs`), `web-opener` (`src/windows.rs`), `confine` (`src/windows.rs` and the leaves of `src/windows/` but `channel.rs`; `src/linux/namespace.rs`, `runtime.rs` and `probe.rs`; `src/unix/descriptors.rs` and `time.rs`) and `os-install` (`src/macos.rs`, `src/windows.rs`), each module saying so with its own `#![expect(unsafe_code, reason)]` (requirement 10.12)."
31. **Requirement 3.2** (the trusted core reads no outside format): the trusted core reads `Set-Cookie` lines with the engine's cookie jar since decision 62; say whether the requirement is amended or the jar moves.

## The security review and the reorganisation (24 October 2026)

32. **Confirm decisions 86 to 100** (`docs/architecture.md` section 2), taken while you were away to settle what the security review left open, each with the finding it answers and the alternatives not taken: the Linux helpers kept off the session's sockets (86); the launcher accepting a copy on every desktop (87); the broker reading only teletext, never through a link (88); the local-network rule in the sandbox, with connecting in the broker as the planned design (89); the Windows interface in an AppContainer (90); a floor on the update root's version (91); the Windows manifest signed (92); a start deadline, then a health deadline (93); Linux helpers signalling only themselves, keeping the Linux 6.10 floor rather than requiring Landlock 6 (94); a narrow AppArmor profile (95); the phone relay's reach (96); macOS paths as profile parameters (97); the cookie key not wiped (98); phone keys outside a typing page only moving between pages (99); and the console, the web opener's characters and the relay override (100). Each "Now" column says how far it is built.
33. **Confirm the engine's decisions E.112 to E.116** (the engine's `docs/spec/decisions.md`), taken for the same review: a form redirected with its method kept stays in its target's origin (E.112); the wider list of local names through a proxy (E.113); the web agent's other choices (E.114); more address ranges counted local (E.115); the qrkeyboard screen trusting the phone and the relay less (E.116). They reach the browser with its re-pin at the engine's `5311f98` on 24 October 2026.
34. **Three dependency choices, each yours by name:**
    - **`subtle` 2.6.1** (BSD-3-Clause) in the engine's `neoteletext-qrkeyboard`, for a comparison the compiler promises to keep constant in time (E.116 item 4). It is in the lockfile already, through `aes-gcm`, `p256` and `rustls`; a direct dependency is a library added. Until then the fold stays.
    - **`unicode-normalization` in `protocol`** (finding F-36): the bound on runs of combining marks counts only the blocks that hold nothing but combining marks, since `protocol` has no Unicode tables; `unicode_normalization::char::is_combining_mark` would count every one, Devanagari's, Hebrew's and Arabic's among them. Decision 49 approved the crate for the trusted core alone, so this is a new dependency edge.
    - **A shared network client crate:** the engine's app (`fetch/transport.rs`), the engine's qrkeyboard (`net/`) and this browser's fetcher (`native/fetcher/src/transport.rs`) each keep their own socket reader, TLS pump and proxy connection, held in step by hand. Joining them needs a new crate, a module of `neoteletext-web` that may do input and output, or qrkeyboard depending on the app (the engine's `TECH-DEBT.md`).
35. **In the Windows virtual machine,** what the run of 24 October 2026 (`main` at `29f23f4`) did not reach; the workspace's tests, the interface container's test and the fetcher's tests passed there:
    - **The interface in its AppContainer** (decision 90), in the desktop session: the window opens and draws a teletext page and the strip's text; typing and an input method's composition (Japanese, say) in the address field; copy and paste in fields; full screen, a change of display scaling and resizing; the pointer's shapes. If the window cannot be made in its container, the work stops for your choice.
    - **The broker's file reads** (decision 88): a `.t42` in Documents and a folder of TTI files open; a file under `%APPDATA%` is refused; a junction in the settings folder to `%USERPROFILE%\.ssh` is refused.
    - **The settings folder's grant** (finding F-70): a junction in the core's settings folder to `%USERPROFILE%\Documents`, a start, and `Documents`' access list read for an AppContainer entry.
    - **Updates:** `scripts\updates-e2e.ps1`, which now signs each manifest (decision 92) and passes `--test-identity`; a staged release with one byte of `manifest.sig` changed is refused; an update trial commits and a test version that crashes rolls back after two starts (decision 93); a junction inside a staged version is refused when the launcher copies it (finding F-50); `build-windows.ps1` refuses a tree with changes and a test identity without `-Test`, and takes the release key as its fifth argument (findings F-73 and F-24).
    - **Libraries from System32 alone** (finding F-25): `dumpbin /loadconfig` shows `DependentLoadFlags 0x800` on `NeoTeletext-Setup.exe`, `NeoTeletext.exe` and `launcher-a.exe`; a harmless DLL renamed `wintrust.dll` beside the installer in Downloads is not loaded (Process Monitor), and the install still works.
    - **Clippy for the whole workspace** in `native\` and `native\tuf\`: done on branch `fix/windows` (merged with `main`) on 24 October 2026, clean with default features and none, in both workspaces.
    - **By hand:** a link from a page opens in the default browser (decision 100 item 2): done on branch `fix/windows` on 24 October 2026, after its fix of the zoom's last frame; paste into the address field (finding F-54); sound plays (finding F-55); the phone keyboard pairs with qrkeyboard.com in a release build (decision 100 item 3).
    - **Seen on branch `fix/windows`, for your choice:** at the virtual machine's screen (1280 by 800, 100 %) a zoomed page's dots are 1.4 device pixels, under the 1.5 from which links are underlined, so its links are not underlined there (`links.rs`, `MIN_DOT_PIXELS`); on Windows the pointer over a link that leaves stays an arrow, since `winit` maps `CursorIcon::Alias` to the arrow (a hand instead is a one-line change); and whether the build tools' licence terms cover the C runtime linked statically into each program (`docs/licensing.md`).
36. **The virtual machine's share lost files on 24 October 2026** (an `rsync --delete` to the share's root): the scripts and test channels were restored, but not `run-windows.ps1`, `latency-windows.ps1` (a reconstruction stands in its place, marked so in its first line), the installer work's probe scripts (`acl-probe-1`, `health-probe-1` and `-2`, `label-probe-1` and `-2`, `loop-probe-1` and `-2`, `lnk-probe` to `lnk-probe5`, `probe.ps1`; versions of most are in the worktree `.claude/worktrees/agent-a62cf4df30ac47bfe/tmp/win/`), and the results of earlier runs. Say whether any is wanted back.
37. **macOS, by hand:** whether the interface can open a web address through LaunchServices under its profile (an `LSOpenCFURLRef` probe, finding F-52). The interface needs `launchservicesd` to draw its window, so the service stays; the probe says whether that is also a way out.
38. **Confirm decision 101, the phone panel without a PIN** (finding F-71, built at the re-pin at `5311f98`): "Phone connected" comes at the phone's first key, not at the scan, so the panel keeps its QR code until then, and closing it after a scan but before a key ends the session. Taken while you were away: below the QR the panel says "Then press a key on the phone to connect.", and the reference channel's page 600 says "Scan with your phone, then press a key." Not yet tried with a real phone.
