# Technical debt

What is owed, as `~/devel/neoteletext-common/DEVELOPMENT.md` section 2.2 asks: everything deferred while behaviour was being shaped, to be worked through at integration and removed here as it is done.

## From the efficiency and sandboxing work of 21 and 22 October 2026

Decisions 52 and 53 are recorded in `docs/architecture.md` section 2 for the owner to confirm; their requirements, documents and tests are done (`docs/sandboxing.md`, `docs/sandboxing-security.md`). What is left is below, each with what stands in its way.

### Waits on packaging

- **Windows: the program folder must grant read to `ALL APPLICATION PACKAGES`** (`icacls <folder> /grant *S-1-15-2-1:(OI)(CI)RX`), or the decoder and fetcher start with the restricted token alone and say so. Program Files grants it; a per-user folder does not. The installer sets it.
- **The trusted core is unconfined on every platform.** On macOS it needs the App Sandbox with each helper as its own helper application (requirement 3.8): macOS refuses a nested `sandbox-exec`, so a core under a profile could start no confined helper. On Linux and Windows the core could confine itself after its helpers are started only if the helpers were started by a broker, which is the App Sandbox's shape too. All three wait for packaging.
- **Windows: the interface can read the user's files.** It runs at low integrity without an AppContainer, since it needs the desktop; the AppContainer for a windowed process is a packaging-time question (the window station and the sound endpoint must grant the container).

### Waits on the engine (its own process)

- **The keyboard process waits on the relay 50 ms at a time** (`keyboard/src/keyboard.rs`, `POLL`): the qrkeyboard client's `poll` is a socket read with a timeout and offers no way to be woken, so Finish is noticed within 50 ms by polling. An event-driven wait needs a waker in the engine's qrkeyboard client.
- **The proxy's `CONNECT` reply is read a byte at a time** (the engine's `proxy.rs`, `tunnel`): bounded now by the agent's connect limit on the connect thread, but one system call a byte; reading in chunks needs `open_tunnel` to hand back the bytes it over-read.
- **The decoder's and core's CPU under flashing pages** is the engine's rendering and the core's PNG encoding (eight flashing pages: decoder 0.7 to 1.1 %, core 1.4 to 1.6 %, interface 3.4 %): the next steps are the engine's row renderer per changed row and a faster deflate, both the engine's.

### Waits on a machine or a hand

- **Linux without Landlock, or without unprivileged user namespaces:** the code degrades and reports, by reading; no such machine was to hand. **32-bit Arm:** no seccomp filter exists for it (`seccompiler` writes x86-64, aarch64 and riscv64), so only Landlock and the namespace apply there, reported; not run.
- **macOS, by hand under the profiles:** typing into a channel (text-input services) and paste (the pasteboard) were not driven; the Dock menu, fonts with non-Latin titles, name resolution and sound were. Anything denied shows as a crash; `sudo log show --predicate 'sender == "Sandbox"'` names it.
- **The audio device's stop and start** was watched on macOS only; on Windows and Linux the device opened in the confined interface and nothing more was watched.

### Its own feature

- **The console's Notices tab should show what confined each process** (today each helper prints it to standard error at start, which a Windows helper started from Explorer has no way to show): a message kind from each helper to the core, and an entry per process. Requirement 3.7 would then be checkable from the window.
- **Requirement 3.9's hostile-interface harness** (an interface that sends every kind of message malformed and oversized, against which the trusted core must stand) is still unwritten; the protocol's own tests cover malformed and oversized messages one kind at a time.
- **Damage-limited drawing in the interface:** a frame on macOS is draw 3 ms, render 4 ms, present 6 ms, and the present is CoreGraphics copying softbuffer's fresh buffer, which needs unsafe code or another presentation library to change.

### Shared process document

- `~/devel/neoteletext-common/DEVELOPMENT.md` section 12.1 was updated (the Windows export with `--checksum`, the vendor folder, the desktop session) and is uncommitted in that repository.
