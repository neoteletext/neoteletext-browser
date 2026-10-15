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
| B on Linux | Is the web content process kept from the display, as on macOS? | **No.** Under Tauri, WebKitGTK's content process sandbox is off, and Tauri cannot turn it on. With the sandbox on, the display is still mounted into it: code in the content process connected to an X display that requires a cookie and covered the strip's area with its own window, and under Wayland it connected to the compositor and was offered the protocol for drawing above every window. |

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

**Spikes:** `spikes/sandbox-network/` (Proof A, and the `SPIKE_WEBKIT_SANDBOX` switch of 6.3) and `spikes/webkitgtk-sandbox/` (6.2 to 6.5).

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
3. **Under Tauri as it stands,** a compromised content process has every right of the window host: its files, its display and its network. Section 6.1's empty network namespace removes the network; nothing has been measured yet that removes the files.
4. **Requirement 11.1's reason** ("on macOS the web content process is denied the window server") has no counterpart on Linux. On Linux the strip protects against page code, not against an escape into the content process.

### 6.7 The decision this needs

These options are for the owner. Option 1 is recommended.

1. **Accept, and confine the window host from outside.**
   - The window host, and with it WebKitGTK's processes, runs in an empty network namespace (measured, 6.1) under a Landlock rule set. Landlock is inherited by child processes; whether WebKitGTK runs under a rule set narrow enough to matter is not yet measured.
   - The strip's weakness against a compromised content process (finding 2) is recorded as a remaining risk on Linux.
2. **As option 1, and also turn WebKitGTK's sandbox on** by carrying a one-line change to `wry` (6.3), until `wry` offers it.
   - It adds a seccomp filter and a private file system view to the content process.
   - It does not stop finding 2.
   - It means maintaining a patched dependency.
3. **As option 1 or 2, and on Linux show the strip's guarantee as weaker** to the user, for example in the site information view.

---

## 7. Not yet proven

| Item | Why not yet |
|---|---|
| Proofs A and B on Windows (WebView2) | No Windows machine available |
| Covering the strip under Wayland, and on compositors without the layer shell (GNOME's) | Section 6.5 measured the connection and the protocols offered, not a covering surface |
| WebKitGTK under a Landlock rule set for the window host | Needs the window host's file needs listed first |
| Proof B on Linux with a window manager running | The test display had none; override-redirect windows bypass window managers by design |
| Proof D: the hostile interface harness | Needs the real trusted core and its message set; first item of the build |
| Proof E: fetching never depends on viewing | Needs the fetching policy in neoteletext or the stand-in; part of the build |
| Why Tauri's proxy setting had no effect | Not investigated further; not relied on |
| Behaviour on Retina displays | This machine's display runs at 1× scale |
