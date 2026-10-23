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
   - one for the Linux release key.
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
22. **The Linux release build on the C library 2.28:** `scripts/release/build-linux.sh` needs the AlmaLinux 8 image; say whether it may be downloaded on `foundintelquiet4060`.
23. **An icon of the browser's own**, for the bundle, the Windows programs and the Linux menu entry: the art is yours.
24. **By hand:** the macOS "Move to Applications" alert of a bundle opened from the disk image (a dialog nobody could press), and the Windows uninstaller's two questions from Settings › Apps.
25. **A channel on this computer on Windows** needs an administrator's one command, which no per-user installer may run: `CheckNetIsolation.exe LoopbackExempt -a -p=<SID>`, where the SID is the fetcher's container's, the key under `HKCU\Software\Classes\Local Settings\Software\Microsoft\Windows\CurrentVersion\AppContainer\Mappings` whose moniker is `neoteletext.fetcher` (the tool refuses the container's name; decision 78 of the installer work).

## The to-do list's round (23 October 2026)

26. **Confirm decisions 66 to 72** (`docs/architecture.md` section 2), each taken while you were away and recorded with its reasons in `IMPLEMENTATION-LOG.md`'s first section: no fonts from channels (66); Fastext, reveal, hold, mix and sub-pages (67); the remote control (68); dragging tabs and their overflow panel (69); the console's Log (70); bookmarks from other browsers, Move…, and new tabs from Command-click and the middle button (71); multiple windows (72). Two choices in them change nothing you had chosen but may not be what you want: Command-Shift-N stays "New private tab", with "New private window" in the menu only (72); and Command-Shift-R shows the remote (68).
27. **Device Bound Session Credentials:** the engine has them (its E.110); the browser needs a key store on each desktop, whose libraries are yours to approve by name: `security-framework` (macOS), `windows` (Windows), `secret-service` or `oo7` with `p256`'s `ecdsa` feature (Linux).
28. **A file chooser for importing bookmarks:** the path is typed today (`docs/history-and-bookmarks.md` section 3.7); a chooser needs the broker to show the platform's own, which is a new power for the broker.
29. **The engine's decisions E.106 to E.111**, taken while you were away (the engine's own records): the qrkeyboard waker, the proxy's answer read in chunks, no new renderer for flash, links and row 24 in the formats crate, channel log lines in TTI (`LG`), Device Bound Session Credentials, and fonts named by channels withdrawn (E.111). The engine's full gate on them is recorded in `IMPLEMENTATION-LOG.md`'s status section.
