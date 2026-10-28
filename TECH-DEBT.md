# Technical debt

What is owed, as `~/devel/neoteletext-common/DEVELOPMENT.md` section 2.2 asks: everything deferred while behaviour was being shaped, to be worked through at integration and removed here as it is done.

## From the efficiency and sandboxing work of 21 and 22 October 2026

Decisions 52 and 53 are recorded in `docs/architecture.md` section 2 for the owner to confirm; decisions 55 (the sound helper) and 56 (the trusted core confined behind a broker) are built. Their requirements, documents and tests are done (`docs/sandboxing.md`, `docs/sandboxing-security.md`). What is left is below, each with what stands in its way.

### Waits on packaging

The installer and updater (decision 54, built as decisions 73 to 82, 23 October 2026) settled the first item that stood here: the Windows installer grants `ALL APPLICATION PACKAGES` read and execute on `versions\`, and the helpers start from the installed folder (the end-to-end test). What is left:

- **Windows: the interface's window in its AppContainer.** Since decision 90 (24 October 2026) the interface runs in an AppContainer of its own with no capabilities, and its refusal of the network and the user's files was verified in the virtual machine; that the window works in the container (drawing, typing and the input method, the clipboard, full screen, display scaling, the pointer) is to be checked there by hand (`PENDINGTASKSFORANDREW.md` item 35).

### Measured after the engine's round of 23 October 2026

- **The decoder's CPU under flashing pages:** since 23 October 2026 a frame is drawn over the page's last, only the rows the engine's tracker says changed (its E.107): on the flashing test channel's page 100, 211 µs a frame against 868 µs drawn whole, release build; at the 7.6 frames a second that channel sends, about 1.6 ms of CPU a second against 6.6. The whole decoder's share, 2.1 to 2.3 % on a loaded machine, did not move by more than `top` resolves: the rest of it is not rendering, and is to be found with a symbolised profile.

### Waits on a machine or a hand

- **Linux on a default Ubuntu from 24.04** (unprivileged user namespaces restricted by AppArmor): the browser refuses to run there without the profile it ships (decision 77), and runs confined with it, as measured on the Linux host with the restriction turned on (23 October 2026); a desktop Ubuntu 24.04 itself was not run. Kernels before 6.10 and 32-bit Arm are refused likewise, by reading.
- **macOS, by hand under the profiles:** typing into a channel (text-input services) and paste (the pasteboard) were not driven; the Dock menu, fonts with non-Latin titles, name resolution and sound were. Anything denied shows as a crash; `sudo log show --predicate 'sender == "Sandbox"'` names it.
- **The audio device's stop and start** was watched on macOS only; on Windows and Linux the confined sound helper played a tone and reported the channel sounding and then silent (`a_tone_plays_through_the_confined_sound_helper`), but the device's own stop was not watched.
- **Requirement 3.1's test of the process list** is not written: the processes were seen by hand on macOS (`ps`), Linux and Windows. A test needs the browser started whole, with a window, in each platform's desktop session.

### History and bookmarks (decision 57)

- **By hand:** dragging a bookmark, the folder list of the star's popover, the bar's folder lists and the clearing ranges were driven by tests, not by a hand on the window (posted clicks do not reach it); the panels were seen on macOS only.
- **Bookmark files from other browsers** are read since 23 October 2026 (decision 71) but not written: export is in this browser's format only. The file is named by its typed path; a file chooser waits on the owner (`history-and-bookmarks.md` section 3.7).
- **Session restore (decision 59) by hand:** a maximised window and a display unplugged between runs were not tried; the window's place is not kept on Wayland, where the system does not say it (`history-and-bookmarks.md` section 15.6).

### Cookies (decision 62)

- **By hand:** the cookies panel, its settings row and the clearing switches were driven by tests; a real sign-in on a channel that sets cookies, kept across a restart, waits on a channel that does (the reference channel sets none).
- **The Keychain may ask an unsigned build** whether it may read the cookie file's key, after each rebuild, and the browser's start waits for the answer; a signed release is not asked (`docs/cookies.md` section 15 item 5).
- **Linux without a Secret Service** (a minimal desktop, or none) keeps cookies for the run only, and says so in the panel. The Linux gate host has GNOME Keyring installed for the test (22 October 2026).
- **Windows: no channel on loopback.** A fetcher's AppContainer cannot reach `localhost`; the exemption (`CheckNetIsolation LoopbackExempt -a -p=<the fetcher container's SID>`; the tool refuses the container's name) needs an administrator, so a per-user installer does not give it (decision 78); it is the owner's one command, and the settings panel does not yet say so (`docs/cookies.md` section 15 item 11).
- **Device Bound Session Credentials:** the engine has them since its E.110, behind a feature; they need a key store on each desktop, whose libraries (`security-framework`, `windows`, `secret-service` or `oo7` with `p256`) are the owner's to approve by name (implementation log row 32).
- **Three advisories ignored narrowly** in `native/deny.toml` (RUSTSEC-2026-0370 to 0372): `zvariant`, `zvariant_derive` and `zbus_names` were merged into `zbus` 6.0 on 16 October 2026; the Linux Secret Service client under `keyring` is still on `zbus` 5. Remove the three when it moves.

### Its own feature

- **Requirement 3.9's harness with running helpers:** the harness built on 23 October 2026 drives the trusted core alone; showing that no file or network is reached, and that requests are identical to a quiet run (requirement 4.7), needs a decoder and fetcher running against a test server.
- **Presenting on macOS copies the whole window:** damage-limited drawing was built on 23 October 2026 (`native-interface-design.md` section 12.7), and cut the cost of building and rendering a frame by more than half; the present, about 4 ms of a 5 ms frame, is softbuffer handing Core Animation a fresh copy of the whole window, which ignores the damage and needs unsafe code or another presentation library to change.


## Multiple windows (decision 72)

- **By hand:** dragging a tab off the strip into a window of its own or onto another window's strip, and the menu's New private window and Move tab to new window, are driven by tests; posted pointer events do not reach the window on macOS. Windows on Windows and Linux were not run.
- **The trusted core's window flow in tests:** opening a window starts decoders, which needs the broker; the core's tests drive windows through the session and the core's targeting, and the whole flow was run by hand on macOS.

## From the owner's requests of 23 October 2026 (decisions 63 to 65)

Their tests, the review of the tips and Chrome's "Remove bookmark" were done on 23 October 2026. Left:

- **The macOS window:** a check that the window is on the screen needs a desktop session; for now by hand, as on 23 October 2026.
- **The latency budget under load:** `keys_round_trip_and_pages_reach_the_screen_within_budget` failed with the key-to-page median at 2.05 ms and 3.22 ms against the 2 ms `WAKE_BUDGET` while the machine's load average was about 30, and passed alone at other moments; it is the test, not the code, that the load decides. Whether its budget should allow for a loaded machine, or the gate run it alone, is to settle.


## From the installer and updater work of 23 October 2026 (decisions 73 to 82)

Built while the owner was away; the questions and answers are `IMPLEMENTATION-LOG.md`'s section "Installer and updater". Owed:

- **The owner's:** the Apple Developer ID and notarisation, the Windows signing certificate in its hardware module, the update host, the hardware root keys and their ceremony, the timestamp machine (`PENDINGTASKSFORANDREW.md`). Until then `native/release-identity/production/` names the host alone and a production build checks nothing. The macOS end-to-end test signs ad hoc with an identifier requirement; a Developer ID requirement and notarised stapling are proven only with the certificate.
- **By hand:** the macOS offer to move a translocated bundle (a CoreFoundation alert, which a test cannot press); the Windows uninstaller's message boxes from Settings › Apps (the end-to-end test runs the removal without them); the Start menu shortcut opened by a click.
- **Not run:** the Linux release build on the C library 2.28 (`scripts/release/build-linux.sh`, which needs an AlmaLinux 8 image); macOS 11 and Windows 10 1703, the oldest the design names; the conformance suite `tuf-conformance` (a Python program; its cases are the browser's own tests).
- **The stub is 233,864 bytes,** not the design's few kilobytes (`IMPLEMENTATION-LOG.md`, "Installer and updater" row 26).
- **The shortcut's item list names folders in ASCII:** a user folder with other characters is given as `?` there and in full in the link information; whether Windows then resolves it was not measured.
- **No icon** of the browser's own on any desktop: the art is the owner's.
- **The full gate's Windows stage** does not run the installer's end-to-end test (`scripts/updates-e2e.ps1`); it is run by hand in the virtual machine (`DEVELOPMENT.md` section 12.1), as the Windows build is.

## From the reorganisation and the security fixes of 24 October 2026

The source was reorganised by concept (`~/devel/neoteletext-common/structure/`, documents 20 to 23, `docs/architecture.md` section 14) and the security review's fixes merged (`~/devel/neoteletext-common/reviews/2026-10-24-security/FIX-PLAN.md`, decisions 86 to 100). What they leave, each with why it was left:

### The reorganisation

- **Fields left open, recorded:** interface-core's `Regions` (22 fields) and `TabRegion` (4) keep `pub(super)` fields: they are the hit-test record every drawing file writes and every press reads, with no invariant to keep, and 52 accessors would only wrap a plain record, which its doc says. The site's `Field` (5), `Invaders` (9) and `Happened` (2) are reached by the session's typing and game input in `terminal/session.rs` and by its tests; closing them is moving that input into `pages.rs` and `invaders.rs`, the later step document 23 section 3.7 names, not a seam of its own. The core's test helper `Test` keeps its three.
- **`native/fetcher/src/transport.rs` is 564 lines,** over the 500 a file of code may hold (`00-conventions.md` section 1): findings F-62 and F-63 grew it after the reorganisation measured it at 386. The connect deadline's reader and writer (`Deadline`) and the address check before connecting are the parts to move out.
- **Not taken:** document 20's optional rename of the library's `restore` to `session_file`, the owner's to choose.
- **interface-core built with `phone-keyboard` alone** has an unused import (`Recorder`, `browser/phone_keys/tests.rs`); no shape of `scripts/check.sh` builds it so.
- **Two backup files are tracked:** `native/interface-core/src/size.rs.bak-2026-10-15-195141` and `wall.rs.bak-2026-10-15-195141`, from 15 October 2026. To delete.

### The security fixes

- **The Windows fixes of 24 October 2026 are merged** (`8e3c381`): each AppContainer profile made whole under a lock before a helper starts in it (06bad65), the test file paths on Windows (849e666), the helpers detached from the console with their errors relayed (F-22), the zoom's last frame drawn, and the C runtime linked statically (F-25). What the virtual machine could not check is in `PENDINGTASKSFORANDREW.md`.
- **F-70, unmeasured:** whether the inheritable grant of the core's settings folder (`SetNamedSecurityInfoW`, at every start) follows a junction the confined core made inside it. If it does, the grant moves to install time, on a handle, protected and without propagation (`docs/sandboxing-security.md` section 2).
- **F-12's remainder:** a decoder or fetcher counts as heard at its first message, which comes only after its channel answers, so a hostile channel that crashes one before that still counts against an update trial. A first message from each right after it is confined, a protocol kind for each pipe, would make "heard" mean "confined and running".
- **F-98's remainder:** the hostile harness has no broker, so hand-offs and helper starts are caught only by the refusal a core without one gives, and the decoders' views are not checked. A stand-in for `confine::broker::Broker` serving its handlers in the test process would exercise them for real.
- **F-55, what the engine's mixer still does under the lock:** since the re-pin at `5311f98` the sound helper allocates a sound's buffer before taking the lock (requirement 16.6), but under it the engine's mixer may grow its small table of sounds and frees a sound that one of the same key replaces or that was forgotten. Moving those out needs the engine's mixer to hand back what it drops.
- **The fetcher's connections** (F-62, F-63): the system's resolver cannot be interrupted, so it runs on a thread that outlives the deadline until the resolver gives up; a name whose servers never answer, retried at the agent's rate, holds a few such threads. A handshake that runs out of the deadline shows as a failed handshake on the certificate-error screen; telling a timeout apart is not built.
- **Smaller residuals, each recorded where it applies:** on Windows the broker refuses a reparse point only at a file's last component, so a junction swapped in for a folder of the path after it was resolved is not seen, and closing it needs `GetFinalPathNameByHandle`, unsafe code the core may not hold (decision 88); a Linux helper whose broker ended before the helper first asked for its parent is not ended with it (F-34); the keyboard helper, between making its screen and storing its waker, exits on a closed input without closing the screen, whose relay instance then lapses (F-26); the clipboard's text is cut as soon as it is read, but `arboard` reads it whole first (F-54); the bound on combining marks counts only the blocks that hold nothing else, and the receiving check (`is_clean`) does not count runs (F-36; the owner's choice of `unicode-normalization` would count every mark); on macOS a trial that never says `started` ends the start with an error rather than running the current version, which macOS's one bundle cannot do without an exchange (F-09), and a lost state keeps the installed bundle's mark rather than accepting again (F-80).
- **Planned designs the decisions name, not built:** connecting in the broker and passing the socket to the fetcher, which closes the local network to a compromised fetcher on Linux, and beyond this computer on macOS (decision 89); a file chooser owned by the broker, which closes the reading of teletext-looking files elsewhere (decision 88); an installation owned by root in `/opt` with its AppArmor profile, which closes user namespaces for a program the user writes under the four names (decision 95).

## Found by the document review of 23 October 2026

- **Source comments name other browsers:** 52 comments in 12 files name Chrome, Chromium or Firefox, mostly as the source of a rule or constant the owner asked to follow (decisions 64 and 65); no code of theirs was used, so `CREDITS.md` names neither, and the rule is that code does not name them. Each comment is to be reworded to cite its design section instead, with care, since each also says where a number came from.
- **Requirement 3.2 and the trusted core's engine crates:** the trusted core reads `Set-Cookie` lines with the engine's cookie jar (`neoteletext-web`), which the requirement's check would refuse; the Statement or the check needs the owner's word (requirement 3.2's State).
- **Requirement 5.13:** a redirect that leaves the channel is refused by the engine's agent but never offered through the site-change screen: its `RedirectToOtherChannel` event is not passed on.

## To do, on the owner's word, not yet started

- **The documents were brought up to date on 23 October 2026** by a review of every requirement's State and of the other documents against the code; what it found open is in the section above. Not reviewed: the engine's own documents that describe the browser (the engine's process).

## The standard's maxima and sub-pages on the wire (26 October 2026)

Owed by decision 112: every item of `neoteletext-common/STANDARD-LIMITS.md` section 7 that names this repository, in its order; none is started. The owner's questions are its section 6.

## The icon (decision 124, 28 October 2026)

- **By hand on macOS:** a release bundle laid out and signed (`scripts/updates-e2e.sh` lays one out): its icon in the Finder, the Dock and the About panel; the browser run from `cargo run`, outside a bundle: the Dock's icon, which `dock-menu`'s `set_icon` sets from the confined interface (built and linted, not run).
- **By hand in the Windows virtual machine:** the programs built with their resource (the stub, `NeoTeletext-Setup.exe`, the launcher, `neoteletext.exe`, the interface): Explorer, the Start menu shortcut, Settings › Apps, the taskbar and the window's title bar.
- **By hand on a Linux desktop:** `--install`'s icons in the menu, the window's icon under X11 and Wayland (its class `neoteletext`), and `--uninstall` removing them.
- **The owner's word:** a hand-drawn 16 by 16, since the art's letters cannot be read at 16 by nearest neighbour.
