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
- **The decoder's and core's CPU under flashing pages** is the engine's rendering and the core's PNG encoding (eight flashing pages: decoder 0.7 to 1.1 %, core 1.4 to 1.6 %, interface 3.4 %): the next steps are the engine's row renderer per changed row and a faster deflate, both the engine's.

### Waits on a machine or a hand

- **Linux on a default Ubuntu from 24.04** (unprivileged user namespaces restricted by AppArmor): the browser refuses to run there since decision 58, as `a_helper_without_its_namespace_does_not_run` shows by a filter; no such machine was run, and the AppArmor profile that would let it run is the owner's question (`docs/sandboxing.md` section 8.3). Kernels before 6.10 and 32-bit Arm are refused likewise, by reading.
- **macOS, by hand under the profiles:** typing into a channel (text-input services) and paste (the pasteboard) were not driven; the Dock menu, fonts with non-Latin titles, name resolution and sound were. Anything denied shows as a crash; `sudo log show --predicate 'sender == "Sandbox"'` names it.
- **The audio device's stop and start** was watched on macOS only; on Windows and Linux the confined sound helper played a tone and reported the channel sounding and then silent (`a_tone_plays_through_the_confined_sound_helper`), but the device's own stop was not watched.
- **Requirement 3.1's test of the process list** is not written: the processes were seen by hand on macOS (`ps`), Linux and Windows. A test needs the browser started whole, with a window, in each platform's desktop session.

### History and bookmarks (decision 57)

- **By hand:** dragging a bookmark, the folder list of the star's popover, the bar's folder lists and the clearing ranges were driven by tests, not by a hand on the window (posted clicks do not reach it); the panels were seen on macOS only.
- **Not built:** reading another browser's bookmarks (the Netscape file, which would need a confined reader of its HTML); a "Move to…" chooser beside dragging; middle-click and Command-click for a new tab (Shift-click does it).
- **Session restore (decision 59) by hand:** a maximised window and a display unplugged between runs were not tried; the window's place is not kept on Wayland, where the system does not say it (`history-and-bookmarks.md` section 15.6).
- **On Linux the zone is read once:** glibc's `localtime_r` reads the time zone at its first call, so a change of zone while the browser runs is seen at the next start (`history-and-bookmarks.md` section 14, item 1).

### Its own feature

- **The console's Notices tab should show what confined each process** (today each helper prints it to standard error at start, which a Windows helper started from Explorer has no way to show): a message kind from each helper to the core, and an entry per process. Requirement 3.7 would then be checkable from the window.
- **Requirement 3.9's hostile-interface harness** (an interface that sends every kind of message malformed and oversized, against which the trusted core must stand) is still unwritten; the protocol's own tests cover malformed and oversized messages one kind at a time.
- **Damage-limited drawing in the interface:** a frame on macOS is draw 3 ms, render 4 ms, present 6 ms, and the present is CoreGraphics copying softbuffer's fresh buffer, which needs unsafe code or another presentation library to change.

