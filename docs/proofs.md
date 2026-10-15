# T42 Browser: proofs

| | |
|---|---|
| **Date** | 15 October 2026 |
| **Machines** | macOS 15.6 (build 24G84), Apple silicon; Ubuntu 26.04 (Linux 7.0.0), x86-64 |
| **Versions** | Tauri 2.12.4 and `wry` 0.57.0 in the spikes (their lockfiles); Rust 1.97.1 on macOS and 1.94.1 on Linux; on Linux WebKitGTK 2.52.6 (`webkit2gtk-4.1`), sway 1.11 |
| **Purpose** | To prove, by measurement, the assumptions in [`architecture.md`](architecture.md) that the design rests on, before building on them. |

Each proof is a throwaway program under `spikes/`, committed so that it can be run again. Results are what was measured on the machines above. Sections 2 to 4 are macOS; section 6 is Linux. Nothing has been run on Windows (section 7).

**The stand-in engine.** The proofs use `standin/`, a stand-in for the neoteletext engine with the same pipe framing (`neoteletext-app/src/pipe.rs`), synthetic pages at the native 480 by 500 raster with at most 32 colours, and an indexed Portable Network Graphics writer that compresses for real. A stand-in page is about 6 kilobytes, decoded correctly by macOS's own image decoder (`sips`) and by WebKit.

---

## 1. Summary

| Proof | Question | Result |
|---|---|---|
| A | Can the operating system keep the interface's webview off the network? | **No on macOS.** Without the network entitlement WKWebView shows nothing at all; with it, the sandbox does not restrict destinations. **Page code can be kept off the network** by a strict Content Security Policy plus removing WebRTC in every frame: nothing reached the test server. |
| B | Can a native trusted strip share a window with a webview from another process, without being covered or imitated? | **Not with two windows**: a window from another process, even a sandboxed one, covered the strip with a fake. **Yes with one window**: page code is confined to the webview's area, and WebKit's content process is denied the window server and network sockets by Apple's sandbox. |
| C | Can frames flow from a decoder process to image elements fast enough for many pages? | **Yes.** 200 pages redrawn twice a second were all shown, each wave within 0.5 seconds; capacity is about 450 to 520 frames a second, limited by single-threaded encoding. |
| A on Linux | Can the operating system keep the window host off the network? | **Yes.** In an empty network namespace the application's own page rendered, its messages to Rust worked, and nothing reached the test server. |
| Nested display on Linux | Does a nested display of the window host's own close the display as a way out? | **Outward, yes; inward, not with a stock compositor.** On a nested cage compositor the confined window host's page worked, and it could reach none of the user's display sockets or local services; typing into the user's terminal failed. But through cage's virtual keyboard, a process sharing the window host's confinement typed an address and Enter into the window host's own text field. |
| Display host on Linux | Does T42 Browser's own display host (decision 30) close what cage left open? | **Yes, in its smallest form.** Under both an X11 and a Wayland desktop the confined window host's page worked and reached nothing of the user's. The host offered 7 interfaces, none for input injection, screen copy, overlays or the clipboard without focus. Typing into the strip from inside failed; a window asking for full screen left the strip on screen; the user's own typing reached the strip and the hosted window as intended. |
| Input methods and screen readers on Linux | Can the confined window host have both without reaching any service? | **Yes, through the display host.** Text composed by the user's input method (fcitx5, Hangul) reached the strip and the confined window through the display host. The display host published the window's accessibility tree itself; a screen reader's view of it was correct and followed the user, and the confined window host was never on the accessibility bus. Orca read the tree on starting but did not announce later moves in this headless set-up; not explained. |
| B on Linux | Is the web content process kept from the display, as on macOS? | **No.** Under Tauri, WebKitGTK's content process sandbox is off, and Tauri cannot turn it on. With the sandbox on, the display is still mounted into it: code in the content process connected to an X display that requires a cookie and covered the strip's area with its own window, and under Wayland it connected to the compositor and was offered the protocol for drawing above every window. |
| Confinement on Linux | Can the window host be confined from outside, so that a compromised content process gains nothing? | **Partly.** An empty network namespace, private `/run` and `/tmp` holding only the display's socket, and a Landlock rule set leave the webview working, and deny the content process the network, the home directory and every local service. **The display remains a way out:** under X11 a fully confined process typed a command into the user's terminal, and it ran. |

**Consequence:** the separate user interface process of decision 18, with its own window and no network, cannot be built on macOS as designed. Section 5 sets out what replaces it, for the owner to decide.

---

## 2. Proof A: keeping the webview off the network

**Spike:** `spikes/sandbox-network/`.

**Method.** A Tauri application loads a probe page (`probe.ts`) that tries every way of reaching the network:
- fetch, XMLHttpRequest, WebSocket, Server-Sent Events, beacon;
- an image, a font, a stylesheet;
- prefetch, an iframe;
- WebRTC, both directly and through a blank iframe.

Every probe targets a test server (`test-server.py`) that logs each connection and each packet. **The server's log is the ground truth,** not what the page believes. The page's Content Security Policy is off unless stated, so that only the operating system stands in the way. A control run without any sandbox shows that the probe reaches the server, so a clean result cannot be an artefact of a broken probe. Applications were started through LaunchServices (`open`), as users start them.

**Results.**

| Variant | The application's own page | Reached the test server |
|---|---|---|
| No sandbox (control) | Rendered | 9 kinds of request over TCP, and WebRTC STUN packets over UDP |
| App Sandbox, **without** the outgoing-network entitlement | **Blank window: nothing rendered** | Nothing |
| App Sandbox **with** the outgoing-network entitlement | Rendered | Everything, as the control |
| As above, with Tauri's proxy setting pointing at a dead port | Rendered | Everything |
| As above, with a live proxy that refuses every request | Rendered | Everything; **the proxy was never contacted** |
| Sandbox with the entitlement, **strict Content Security Policy** (requirement 10.5) | Rendered | **No TCP requests**; WebRTC STUN packets still sent |
| As above, **plus WebRTC removed** in every frame before page scripts run | Rendered | **Nothing**; WebRTC through a blank iframe also unavailable |

**Findings.**
1. On macOS the network entitlement is all or nothing for a webview. Without it WKWebView renders nothing, not even pages bundled with the application; with it, the App Sandbox does not restrict where requests go. **Requirement 10.11's third level, an interface process without network permission, cannot be met on macOS with the system webview.**
2. Tauri 2.12.4's `proxy_url` had no observable effect on macOS 15.6: a live proxy was never contacted. The cause was not determined. It cannot be relied on.
3. The Content Security Policy of requirement 10.5 stops every TCP route but not WebRTC, which it does not govern.
4. Removing WebRTC's interfaces in every frame, before any page script runs (Tauri's `initialization_script_for_all_frames`), closes WebRTC, including the blank-iframe route.
5. Prefetch did not reach the server in any run.
6. These are protections against page code. A compromised WebKit content process is considered in section 3.

---

## 3. Proof B: one window, a native strip and a webview

**Spike:** `spikes/window-composition/`.

### 3.1 B1: two processes, two windows

**Method.** `strip.swift` draws a native strip in its own borderless window at the floating window level. `attacker.swift`, a separate process, places its own window exactly over the strip, at the screen-saver window level, imitating it. The attacker was run plainly and as an App-Sandboxed application bundle. The screen region was captured each time.

**Result.** In both cases the screen showed only the attacker's fake strip. **A strip in a window of its own can be covered and imitated by any process that can open windows, including a sandboxed one.**

### 3.2 B2: one process, one window

**Method.** `hosted.swift` creates one window holding a WKWebView and a native strip above it. The page tries to:
- draw over the strip, with a fixed element above the viewport at the highest stacking order, translated and scaled;
- open a pop-up window;
- go full screen.

**Result.**
- **The strip was untouched.** The page's element was confined to the webview's own area.
- **No second window opened:** the process owned one window throughout.
- **Full screen did not happen.**

**What protects the strip from a compromised content process.** On macOS, page code runs in WebKit's content process. Its sandbox profile, `/System/Library/Frameworks/WebKit.framework/Versions/A/Resources/com.apple.WebProcess.sb`:
- **denies by default** (line 47, `(deny default)`);
- **denies the window server** (line 52 of the rule at lines 1048 to 1052: `deny mach-lookup … (global-name "com.apple.windowserver.active")`);
- **allows network output only to the system log socket, and only on Apple's internal builds** (lines 1079 to 1082).

So even a content process taken over by an attacker cannot open a window to cover the strip, and cannot open a network connection. It can only ask WebKit's networking process to load addresses, which Proof A's measures stop for page code. Measured as well: WebKit content processes owned no windows anywhere on the system during the test.

**Finding.** A trusted strip is protected by putting it in the same window as the webview, drawn by the process that hosts the webview. It is not protected by putting it in a window of a different process.

---

## 4. Proof C: frame throughput

**Spike:** `spikes/frame-throughput/`.

**Method.**
1. The stand-in decoder runs as a child process over the framed pipe.
2. It is asked for N pages, then sent flash ticks, so that every page is rendered again on every tick: the worst case.
3. Each frame is encoded as an indexed Portable Network Graphics image and sent to the page over a Tauri channel as raw bytes.
4. The page decodes each image away from the screen and swaps it into its `<img>`. A frame superseded during decoding is dropped.

**Results.**

| Load | Frames | Displayed | Time to show every page after a tick |
|---|---|---|---|
| 50 pages, a tick every 250 milliseconds | 1,050 | All, 203 a second | median 99 milliseconds, worst 137 |
| 200 pages, a tick every 500 milliseconds | 2,200 | All, 407 a second | median 332 milliseconds, worst 497 |
| 200 pages, ticks back to back (saturation) | 4,200 | 1,035, the rest superseded | median 193 milliseconds, worst 1,353 |
| 800 pages, ticks back to back (saturation) | 4,800 | 4,092, 456 a second | median 400 milliseconds, worst 1,165 |

Mean image 6.1 kilobytes; mean encoding time 1.1 to 1.5 milliseconds on one thread.

**Findings.**
1. The pipeline sustains about 450 to 520 frames a second on this machine. Single-threaded encoding is the main limit, and encoding can be spread across cores if ever needed.
2. Dropping superseded frames keeps the display current under overload instead of building a queue.
3. Real teletext load is far lower: flash is about once a second and changes only pages that flash, and only visible pages are sent (requirement 9.5).

---

## 5. What this means for the design, and the decision it needs

### 5.1 What does not hold

Decision 18 places the interface in its own process, with its own window, no network and no secrets, beside a native trusted strip owned by the trusted core. On macOS:
- **The interface process cannot be denied the network** (Proof A, finding 1).
- **A strip in the trusted core's own window could be covered** by any process with a window, including the interface process (Proof B1).

### 5.2 What does hold

- **The host process can protect the strip.** The process that hosts the webview can draw a native strip in the same window, and page code cannot reach it (Proof B2).
- **Page code can be kept off the network** by the Content Security Policy plus WebRTC removal (Proof A).
- **Apple's sandbox confines a compromised content process:** it denies the content process the window server, network sockets and everything else not explicitly allowed (Proof B2).

### 5.3 The options

**Option 1, recommended: a window host process separate from the trusted core.**
- **The window host:**
  - owns the window, hosts the webview and draws the native trusted strip;
  - holds the network entitlement, because WebKit requires it, but never uses it;
  - holds no secrets and no data directory.
- **The trusted core stays separate,** holding cookies, history, bookmarks and settings and making every decision.
- **Page code is confined** by WebKit's content process sandbox, the Content Security Policy and WebRTC removal.
- **The strip's integrity rests on WebKit's content process sandbox.** An attacker who escaped that sandbox into the window host could imitate the strip and forge confirmations, but would hold no secrets.

**Option 2: the window host is the trusted core** (the model of Safari and Chrome, whose interface process hosts the web engine).
- **Simpler:** one process fewer.
- **Weaker after an escape:** an escape from WebKit's content process then reaches every secret.
- **The trusted core would hold the network entitlement.**

**Option 1 keeps more of decision 18:** secrets stay out of the process that an escape from the web engine's sandbox would reach.

**Decided:** option 1, by the owner on 15 October 2026 (decision 27 in `architecture.md`). The documents and requirements below were updated the same day.

### 5.4 Requirements changed by decision 27

- **Requirement 3.6** (the interface process has no rights): replaced for the window host.
- **Requirement 10.11** (the webview cannot use the network): its third level is impossible on macOS; the first two levels and WebRTC removal become the protection there, with the content process sandbox behind them.
- **Requirement 11.1** (the strip drawn by the trusted core): the strip is drawn by the window host, in the webview's window.
- **Security analysis sections 5.6, 6 and 7:** rewritten with these measurements.

---

## 6. Linux: Proofs A and B

**Spikes:** `spikes/sandbox-network/` (Proof A, and the `SPIKE_WEBKIT_SANDBOX` switch of 6.3) and `spikes/webkitgtk-sandbox/` (6.2 to 6.5, and 6.8).

**Machine and method.**
- Ubuntu 26.04 with WebKitGTK 2.52.6. The test displays were private: Xvfb with no window manager, and sway with no screen or input devices (`WLR_BACKENDS=headless`). The machine's own desktop was not touched.
- The Tauri spike was built with `cargo build --release --features tauri/custom-protocol`. For Proof A its probe page was pointed at a test server on the same machine, with the Content Security Policy off, so that only the operating system stood in the way.
- Later runs were started under a private session bus (`dbus-run-session`). The machine's session bus had stopped answering, and GTK then waits for ever in `g_application_register` before opening a window.

### 6.1 Proof A: the window host off the network

| Variant | The application's own page | Reached the test server |
|---|---|---|
| No restriction (control) | Rendered | 9 requests over TCP |
| The whole application in an empty network namespace (`unshare --user --map-root-user --net`) | Rendered; its messages to Rust arrived | **Nothing** |

WebRTC was not available in this WebKitGTK build, so it sent nothing in either run.

**Finding.** On Linux, unlike macOS, the window host can be denied the network by the operating system without breaking the webview. WebKitGTK's processes are its children and inherit the empty namespace, so this also denies the network to a compromised web content process.

### 6.2 The web content process under Tauri

**Method.** The spike's `WebKitWebProcess` was inspected through `/proc` while it ran.

**Result.**
- **Seccomp:** none.
- **Namespaces:** the same network and mount namespaces as the application.
- **Files:** it could see every X display socket and the whole home directory.

`wry` 0.57.0 never calls `webkit_web_context_set_sandbox_enabled`, and WebKitGTK's sandbox is off unless the host turns it on.

### 6.3 Turning the sandbox on from Tauri

**Method.** With `SPIKE_WEBKIT_SANDBOX` set, the spike calls `set_sandbox_enabled(true)` on the webview's context through Tauri's `with_webview`, as soon as the window is built.

**Result.** WebKitGTK stopped the application: `Sandboxing cannot be changed after subprocesses were spawned.` Tauri 2.12.4 offers no earlier point. WebKitGTK must be told before the context starts its first web process, and `wry` creates the context and loads the page in one step. **Turning the sandbox on needs a change to `wry`** (one call in `WebContextImpl::create_context`, `src/webkitgtk/web_context.rs`).

### 6.4 With WebKitGTK's sandbox on: the X display

**Method.**
- `sandboxed_view.c` is a minimal WebKitGTK program with the sandbox on.
- It loads a web process extension built from `x11_handshake.c`. WebKitGTK runs the extension inside every web content process, so the extension's code runs as code that had taken over the content process would, under its seccomp filter and namespaces.
- The display requires an authorisation cookie, as a real desktop's does. As controls, `xdpyinfo` connected with the cookie and was refused without it.
- Inside the content process, the extension:
  1. performs the X11 connection handshake itself, with the cookie;
  2. opens an override-redirect window at the strip's place (an override-redirect window is placed and stacked by the client alone; Inter-Client Communication Conventions Manual section 4.1.10);
  3. raises it once the host's window is up.
- The screen was then captured, and read at two points.

**Results.**
- **The sandbox was real:**
  - the content process ran under `bwrap` with a seccomp filter;
  - it had its own network namespace with only the loopback interface;
  - it had its own mount namespace, and the home directory was not visible.
- **The display was mounted in:** WebKitGTK bind-mounts the display socket (`/tmp/.X11-unix/X98`) and the cookie file named by `XAUTHORITY` into the sandbox, and passes both in the environment.
- **The X server accepted the connection** from inside the content process.
- **The fake strip covered the host's window.** At (400, 20), in the strip's area, the screen showed the extension's window (white, `srgb(255,255,255)`). Below it, at (400, 200), it showed the host's page (`srgb(34,34,34)`).

### 6.5 With WebKitGTK's sandbox on: Wayland

**Method.** As 6.4, under sway, with X disabled and GTK's Wayland back end. `wayland_probe.c` connects to the compositor and lists what it offers, first from the host and then as a web process extension.

**Results.**
- The compositor's socket (`/run/user/1000/wayland-1`) was mounted into the sandbox.
- Code inside the content process connected, and was offered the same 50 interfaces as the host, including `zwlr_layer_shell_v1`. That protocol lets any client place a surface above every window.
- Covering was not attempted under Wayland.
- Compositors that do not offer the layer shell were not tested.

### 6.6 Findings

1. **Page code** is kept from the strip on Linux as on macOS: the strip is outside the webview's area, and page code cannot open windows.
2. **A compromised web content process can cover the strip on Linux.**
   - **On macOS** the content process is denied the window server (section 3.2).
   - **On Linux** it must draw through the display, so WebKitGTK gives it the display even when its sandbox is on.
   - **Under X11** any client can place a window over any other; measured.
   - **Under Wayland** a client offered the layer shell can do the same. Without the layer shell, a client can still ask to go full screen; not tested.
3. **Under Tauri as it stands,** a compromised content process has every right of the window host: its files, its display and its network. Section 6.1's empty network namespace removes the network; section 6.8 removes the files and the local services as well.
4. **Requirement 11.1's reason** ("on macOS the web content process is denied the window server") has no counterpart on Linux. On Linux the strip protects against page code, not against an escape into the content process.

### 6.7 The options, and the decision

These options were put to the owner, with option 1 recommended.

1. **Accept, and confine the window host from outside.**
   - The window host, and with it WebKitGTK's processes, runs in an empty network namespace (measured, 6.1) under a Landlock rule set. Landlock is inherited by child processes; whether WebKitGTK runs under a rule set narrow enough to matter is not yet measured.
   - The strip's weakness against a compromised content process (finding 2) is recorded as a remaining risk on Linux.
2. **As option 1, and also turn WebKitGTK's sandbox on** by carrying a one-line change to `wry` (6.3), until `wry` offers it.
   - It adds a seccomp filter and a private file system view to the content process.
   - It does not stop finding 2.
   - It means maintaining a patched dependency.
3. **As option 1 or 2, and on Linux show the strip's guarantee as weaker** to the user, for example in the site information view.

**Decided:** option 1, by the owner on 15 October 2026 (decision 28 in `architecture.md`). Section 6.8 measured it, and added what the measurement showed was missing.

### 6.8 Option 1 measured

**Spike:** `spikes/webkitgtk-sandbox/confined-host.sh`, with:
- `landlock_host.c`, which applies a Landlock rule set and then starts the program;
- `private-mounts.sh`, which gives the program private `/run` and `/tmp` directories;
- `socket_reach.py`, which tries every listening local socket.

**Method.** The Tauri spike is started as the window host would be, inside three layers:
1. **An empty network namespace** (6.1).
2. **A private mount namespace** in which `/run` and `/tmp` are empty apart from the one socket of the display in use. This layer was added because the first run without it failed the third test below.
3. **A Landlock rule set** (Landlock version 8 on this kernel):
   - read access to the system directories, the display's socket directory and the program;
   - write access only to shared memory, the null device and the graphics devices;
   - no home directory and no data directory.

**Results.**

| Test | With fewer layers | Fully confined |
|---|---|---|
| The probe page | — | Rendered; its own asset loaded; its report reached Rust; every network probe failed |
| `open()` of a file in the home directory, called inside the web content process through a debugger | Succeeded (descriptor 26), with the network namespace alone | **Refused** (`EACCES`) |
| Listening local sockets that could be reached by path, of 85 on the machine | 52 with Landlock and the network namespace, among them the user's session bus, the user's systemd, the SSH agent, the GnuPG agent, PipeWire, PulseAudio, the system bus and PostgreSQL | **1: the display** |
| Asking the user's systemd, over the session bus, to run a program (`systemd-run --user`) | **Succeeded with Landlock and the network namespace**: the program, started outside the confinement, wrote a file in the home directory that the confined process itself could not write | Failed: no bus to reach |
| Typing a command into a terminal of the user's on the same X display (`xdotool`, through the XTEST extension) | The command ran (control, without confinement) | **The command ran** |

**Also measured:**
- **Under sway,** the sandboxed content process is offered more than the layer shell. It also gets the virtual keyboard, virtual pointer, screen copy and clipboard control protocols (`wayland_probe.c`, extended). Typing through them was not attempted.
- **The webview works without a writable data directory.** WebKitGTK's network process warned that it could not open its storage, and carried on.
- **Without a session bus,** GTK warned and carried on: no accessibility bus, no input method service (IBus) and no settings service (dconf).

### 6.9 Findings

1. **Landlock alone does not confine.**
   - Landlock does not govern connecting to a Unix socket by path.
   - The user's session bus lets any client have the user's systemd run a program outside the confinement.
   - A private mount namespace that leaves only the display's socket closes this.
2. **Option 1, as measured, denies a compromised content process the network, the user's files and every local service, and the webview still works.**
3. **The display is a way out that no confinement of the window host can close,** because the webview needs the display.
   - **Under X11,** any client can send key presses to other programs, and what they run runs as the user. Measured.
   - **Under wlroots compositors such as sway,** the virtual keyboard protocol is offered to every client. Not demonstrated.
   - **So on Linux under X11, a fault in the web engine reaches the user's account.** That is level 2 of `security-analysis.md` section 5.6, which on macOS is confined to the webview.
4. **Removing the session bus removes services the shell relies on.**
   - **Screen readers:** Orca reaches applications through the accessibility bus, and requirement 12.9 requires Orca to work.
   - **Input methods** use IBus.
   - **Passing those services through reopens part of what finding 1 closed.** The accessibility interface lets a client read and operate other programs. This was not measured.

### 6.10 What remains open

Findings 3 and 4 are **open decision 19** in `architecture.md` section 17. The options:

1. **A nested display of the window host's own.**
   - **How:** the window host runs on a nested X server (such as Xephyr) under X11, or a nested Wayland compositor. A small process outside the confinement shows that display on the user's.
   - **Benefit:** key presses and screen reads reach only the window host's own windows.
   - **Cost:** another process that reads the window host's display protocol. Input methods, the clipboard, accessibility and high-resolution scaling are lost or must be rebuilt across the nesting.
   - Not measured.
2. **Full support only on Wayland compositors that offer ordinary clients no input injection, screen copy or overlay protocols.**
   - **Candidates:** GNOME's and KDE's compositors.
   - **Elsewhere:** under X11 and wlroots compositors, the application warns the user, as in option 3.
   - **Still to settle:** the accessibility bus.
   - Not measured.
3. **Accept, and tell the user.** On Linux under X11 and wlroots compositors, the application says that a fault in the web engine can reach their account.

**Decided:** option 1, by the owner on 15 October 2026, because security is critical (decision 29 in `architecture.md`). Options 2 and 3 would accept the risk on some sessions. Section 6.11 measured option 1.

### 6.11 Option 1 measured: a nested display

**Spike:** `spikes/webkitgtk-sandbox/nested-host.sh`. Two more programs play the parts of the trusted strip and the attacker:
- `strip_standin.c`: a window with one text field, standing in for the strip's address field. It prints whatever is entered followed by Enter.
- `virtual_type.c`: types a line through the compositor's virtual keyboard protocol, as a compromised content process could.

**Method.**
- **The user's desktop:** Xvfb (X11), or sway (Wayland), each run once.
- **The nested display:** cage 0.2.1, a wlroots compositor that runs as a window on the user's desktop.
- **The window host:** started inside cage, confined as in 6.8, and given only cage's Wayland socket. `DISPLAY` is removed, and GTK uses its Wayland back end.
- **One more grant:** write access to `/tmp`, which is the private, empty one. GTK's Wayland back end writes temporary icon files there, and stops without it.

**Results** (the same under both desktops).

| Test | Result |
|---|---|
| The probe page | Rendered; its own asset loaded; its report reached Rust |
| The user's display sockets reachable from inside | **None** (X11: `X0`, `X97`; Wayland: `X0`, `wayland-1`) |
| All of the user's listening local sockets reachable from inside | **None** of 86 (X11) and 87 (Wayland) |
| Typing into the user's terminal through the user's X display (X11 run) | **Failed**: no display could be opened. The same command typed without confinement ran (6.8) |
| What cage offers the confined process | 24 interfaces, among them the virtual keyboard, the virtual pointer and screen copy |
| Typing into the window host's own text field through cage's virtual keyboard | **Succeeded**: the stand-in received `attacker.example` and Enter |

### 6.12 Findings

1. **The nested display closes the way out to the user's account.** The window host and its content process reach nothing of the user's but the nested compositor.
2. **A stock nested compositor still lets a compromised content process forge strip input.**
   - The content process shares the window host's confinement, so it holds the nested compositor's socket.
   - cage offers every client a virtual keyboard, and the window host's own window has the focus.
   - The same holds on the user's display without nesting: under X11 through the XTEST extension (the mechanism measured in 6.8), and under sway through its virtual keyboard.
   - On macOS this is impossible, because the content process has no window server (3.2).
3. **Removing those protocols would not be enough.** Even with no virtual keyboard, a client of the nested compositor can open a window of its own. In a compositor like cage, that window covers the window host's window, strip included, and takes the keyboard.
4. **So on Linux the strip cannot be drawn inside the nested display.** It must be drawn by the process that hosts the nested display, outside the confinement, beside the nested content. Strip input must then go from that process straight to the trusted core. A process inside the nested display could then neither draw over the strip nor type into it, even from the window host itself (level 3 of `security-analysis.md` section 5.6), which is more than macOS gives.
5. **How screen readers and input methods cross the nesting is still unsolved** (6.9, finding 4).

### 6.13 What remains open

Finding 4 needs a nested display host that draws the strip itself and offers its clients only what a window needs. No stock compositor does both. This is **open decision 20** in `architecture.md` section 17:

1. **Recommended: T42 Browser's own display host for Linux.**
   - **What it is:** a small Wayland compositor, run outside the window host's confinement, as a client of the user's desktop.
   - **What it offers the window host:** only core window protocols, such as surfaces, shared memory and graphics buffers, and the desktop shell's windows. Input comes only from the user's real devices.
   - **What it does itself:** composites the window host's window below a natively drawn trusted strip, and sends strip input to the trusted core.
   - **Cost:** it means choosing a compositor library, such as Smithay in Rust, which is an architectural decision, and building and maintaining a new trusted component. Not built or measured.
2. **Patch a stock compositor** such as cage to remove the protocols and to draw the strip. This means carrying a patched C program in the trusted path.

**Decided:** option 1, by the owner on 15 October 2026 (decision 30 in `architecture.md`). It is written in Rust on Smithay (decision 31). Screen readers and input methods are handled by the display host, outside the confinement (decision 32, section 6.16).

### 6.14 The display host proved

**Spike:** `spikes/display-host/`, the smallest form of the display host, in Rust on Smithay 0.7.0. It runs as a window on the user's desktop through Smithay's winit back end, and hosts one program on a nested display. That program is the confined window host, started by `spikes/webkitgtk-sandbox/nested-host.sh` exactly as in 6.11. The display host:
- offers the hosted program only the interfaces listed below;
- places every window it is given below the strip and sizes it to that area, whatever the window asks for, full screen included;
- draws the strip last, above everything;
- moves the keyboard only when the user clicks;
- prints what the user types in the strip, where the trusted core would receive it.

Two programs from 6.11 were used again, and one more was added:
- `virtual_type.c`: the attacker typing through a virtual keyboard;
- `strip_standin.c`: an ordinary text field;
- `cover_attempt.c` (new): a window that asks for full screen at the top left corner.

**What it offers** (as listed by a client on the nested display): `wl_compositor`, `wl_subcompositor`, `xdg_wm_base`, `wl_shm`, `wl_data_device_manager`, `wl_seat`, `wl_output`.
- The data device was added because GTK 3 sets up no keyboard or pointer without it.
- It gives a client the clipboard only while that client has the keyboard.
- This clipboard is not connected to the user's desktop.

**Results.**

| Test | Desktop | cage (6.11) | Display host |
|---|---|---|---|
| The probe page, confined | X11 and Wayland | Worked | Worked |
| The user's display sockets reachable from inside | X11 and Wayland | None | None |
| The user's listening local sockets reachable from inside | X11 and Wayland | None | None, of 86 and 87 |
| Typing into the user's terminal | X11 | Failed | Failed |
| Interfaces offered for input injection, screen copy or overlays | X11 and Wayland | Virtual keyboard, virtual pointer, screen copy | **None** |
| Typing into the window host's text field through a virtual keyboard | X11 and Wayland | **Succeeded** | **Failed**: no virtual keyboard offered; no input arrived |
| A window asking for full screen at the top left | X11 | — | **The strip stayed on screen**: the strip's area showed the strip's colour (`srgb(0,51,153)`) and the area below showed the window |
| The user's own click and typing in the strip | X11 | — | Arrived as strip input: `"user.example"` |
| The user's own click and typing in the hosted window | X11 | — | Arrived in the window: `"typed by the user"` |

**Method notes.**
- **Tests needing X:** reading the screen and driving the user's keyboard and pointer were done on the X11 desktop only, with `xwd` and `xdotool`.
- **Keyboard focus:** the test display has no window manager, so the display host's window was given the keyboard directly, as a desktop's window manager does on a click.
- **A crash on exit, caused by the test machine:** the display host crashed when it exited, in `xkb_keymap_unref`. The machine has libxkbcommon 0.8.0 installed in `/usr/local`, and it shadows the distribution's 1.13.1, while `libxkbcommon-x11` is the distribution's. A keymap made by one and freed by the other has a different layout. With the distribution's library loaded first, the crash stopped. The test script does that, with a comment saying why. The machine was not changed.

### 6.15 Findings

1. **The display host closes what cage left open.** A compromised content process on its nested display has no interface to inject input, read the screen, place a surface above the strip, or take the keyboard. Its own windows are placed below the strip.
2. **The strip is outside everything the window host can reach.** That holds even for an escape into the window host itself (level 3 of `security-analysis.md` section 5.6). On Linux, the strip is then protected more strongly than on macOS.
3. **The user's own input still works:** in the strip as strip input, and in the hosted window as that window's input.
4. **Not yet in the proof:**
   - pop-up windows, such as menus (not hosted);
   - text in the strip (the strip is a band of colour);
   - several windows;
   - high-resolution scaling;
   - the real trusted core;
   - fuzzing of the host's protocol handling;
   - screen readers and input methods (added in section 6.16).

### 6.16 Input methods and screen readers through the display host

**Decided** by the owner on 15 October 2026 (decision 32): both are handled outside the confinement. Nothing new enters it.

**What was added to the display host:**
1. **Its own window layer.** Smithay's winit back end does not pass on input method events, so the display host opens and draws its window itself, on winit and Smithay's OpenGL ES renderer (`src/window.rs`).
2. **The text-input protocol** (`zwp_text_input_manager_v3`), served by the display host itself (`src/text_input.rs`).
   - Smithay's implementation only relays to an input method client connected to the compositor, which the display host never allows. It discards a window's requests when no such client exists.
   - The display host delivers only text the user's input method composed.
   - It does not offer the input-method protocol, which would let a client be the input method.
3. **The window's accessibility tree,** published to the user's screen reader by the display host through AccessKit 0.25 (`src/accessibility.rs`). It holds:
   - the window;
   - the strip's address field, with its text;
   - the page image, labelled with the page's text. Here the label comes from the command line, standing in for the trusted core.
4. **No raw key releases after composition.** A key release reaches a window only if its press did; the input method keeps the presses it composes with.

**The tests.**
- **Input methods:** `spikes/webkitgtk-sandbox/input-method.sh`, on an X11 desktop.
  - The user's input method is fcitx5 with its Hangul engine, in which the keys g, k, s always compose 한.
  - fcitx5 connects to the display host only, through the X input method protocol.
  - The confined window uses GTK's own Wayland input method module, and no input method service is reachable inside the confinement (6.8).
- **Screen readers:** `spikes/webkitgtk-sandbox/accessibility.sh`, on an X11 desktop with the accessibility bus running.
  - `accessibility_probe.py` reads the bus through the same library Orca uses.
  - Orca 49 itself was then run, with its debug log recording what it would say.

**Results.**

| Test | Result |
|---|---|
| The user types g, k, s and Enter in the strip | Strip input `"한"` |
| The same in the confined window host's text field | The field received `"한"`. It got only the composed text and the Enter key: no raw letters, and no key releases without presses |
| The accessibility bus, read from outside | One application, the display host: a frame "T42 Browser", an entry "Address", and an image labelled with the page text, focused |
| The user types `teletext.example` in the strip | The entry's text read `teletext.example`, and the entry was focused |
| A screen reader asks for the strip while the page has focus | Granted; the display host received the request and moved the keyboard to the strip |
| Focus events while the user moves from page to strip | Received in order: the image, then the entry |
| Orca on starting | Said "T42 Browser", "Address", "entry", "teletext.example" |
| Orca as the user then moves | **Nothing said, and no event logged by Orca**, although a second listener running alongside Orca received both focus events from the display host at the same moments. Not explained; this set-up has no window manager and no real session |
| The confined window host on the accessibility bus | **Never**, though it was running throughout |

**Method notes.**
- **Screen reader flag:** the accessibility bus was told a screen reader was on, as Orca does when it starts, because AccessKit publishes only then.
- **The machine's old libxkbcommon:** fcitx5 and Orca also crashed with the old libxkbcommon in `/usr/local` (6.14), and were run with the distribution's library.
- **Private settings:** fcitx5 and Orca were given private configuration directories, so the machine's settings were not touched.

### 6.17 Findings

1. **The confined window host gets composed text without any input method service.** The user's input method talks only to the display host, and the display host delivers only the composed text.
2. **Screen readers get the window's tree from the display host,** built from state the trusted core holds. The confined window host never reaches the accessibility bus, so that bus gives page code or a compromised content process nothing to use.
3. **Accessibility requests are a new input path into the display host.** A screen reader can move the keyboard, and in the real host also set the address and press the strip's controls. Any process of the user's on the accessibility bus can send them, as it can to every application on the desktop. Nothing inside the confinement can.
4. **Still to test:**
   - Orca following the user's moves on a real desktop session;
   - input methods on a Wayland desktop (only X11 was tested);
   - the input method's candidate window placed at the text cursor;
   - a real page's accessibility tree from the trusted core.

---

## 7. Not yet proven

| Item | Why not yet |
|---|---|
| Proofs A and B on Windows (WebView2) | No Windows machine available |
| Covering the strip under Wayland, and on compositors without the layer shell (GNOME's) | Section 6.5 measured the connection and the protocols offered, not a covering surface |
| Typing through sway's virtual keyboard directly on the user's desktop | Measured through the same protocol on cage, also wlroots (section 6.11); not on sway itself. No longer decisive: the window host never receives the user's display (decision 29) |
| The accessibility bus and input method service passed through the confinement | Section 6.9, finding 4 |
| The rest of T42 Browser's own display host | Section 6.15, finding 4, and section 6.17, finding 4 |
| WebKitGTK and GTK under a system call filter, inside the window host's confinement | Open decision 22 (`architecture.md` section 6.7, approach 1) |
| The display host tagging its own connection with the Wayland security-context protocol | Open decision 23 (`architecture.md` section 6.7, approach 4) |
| Orca following the user's moves | Section 6.16: Orca received no events from the display host after starting, though another listener did; needs a real desktop session |
| Input methods on a Wayland desktop | Section 6.16 tested an X11 desktop only |
| Proof B on Linux with a window manager running | The test display had none; override-redirect windows bypass window managers by design |
| Proof D: the hostile interface harness | Needs the real trusted core and its message set; first item of the build |
| Proof E: fetching never depends on viewing | Needs the fetching policy in neoteletext or the stand-in; part of the build |
| Why Tauri's proxy setting had no effect | Not investigated further; not relied on |
| Behaviour on Retina displays | This machine's display runs at 1× scale |
