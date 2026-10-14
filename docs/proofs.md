# T42 Browser: proofs

| | |
|---|---|
| **Date** | 15 October 2026 |
| **Machine** | macOS 15.6 (build 24G84), Apple silicon |
| **Versions** | Tauri 2.12.3, `wry` 0.57.0, Rust 1.97.1 |
| **Purpose** | To prove, by measurement, the assumptions in [`architecture.md`](architecture.md) that the design rests on, before building on them. |

Each proof is a throwaway program under `spikes/`, committed so that it can be run again. Results are what was measured on the machine above. Nothing here has been run on Windows or Linux yet (section 6).

**The stand-in engine.** The proofs use `standin/`, a stand-in for the neoteletext engine with the same pipe framing (`neoteletext-app/src/pipe.rs`), synthetic pages at the native 480 by 500 raster with at most 32 colours, and an indexed Portable Network Graphics writer that compresses for real. A stand-in page is about 6 kilobytes, decoded correctly by macOS's own image decoder (`sips`) and by WebKit.

---

## 1. Summary

| Proof | Question | Result |
|---|---|---|
| A | Can the operating system keep the interface's webview off the network? | **No on macOS.** Without the network entitlement WKWebView shows nothing at all; with it, the sandbox does not restrict destinations. **Page code can be kept off the network** by a strict Content Security Policy plus removing WebRTC in every frame: nothing reached the test server. |
| B | Can a native trusted strip share a window with a webview from another process, without being covered or imitated? | **Not with two windows**: a window from another process, even a sandboxed one, covered the strip with a fake. **Yes with one window**: page code is confined to the webview's area, and WebKit's content process is denied the window server and network sockets by Apple's sandbox. |
| C | Can frames flow from a decoder process to image elements fast enough for many pages? | **Yes.** 200 pages redrawn twice a second were all shown, each wave within 0.5 seconds; capacity is about 450 to 520 frames a second, limited by single-threaded encoding. |

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
2. Tauri 2.12.3's `proxy_url` had no observable effect on macOS 15.6: a live proxy was never contacted. The cause was not determined. It cannot be relied on.
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

### 5.4 Requirements affected, once decided

- **Requirement 3.6** (the interface process has no rights): replaced for the window host.
- **Requirement 10.11** (the webview cannot use the network): its third level is impossible on macOS; the first two levels and WebRTC removal become the protection there, with the content process sandbox behind them.
- **Requirement 11.1** (the strip drawn by the trusted core): the strip is drawn by the window host, in the webview's window.
- **Security analysis sections 5.6, 6 and 7:** rewritten with these measurements.

---

## 6. Not yet proven

| Item | Why not yet |
|---|---|
| Proofs A and B on Windows (WebView2) | No Windows machine available |
| Proofs A and B on Linux (WebKitGTK) | Not yet run; the Linux test machine `oldvga` is available |
| Proof D: the hostile interface harness | Needs the real trusted core and its message set; first item of the build |
| Proof E: fetching never depends on viewing | Needs the fetching policy in neoteletext or the stand-in; part of the build |
| Why Tauri's proxy setting had no effect | Not investigated further; not relied on |
| Behaviour on Retina displays | This machine's display runs at 1× scale |
