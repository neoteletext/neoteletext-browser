# Technical debt

What is owed, as `~/devel/neoteletext-common/DEVELOPMENT.md` section 2.2 asks: everything deferred while behaviour was being shaped, to be worked through at integration and removed here as it is done.

## From the efficiency and sandboxing work of 21 and 22 October 2026

Decisions 52 and 53 are recorded in `docs/architecture.md` section 2 for the owner to confirm; decisions 55 (the sound helper) and 56 (the trusted core confined behind a broker) are built. Their requirements, documents and tests are done (`docs/sandboxing.md`, `docs/sandboxing-security.md`). What is left is below, each with what stands in its way.

### Waits on packaging

The design of installation and updates on every desktop is `docs/installation-and-updates.md` (adopted 22 October 2026, decision 54); it settles the first item below once built, and the second waits on it.

- **Windows: helpers run from a per-user folder** that grants containers nothing were seen to start (a decoder, 22 October 2026) but not run through a whole session; the installer is to grant `ALL APPLICATION PACKAGES` read on the program folder, as Program Files does, until that is measured. A container that cannot start no longer falls back: the process does not run (decision 58).
- **Windows: the interface can read the user's files.** It runs at low integrity without an AppContainer, since it needs the desktop; the AppContainer for a windowed process is a packaging-time question (the window station must grant the container). Sound no longer stands in the way: it is the sound helper's (decision 55).

### Waits on the engine (its own process)

- **The keyboard process waits on the relay 50 ms at a time** (`keyboard/src/keyboard.rs`, `POLL`): the qrkeyboard client's `poll` is a socket read with a timeout and offers no way to be woken, so Finish is noticed within 50 ms by polling. An event-driven wait needs a waker in the engine's qrkeyboard client.
- **The proxy's `CONNECT` reply is read a byte at a time** (the engine's `proxy.rs`, `tunnel`): bounded now by the agent's connect limit on the connect thread, but one system call a byte; reading in chunks needs `open_tunnel` to hand back the bytes it over-read.
- **The decoder's CPU under flashing pages:** since 23 October 2026 a frame is drawn over the page's last, only the rows the engine's tracker says changed (its E.107): on the flashing test channel's page 100, 211 µs a frame against 868 µs drawn whole, release build; at the 7.6 frames a second that channel sends, about 1.6 ms of CPU a second against 6.6. The whole decoder's share, 2.1 to 2.3 % on a loaded machine, did not move by more than `top` resolves: the rest of it is not rendering, and is to be found with a symbolised profile.

### Waits on a machine or a hand

- **Linux on a default Ubuntu from 24.04** (unprivileged user namespaces restricted by AppArmor): the browser refuses to run there since decision 58, as `a_helper_without_its_namespace_does_not_run` shows by a filter; no such machine was run, and the AppArmor profile that would let it run is the owner's question (`docs/sandboxing.md` section 8.3). Kernels before 6.10 and 32-bit Arm are refused likewise, by reading.
- **macOS, by hand under the profiles:** typing into a channel (text-input services) and paste (the pasteboard) were not driven; the Dock menu, fonts with non-Latin titles, name resolution and sound were. Anything denied shows as a crash; `sudo log show --predicate 'sender == "Sandbox"'` names it.
- **The audio device's stop and start** was watched on macOS only; on Windows and Linux the confined sound helper played a tone and reported the channel sounding and then silent (`a_tone_plays_through_the_confined_sound_helper`), but the device's own stop was not watched.
- **Requirement 3.1's test of the process list** is not written: the processes were seen by hand on macOS (`ps`), Linux and Windows. A test needs the browser started whole, with a window, in each platform's desktop session.

### History and bookmarks (decision 57)

- **By hand:** dragging a bookmark, the folder list of the star's popover, the bar's folder lists and the clearing ranges were driven by tests, not by a hand on the window (posted clicks do not reach it); the panels were seen on macOS only.
- **Bookmark files from other browsers** are read since 23 October 2026 (decision 71) but not written: export is in this browser's format only. The file is named by its typed path; a file chooser waits on the owner (`history-and-bookmarks.md` section 3.7).
- **Session restore (decision 59) by hand:** a maximised window and a display unplugged between runs were not tried; the window's place is not kept on Wayland, where the system does not say it (`history-and-bookmarks.md` section 15.6).
- **On Linux the zone is read once:** glibc's `localtime_r` reads the time zone at its first call, so a change of zone while the browser runs is seen at the next start (`history-and-bookmarks.md` section 14, item 1).

### Cookies (decision 62)

- **By hand:** the cookies panel, its settings row and the clearing switches were driven by tests; a real sign-in on a channel that sets cookies, kept across a restart, waits on a channel that does (the reference channel sets none).
- **The Keychain may ask an unsigned build** whether it may read the cookie file's key, after each rebuild, and the browser's start waits for the answer; a signed release is not asked (`docs/cookies.md` section 15 item 5).
- **Linux without a Secret Service** (a minimal desktop, or none) keeps cookies for the run only, and says so in the panel. The Linux gate host has GNOME Keyring installed for the test (22 October 2026).
- **Windows: no channel on loopback.** A fetcher's AppContainer cannot reach `localhost`; a loopback exemption is a packaging question (`docs/cookies.md` section 15 item 11).
- **Device Bound Session Credentials** wait on the engine's REQ-WEB-017.
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

## To do, on the owner's word, not yet started

- **Bring every document up to date** (the owner, 23 October 2026: "the documents have drifted, update them all"). Not actioned yet. Every requirement's State checked against the code and its tests, with dated evidence, and every "built" and "not built" list against what is built. Found in the review of 23 October 2026, a start, not the whole: requirement 7.5 says each fetcher holds its own cookies (decision 62 moved them to the trusted core); 7.11 says Transport Layer Security is not built (it is, decision 41); 12.5 says the menu button has no menu (it has, decision 57); 15.1 says macOS only (Windows and Linux were run, decisions 52 to 59); 12.1 lists session restore as not built (decision 59); `architecture.md` section 15 lists WebSocket push as not built though the latency test uses it, and its "Built" heading still reads 16 October 2026 with 67 tests. Then the other documents the same way: the feature designs' "what building changed" sections, `security-analysis.md`'s boundaries and ranked risks, `licensing.md`'s dependency list, `CREDITS.md`, and the engine's documents that describe the browser (its own process). Each finding fixed where it is, as the process's section 4.1 forms say.
- **The installer and the updater, fully** (the owner, 23 October 2026: "FULLY implement the installer and updater"). Not actioned yet. All of `docs/installation-and-updates.md` as adopted (decision 54) and answered by the owner (its section 12), on all three desktops, nothing left as a stand-in: the launcher, side-by-side versions and the state file switched by an atomic rename; the trial with its health check and rollback after two failed starts (sections 3.1 and 3.2); the user's data kept across versions (section 3.3); what the user sees (section 3.4); each platform's installer, per user and without an administrator (section 4: macOS, Windows, Linux); the two signatures by keys held apart, The Update Framework in the updater (sections 5 and 8.2); the release files (section 6); updating the launcher itself (section 7); the updater confined and only staging (section 8.1); the repository, its roles and keys, hosting and the timestamp's 7 days (section 9); the release procedure (section 10); its requirements (section 11) added to `requirements.md`; and every proof of section 13 run on every platform. It also carries what packaging settles elsewhere in this file: code signing and macOS notarisation, which ends the Keychain prompt on every new build; the Windows interface's AppContainer and the program folder's grant ("Waits on packaging" above); and the Linux AppArmor profile a default Ubuntu needs (`sandboxing.md` section 8.3). Libraries it needs (The Update Framework's client among them) are the owner's to approve first.
- **Tabs dragged between windows** (decision 69 section 2): waits on multiple windows (the item above); the panel and dragging by a real hand on macOS, Windows and Linux.
- **A channel's own log lines in the console's Log tab** (decision 70): wait on the engine's channel log record (asked of the engine 23 October 2026); then a fetcher or decoder message carrying them, bounded, to the trusted core, and the Log tab shows them as the channel's.
