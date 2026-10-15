# T42 Browser: requirements

| | |
|---|---|
| **Status** | Proposed, 15 October 2026. Every limit these requirements refer to is a constant defined in neoteletext (decision 26; requirement 2.4). |
| **Companion documents** | [`architecture.md`](architecture.md) explains the design these requirements describe. [`security-analysis.md`](security-analysis.md) analyses its security. |

## How to read this document

- **No abbreviations.** Every technical term is written out in full. Initials appear only where they are a literal identifier in code, configuration, a file name or a standard's title. `architecture.md` section 18 is the glossary.
- **Numbering.** Each requirement is numbered by section: "Requirement 6.3" is the third requirement of section 6. Numbers are never reused; a withdrawn requirement keeps its number and is marked withdrawn.
- **"Must"** means the requirement is mandatory. **"Should"** means it is expected unless a recorded reason says otherwise.
- **Each requirement states:**
  - **Statement:** what must be true.
  - **Reason:** why it matters.
  - **Source:** the owner's decision it comes from (numbered in `architecture.md` section 2), or "Architecture" when it follows from the design, or "Prototype finding" when it was learned on this codebase.
  - **Verification:** how it is shown to hold. "Test" means an automated test. "Check" means an automated check in the project's check script. "Review" means a recorded human review.
- **Engine requirements.** Where a requirement is already met by the teletext engine `neoteletext`, its own requirement identifier is cited, for example "`neoteletext` requirement REQ-SEC-006". That identifier is a literal name in that project.

## Contents

1. Purpose and scope
2. The teletext engine
3. Processes and isolation
4. Messages between processes
5. Network access and resource roles
6. Content detection
7. The security model of a tab
8. Navigation and the site-change screen
9. Rendering and showing pages
10. The window host and its webview
11. The trusted strip
12. Shell features
13. The application update mechanism
14. Build, dependencies and verification
15. Platforms

---

## 1. Purpose and scope

### Requirement 1.1: What the application is
- **Statement:** T42 Browser must be an application for browsing teletext delivered over the web and from local files. Each tab must show one teletext channel, and every page shown must be a teletext page.
- **Reason:** This defines the product and bounds what it may fetch and show.
- **Source:** Decisions 1, 3 and 12.
- **Verification:** Review.

### Requirement 1.2: Security comes first
- **Statement:** Where a requirement trades capability, convenience or performance against security, the choice that is more secure must be made, and the trade-off must be recorded in `security-analysis.md`.
- **Reason:** The owner has stated that being hacked is not acceptable under any circumstances.
- **Source:** Decision 4.
- **Verification:** Review of every change that touches a requirement in sections 3 to 8, 10, 11 or 13.

### Requirement 1.3: No executable content
- **Statement:** The application must never execute code supplied by a page, a channel, a server or a file. The only code that runs must be code signed and shipped as part of the application.
- **Reason:** Downloaded code is the most direct route to compromise.
- **Source:** Decision 4; `neoteletext` decision E.32.
- **Verification:** Review; check that the application contains no interpreter that is reachable from network data other than the decoder's WebAssembly interpreter, which runs only the application's own decoder module.

---

## 2. The teletext engine

### Requirement 2.1: The engine is `neoteletext`
- **Statement:** All teletext recognition, decoding and rendering must be done by the `neoteletext` crates `neoteletext-core`, `neoteletext-formats` and `neoteletext-slicer`. All web protocol reading must be done by `neoteletext-web`. T42 Browser must not contain a second implementation of any of these.
- **Reason:** One implementation, already tested and fuzzed, is safer than two.
- **Source:** Decision 1.
- **Verification:** Review.

### Requirement 2.2: The engine does not depend on a user interface toolkit
- **Statement:** No `neoteletext` crate that T42 Browser uses may depend on egui, eframe, winit, wgpu, glow, Tauri or any other user interface or graphics toolkit.
- **Reason:** The engine must stay usable from any front end, and the trusted processes must not carry toolkit code they do not need. As of 15 October 2026 only `neoteletext-gui` depends on egui, and T42 Browser does not use it.
- **Source:** Decision 1.
- **Verification:** Check: a script runs `cargo tree` on each `neoteletext` crate the application uses and fails if any of those toolkits appears.

### Requirement 2.3: The engine's own guarantees are kept
- **Statement:** The `neoteletext` crates must be built with their own checks, fuzzing and feature rules, and T42 Browser must not enable any feature of them that the engine's rules forbid in a browser build.
- **Reason:** The engine's safety rests on those rules.
- **Source:** Architecture.
- **Verification:** Check: the T42 Browser check script runs the `neoteletext` check script for the revision it builds against.

### Requirement 2.4: Limits and fetching behaviour come from neoteletext
- **Statement:** Every limit on fetching, refreshing and input (response sizes, time-outs, redirects, decompressed size, channel size, request rate, refresh schedule, the grace period after the last tab on a channel closes, and the live-feed malformed-packet threshold) must be a constant defined once in neoteletext and used by T42 Browser by name. The fetching policy of requirements 4.7 and 5.15 to 5.17 must be implemented in neoteletext and switched on by T42 Browser. Where neoteletext does not yet define a limit or behaviour, it must be added there, as listed in `architecture.md` section 5.1, not in T42 Browser.
- **Reason:** One definition of every value and one implementation of channel fetching, as neoteletext's own rule REQ-GEN-014 and requirement 2.1 of this document require.
- **Source:** Decision 26.
- **Verification:** Check: T42 Browser's source defines no constant for any of these limits; review.

---

## 3. Processes and isolation

The process model is decided (decision 18).

### Requirement 3.1: Separate processes
- **Statement:** The application must run as separate processes: a trusted core, a fetcher, one decoder per channel (one per tab is acceptable in the first version), a font helper, a window host, an updater and a launcher.
- **Reason:** Separation lets each part hold only what it needs, so that a compromise of one part gains little.
- **Source:** Architecture.
- **Verification:** Test: the running application's process list matches this set.

### Requirement 3.2: The trusted core contains no web engine
- **Statement:** The trusted core must not link or load a web engine, a JavaScript engine or any parser of network formats. It must receive only typed, bounded results from the other processes.
- **Reason:** The trusted core holds everything the user trusts; it must be the smallest and least exposed part.
- **Source:** Decision 4.
- **Verification:** Check: the trusted core's dependency tree contains no webview, JavaScript engine, HyperText Markup Language parser or `neoteletext-formats` reader.

### Requirement 3.3: The fetcher has only the network
- **Statement:** The fetcher must be able to reach the network and nothing else: no files, no other processes except through its pipe to the trusted core, and no access to the cookie store beyond the cookies of the requests it is currently making.
- **Reason:** The fetcher reads untrusted protocol data and must be worthless to an attacker who compromises it.
- **Source:** Architecture; `neoteletext` requirement REQ-SEC-018.
- **Verification:** Test on each platform: the fetcher's attempts to open a file, start a process or reach the trusted core's data directory fail.

### Requirement 3.4: The decoder has no rights
- **Statement:** Each decoder must have no network access, no file access and no secrets. It must run the `neoteletext` decoder compiled to WebAssembly inside the `wasmi` interpreter, with limits on memory, tables and fuel.
- **Reason:** The decoder reads the most untrusted data; a fault in it must stay inside its own memory.
- **Source:** Architecture; `neoteletext/docs/desktop-browser.md` section 3.6.
- **Verification:** Test on each platform: the decoder's attempts to open a file or a network connection fail; a test module that exceeds its memory or fuel is stopped.

### Requirement 3.5: The font helper has no rights
- **Statement:** Fonts fetched from the network must be read by a confined font helper with no network access, no file access and no secrets. Its output must be checked by the trusted core (hash and table shape) and checked again by the decoder before use.
- **Reason:** A font file is untrusted input in its own format.
- **Source:** Decision 14; `neoteletext/docs/font-security.md`.
- **Verification:** Test: a malformed font is rejected; a font table whose shape is wrong is rejected by both checks.

### Requirement 3.6: The window host holds no secrets and decides nothing
- **Statement:** The window host must never receive cookies, credentials, history or bookmarks beyond what is on screen, or any setting value that affects security, and must have no file access beyond its own bundled resources. It may hold the outgoing-network entitlement only where the platform's webview cannot render without it (macOS: `docs/proofs.md` section 2), and must never open a network connection of its own. On Linux it must have no network at all (requirement 3.10). It must make no decision: it shows the state the trusted core sends and passes on what the user did.
- **Reason:** The window host contains the system webview's interface to the web engine. An attacker who escaped the web content process into it must find nothing of value.
- **Source:** Decisions 4, 18 and 27.
- **Verification:** Test on each desktop platform: file access outside the bundle fails from the window host; the window host's own code makes no network connection (checked with the operating system's per-process network accounting); review of every message type sent to it.

### Requirement 3.7: Operating-system sandboxes
- **Statement:** On each platform the processes must be sandboxed as in `architecture.md` section 6.5: App Sandbox and services on macOS; AppContainer, low integrity and job objects on Windows; Landlock and seccomp filters on Linux, and for the Linux window host the confinement of requirement 3.10 (whether it also gets a seccomp filter is open decision 22); isolated service processes on Android. On iOS, where one process is mandatory, the decoder must still run inside `wasmi`.
- **Reason:** Each sandbox is a second wall behind the first.
- **Source:** Architecture.
- **Verification:** Test on each platform, as in requirements 3.3 to 3.6.

### Requirement 3.8: The macOS sandboxing gap is closed
- **Statement:** The fetcher and decoder must be sandboxed on macOS before the first release. Today `neoteletext` separates them on macOS but does not sandbox them.
- **Reason:** Without it the macOS build lacks the second wall.
- **Source:** Architecture; `neoteletext/docs/user-guide.md`.
- **Verification:** Test, as in requirements 3.3 and 3.4, on macOS.

### Requirement 3.9: Security does not depend on the interface's code
- **Statement:** Every security property in this document must hold even if the window host runs code written by an attacker. No security property may rely on the interface being written with React, with plain markup and script, or correctly at all.
- **Reason:** The owner's principle that the choice of interface technology must make no difference to security. The interface contains a very large engine and third-party code; it is treated as hostile.
- **Source:** Decision 20.
- **Verification:** Test: a hostile interface harness replaces the window host with an adversarial client that sends every intent kind with arbitrary values, malformed messages, floods and replayed messages, and the test shows that no channel change, settings change, secret disclosure, network access, file access or trusted strip forgery results. The test must also show that no information from one channel reaches another through requests (requirement 4.7).
- **Status note:** The covert channel between channels described in `security-analysis.md` section 5.6 is closed by design by decision 22 and requirements 4.7 and 5.15 to 5.17; this requirement is met once those are implemented and pass their tests, **with one accepted exception:** on channels that publish only separate page files, page requests follow viewing (decision 23), so a compromised interface could signal through them.

### Requirement 3.10: The Linux window host is confined from outside
- **Statement:** On Linux the trusted core must start the window host:
  1. in an empty network namespace;
  2. in a private mount namespace in which `/run` and `/tmp` hold only the socket of the display in use;
  3. under a Landlock rule set that grants read access only to the system directories, the display's socket and the application's own files, and write access only to shared memory, the null device and the graphics devices.

  The window host must have no access to the home directory or to any data directory, and its webview must keep no website data. WebKitGTK's processes inherit all three layers, and nothing in the window host may lift them.
- **Reason:** Under Tauri, WebKitGTK's content process has no sandbox of its own. Landlock alone is not enough, because it does not govern connecting to local sockets: with Landlock and an empty network namespace, 52 local services were still reachable. One of them was the user's session bus, through which the user's systemd ran a program outside the confinement (`docs/proofs.md` section 6.8).
- **Source:** Decision 28.
- **Verification:** Test on Linux, as `spikes/webkitgtk-sandbox/confined-host.sh` does:
  - the webview renders the application's page and its messages reach Rust;
  - `open()` of a file in the home directory, called inside the web content process, fails;
  - of the machine's listening local sockets, only the display's can be reached from inside;
  - `systemd-run --user` from inside fails;
  - no network probe reaches a listening test server.
- **Status note:** This requirement does not close the display. Requirement 3.11 does.

### Requirement 3.11: The Linux window host draws on a nested display of its own
- **Statement:** On Linux the window host must never be given the user's display, on any session, X11 or Wayland. It must draw on a nested display whose host runs outside the window host's confinement. The only display socket inside the confinement must be the nested display's. Then, requirements:
  1. The nested display's host must offer the window host no protocol for input injection, screen copy, clipboard access without focus, or surfaces above other windows. It may offer the text-input protocol, through which it delivers only text the user's input method composed, and must not offer the input-method protocol (decision 32).
  2. The trusted strip must be drawn by the nested display's host, outside the nested display, and its input sent by that host straight to the trusted core. That host is T42 Browser's own display host (decision 30).
- **Reason:** The web content process must have a display, and under X11 a confined process typed a command into the user's terminal through it. On a nested display it reached nothing of the user's (`docs/proofs.md` section 6.11). Through a stock nested compositor's virtual keyboard, a process sharing the window host's confinement typed into the window host's own text field, so the strip cannot live inside the nested display (section 6.12).
- **Source:** Decisions 29, 30, 31 and 32.
- **Verification:** Test on Linux under an X11 desktop and a Wayland desktop, as `spikes/webkitgtk-sandbox/nested-host.sh` does:
  - the page works;
  - none of the user's display sockets or local services can be reached from inside;
  - typing into a terminal on the user's display fails;
  - typing and pointing through every protocol the nested display offers fails to reach the trusted strip;
  - a window opened from inside the nested display cannot cover the strip;
  - the user's own typing reaches the strip as strip input, and the window host's window as its input.
- **Status note:** The smallest form of the display host (`spikes/display-host`) passes every one of these checks (`docs/proofs.md` section 6.14).

---

## 4. Messages between processes

### Requirement 4.1: One framed protocol
- **Statement:** All processes must communicate over the framed pipe protocol of `neoteletext-app/src/pipe.rs`: one kind byte, a 32-bit length and a payload.
- **Reason:** One small, strict, fuzzed protocol for every boundary.
- **Source:** Architecture.
- **Verification:** Review.

### Requirement 4.2: Every message is bounded
- **Statement:** Every message kind must have a fixed maximum size, stated in code as a named constant (the pipe protocol is `neoteletext-app`'s). A message larger than its maximum must be rejected before its payload is read.
- **Reason:** Unbounded messages are a route to exhausting memory.
- **Source:** Architecture.
- **Verification:** Test for every message kind: a message one byte over the maximum is rejected.

### Requirement 4.3: A malformed message closes its sender
- **Statement:** A process that sends a malformed message, or a message it is not allowed to send, must be closed, and every tab it served must show an error page.
- **Reason:** A process that misbehaves may be compromised; continuing to trust it is not safe.
- **Source:** Architecture; `neoteletext/docs/desktop-browser.md` section 3.4.
- **Verification:** Test for each process: a malformed message closes it and the error page appears.

### Requirement 4.4: Every message is checked by the receiver
- **Statement:** The trusted core must check every message from every other process against the state it holds: known tab, known channel, sizes within limits, palette within 32 entries, every palette index within the palette.
- **Reason:** A compromised process can send well-formed but false messages.
- **Source:** Architecture.
- **Verification:** Test: messages naming an unknown tab, a wrong channel or out-of-range values are refused.

### Requirement 4.5: Intents from the window host cannot change channel
- **Statement:** No page intent may cause a tab to navigate to another channel, change a security setting, read a cookie or confirm the site-change screen. Such actions must be possible only through strip input from the trusted strip (requirement 11.1).
- **Reason:** The window host is untrusted.
- **Source:** Decisions 4 and 18.
- **Verification:** Test: every intent kind is sent with values that would leave the channel or change a setting, and none succeeds.

### Requirement 4.6: The parsers are fuzzed
- **Statement:** The trusted core's readers of messages from the fetcher, the decoders, the font helper and the window host must be fuzzed continuously.
- **Reason:** These readers are the trusted core's only exposure to the other processes.
- **Source:** Architecture.
- **Verification:** Check: fuzzing targets exist for each reader and run in the full check.

### Requirement 4.7: No interface request or key affects fetching
- **Statement:** No intent from the window host and no key press, however received, may cause, delay, reorder or prioritise a network request. Selecting a page, by key or by intent, may only show a page the channel's decoder already holds. **Exception (decision 23):** on a channel that publishes only separate page files, with no bundle and no contents file, selecting a page not yet held fetches that page's file; such a request must stay within the tab's own channel and within the limits of requirement 5.12.
- **Reason:** A hostile interface sees pages of every open channel; if its requests or forged keys could influence fetching, it could encode one channel's pages in the pattern of requests made to another (`security-analysis.md` section 5.6). Keys must work in the tab, so the protection must not depend on where keys come from.
- **Source:** Decisions 20 and 22.
- **Verification:** Test with the hostile interface harness on bundled channels and channels with a contents file: across every intent kind and every key, with arbitrary values and timing, the sequence and timing of requests reaching a test server is identical to a run with no interface activity. On a channel with only separate page files, test that every request the harness causes is for a page file of that tab's own channel.

---

## 5. Network access and resource roles

### Requirement 5.1: Only teletext resources
- **Statement:** The application must fetch from the network only resources in one of these roles: teletext data (pages, page files and live feeds), channel descriptor, contents file, font, discovery. The updater's fetches of updates are the only exception (section 13).
- **Reason:** Limiting what is fetched limits what can attack the application.
- **Source:** Decisions 12 and 14.
- **Verification:** Test: every request the fetcher makes carries one of these roles; a request without a role is refused.

### Requirement 5.2: A response is read only in its role
- **Statement:** A response must be read only by the reader for the role it was requested in. No response may be passed to a reader of another role, whatever it contains or declares.
- **Reason:** A server must not be able to move its content into a different, possibly weaker, reader.
- **Source:** Decision 14.
- **Verification:** Test: a font request answered with a valid descriptor, a descriptor request answered with markup, and similar crossings are all refused.

### Requirement 5.3: Teletext data is read in the decoder
- **Statement:** Responses in the teletext data role must be passed, unparsed, by the trusted core to the decoder of the channel that requested them, which must detect and read them.
- **Reason:** Detection and decoding read untrusted bytes and belong in the process with no rights.
- **Source:** Architecture.
- **Verification:** Review; test that the trusted core forwards these bytes without reading them.

### Requirement 5.4: Descriptors, contents files and discovery are read in the fetcher
- **Statement:** Channel descriptors (JavaScript Object Notation), contents files and discovery responses must be read by `neoteletext-web`'s strict readers inside the fetcher. The trusted core must receive only the typed, bounded results and must check them again: entry counts, string lengths and address syntax.
- **Reason:** Each is untrusted input; reading it in the fetcher keeps it out of the trusted core.
- **Source:** Decision 14.
- **Verification:** Test: oversized, malformed and hostile descriptors, contents files and discovery responses are rejected; results over the limits are refused by the trusted core.

### Requirement 5.5: Discovery reads only link pointers
- **Statement:** Discovery must read only `Link` response headers and `<link>` elements within the first 8,192 bytes of a HyperText Markup Language response, and nothing else. No HyperText Markup Language may be rendered, stored, cached or passed to any other process. Discovery must run only for an address the user typed that did not answer with teletext.
- **Reason:** Discovery is the application's only contact with HyperText Markup Language; it must stay a narrow pointer extractor.
- **Source:** Decision 14; `neoteletext/docs/web-discovery-security.md`.
- **Verification:** Test with hostile markup; review against `neoteletext/docs/web-discovery-security.md`.

### Requirement 5.6: Discovery results are navigations
- **Statement:** Each address found by discovery must be treated as a page-initiated navigation, checked by the strict address parser, and shown on the site-change screen before it is opened if it leads to a different channel.
- **Reason:** A server must not be able to send the user elsewhere without the user seeing where.
- **Source:** Decision 14.
- **Verification:** Test.

### Requirement 5.7: Fonts are fetched only when named
- **Statement:** A font must be fetched only when a page or channel names it, and must be used only after the checks of requirement 3.5. Teletext fonts must never be passed to the window host.
- **Reason:** Fonts are untrusted input; the webview must never read them.
- **Source:** Decision 14.
- **Verification:** Test.

### Requirement 5.8: Transports
- **Statement:** The fetcher must support the HyperText Transfer Protocol version 1.1 with and without Transport Layer Security, WebSocket, Server-Sent Events and the zxnet live feed. It must not support the push mechanism of the HyperText Transfer Protocol version 2.
- **Reason:** These are the standard ways teletext services publish pages and push updates.
- **Source:** Decision 12.
- **Verification:** Test against a test server for each transport.

### Requirement 5.9: Plain connections are allowed and shown
- **Statement:** Channels reached without Transport Layer Security must be allowed. The trusted strip must show that such a channel is not encrypted for as long as it is shown.
- **Reason:** Many teletext services are plain; the user must still know.
- **Source:** Decision 10; `neoteletext` decision E.32.
- **Verification:** Test.

### Requirement 5.10: The local network is refused by default
- **Statement:** Addresses on the local network (loopback, private and link-local ranges) must be refused unless the user allows a specific address in the trusted strip's settings.
- **Reason:** A remote page must not be able to reach devices on the user's own network.
- **Source:** Architecture; `neoteletext` requirement REQ-SEC-006.
- **Verification:** Test for each range.

### Requirement 5.11: Compression is bounded
- **Statement:** The fetcher must undo `Content-Encoding` compression with a fixed maximum decompressed size, a constant defined in neoteletext (requirement 2.4). A response that would exceed it must be refused.
- **Reason:** A small compressed response must not be able to expand without bound.
- **Source:** Architecture.
- **Verification:** Test with a highly compressible response.

### Requirement 5.12: Every response is bounded
- **Statement:** Every response must have a maximum size, a maximum time to its first byte, a maximum total time and a maximum number of redirects, each per role and each a constant defined in neoteletext (requirement 2.4).
- **Reason:** No server may hold the application's resources without bound.
- **Source:** Architecture.
- **Verification:** Test for each limit.

### Requirement 5.13: Redirects to another channel are navigations
- **Statement:** A redirect that leads to another channel must be treated as a page-initiated navigation and shown on the site-change screen before it is followed.
- **Reason:** A redirect must not move a tab to another channel without the user.
- **Source:** Architecture.
- **Verification:** Test.

### Requirement 5.14: No referrer and a minimal user agent
- **Statement:** The fetcher must send no `Referer` header and only `neoteletext-web`'s minimal user agent profile.
- **Reason:** Requests must reveal as little as possible about the user and what they viewed before.
- **Source:** Architecture.
- **Verification:** Test: inspect requests received by a test server.

### Requirement 5.15: Whole-channel fetching in a fixed order
- **Statement:** When a channel opens, its pages must be fetched in an order fixed by the channel's own data: the whole bundle (`index.t42`) if there is one; every page file listed in the contents file (`contents.txt`), in its order, if there is one; otherwise, for a channel with only separate page files, the initial page, with further pages fetched when selected (decision 23). Fetching must stop at neoteletext's maximum channel size (`contents::MAX_PAGES`) and proceed at neoteletext's request rate (`DEFAULT_FETCH_INTERVAL_US`), and the policy must be implemented in neoteletext (requirement 2.4).
- **Reason:** Fetching that does not depend on viewing reveals nothing about viewing.
- **Source:** Decision 22.
- **Verification:** Test: for each kind of channel, the request sequence is identical whatever pages are viewed; the size limit and rate are enforced.

### Requirement 5.16: Refreshing on a fixed schedule
- **Statement:** A channel's pages must be refreshed only by its declared refresh schedule in `channel.json`, by a fixed default schedule when it declares none, or by its push feed, never according to which page is shown, selected or requested. `neoteletext`'s fetching of a page's own file on viewing and its prefetching of Fastext targets and neighbouring pages (its requirement REQ-NAV-004) must not be used.
- **Reason:** View-driven refreshing and prefetching reveal and can signal what is viewed.
- **Source:** Decision 22.
- **Verification:** Test: refresh requests reaching a test server are identical with and without viewing activity.

### Requirement 5.17: A closed tab's channel keeps refreshing for a grace period
- **Statement:** When the last tab on a channel closes, that channel must keep refreshing on its schedule for a fixed grace period, a constant defined in `neoteletext-browser` (requirement 2.4), before its decoder stops.
- **Reason:** Otherwise the moment a tab closes is visible to the channel's server, and a hostile interface, which can close tabs, could signal through it.
- **Source:** Decisions 20 and 22.
- **Verification:** Test.

---

## 6. Content detection

### Requirement 6.1: The declared media type is never used for acceptance
- **Statement:** The `Content-Type` a server declares must never be used to accept or reject a resource. It may be shown in the developer tools for diagnosis.
- **Reason:** Teletext files are served with any media type.
- **Source:** Decision 13.
- **Verification:** Test: valid teletext served with unrelated media types is accepted; non-teletext served with a teletext-like media type is refused.

### Requirement 6.2: Content decides
- **Statement:** Responses in the teletext data role must be recognised by `neoteletext-formats` content detection (`neoteletext` requirement REQ-INP-005), in the decoder.
- **Reason:** Only content can be relied on.
- **Source:** Decision 13.
- **Verification:** Test with each supported format.

### Requirement 6.3: Unrecognised content is refused, never shown
- **Statement:** A response that detection does not recognise must produce a refusal page that states the HyperText Transfer Protocol status, and must not be cached, passed on or shown in any form.
- **Reason:** Web error pages, images and documents are not teletext.
- **Source:** Decisions 12 and 13.
- **Verification:** Test with markup, images and random bytes.

### Requirement 6.4: Detection is deterministic
- **Statement:** A response valid in more than one format must be read by the first match in detection's fixed order, and detection's decision must not depend on how the network split the data.
- **Reason:** The same bytes must always be read the same way.
- **Source:** Architecture; `neoteletext` requirement REQ-INP-001.
- **Verification:** Test with the same input split at every possible point.

### Requirement 6.5: A misbehaving live feed is closed
- **Statement:** A live feed that produces malformed packets beyond a fixed threshold, a constant defined in neoteletext's live input handling (requirement 2.4), must be closed and reported to the user.
- **Reason:** A hostile feed must not be able to keep the decoder busy indefinitely.
- **Source:** Architecture.
- **Verification:** Test.

---

## 7. The security model of a tab

### Requirement 7.1: Origins
- **Statement:** An origin must be scheme, host and port, as defined by `neoteletext-web`.
- **Reason:** The same definition traditional browsers use.
- **Source:** Decision 9.
- **Verification:** Test.

### Requirement 7.2: The channel is the unit of isolation
- **Statement:** A channel must be identified by its origin and folder path (`neoteletext` decision E.70), and must be the unit for cookies, caches, decoders and the site-change screen.
- **Reason:** The channel plays the part a site plays in a traditional browser.
- **Source:** Decision 9.
- **Verification:** Test.

### Requirement 7.3: Channels never share data
- **Statement:** A channel's pages, cookies, caches and history must never be visible to, mixed with or sent to another channel's decoder or requests.
- **Reason:** The equivalent of the same-origin policy.
- **Source:** Decision 9.
- **Verification:** Test with two channels whose pages, cookies and caches use identical names.

### Requirement 7.4: Tabs on the same channel share
- **Statement:** Tabs showing the same channel must share that channel's cookies and decoder.
- **Reason:** The same behaviour as same-site tabs in a traditional browser.
- **Source:** Decision 11.
- **Verification:** Test.

### Requirement 7.5: The trusted core owns the cookie store
- **Statement:** The cookie store must live in the trusted core. For each request the trusted core must give the fetcher only the cookies that request carries, and the fetcher must return any `Set-Cookie` headers to the trusted core, which applies them. This changes the current `neoteletext` arrangement, in which the cookie store runs inside the fetcher.
- **Reason:** A compromised fetcher must see only the cookies of requests in flight, never the whole store.
- **Source:** Decision 4.
- **Verification:** Test: the fetcher process holds no cookie store; review of the message types.

### Requirement 7.6: Cookie rules
- **Statement:** The cookie store must follow `neoteletext-web`: the cookie specification draft RFC 6265bis of 30 September 2026, every cookie scoped to its host only (the `Domain` attribute ignored), `Secure` cookies never sent over plain connections, the `__Host-` and `__Secure-` prefixes enforced, expiry enforced, and `SameSite` applied to form submissions that go to another channel.
- **Reason:** The same cookie behaviour as a traditional browser, made stricter.
- **Source:** Decision 9.
- **Verification:** Test for each rule.

### Requirement 7.7: No third-party context
- **Statement:** No page may cause a request that carries credentials to an origin other than its channel's.
- **Reason:** Third-party cookies are a tracking and attack route; traditional browsers now block or partition them.
- **Source:** Decision 9.
- **Verification:** Test.

### Requirement 7.8: Caches are keyed by channel
- **Statement:** The decoder's page store, the trusted core's frame cache, the back-and-forward cache and the network cache must all be keyed by channel.
- **Reason:** Caches must not leak data between channels.
- **Source:** Decision 9.
- **Verification:** Test.

### Requirement 7.9: The webview holds no channel data
- **Statement:** The webview must hold no cookies, no site storage and no network cache of channel content. Its storage must be cleared on every start.
- **Reason:** The webview is untrusted and must not accumulate anything of value.
- **Source:** Decision 4.
- **Verification:** Test: inspect the webview's data directory after use.

### Requirement 7.10: Forms
- **Statement:** A page's form must be sent only to its declared destination, through the fetcher. A form whose destination is another channel must go through the site-change screen, with that channel's `SameSite` cookies withheld. No form value may be filled in automatically.
- **Reason:** The same protection traditional browsers give against cross-site form submission.
- **Source:** Decision 9.
- **Verification:** Test.

### Requirement 7.11: Certificate failures
- **Statement:** A Transport Layer Security certificate that fails its checks must stop the navigation, with no way for the user to proceed (decision 24).
- **Reason:** A failed certificate means the connection cannot be trusted.
- **Source:** Decision 4.
- **Verification:** Test with expired, self-signed and wrong-host certificates.

### Requirement 7.12: Local files
- **Statement:** Each local file or capture must be its own origin with no cookies, and links from it to web addresses must go through the site-change screen.
- **Reason:** A file must not borrow the identity or cookies of a channel.
- **Source:** Architecture.
- **Verification:** Test.

---

## 8. Navigation and the site-change screen

### Requirement 8.1: One navigation controller per tab
- **Statement:** The trusted core must keep one navigation controller per tab, with a flat history list, as in `neoteletext/docs/desktop-browser.md` section 6.
- **Reason:** Navigation state is security state and belongs in the trusted core.
- **Source:** Architecture; decision 24 (reuse of `neoteletext-browser`).
- **Verification:** Test.

### Requirement 8.2: User-started and page-started navigations are distinguished
- **Statement:** Navigations the user starts in the trusted strip (typed address, bookmark, preset, Back, Forward, Home) and navigations a page starts (Fastext, page catching, page links, redirects, discovery results, decoder requests) must be recorded as such and handled under their own rules.
- **Reason:** Navigations a page starts are less trusted.
- **Source:** Architecture.
- **Verification:** Test.

### Requirement 8.3: The committed address is shown
- **Statement:** The trusted strip must show the committed address until a new page commits.
- **Reason:** A pending navigation must not be presented as the current one.
- **Source:** Architecture.
- **Verification:** Test.

### Requirement 8.4: Leaving a channel needs the site-change screen
- **Statement:** Every navigation that leaves the tab's channel, other than one the user typed in the trusted strip, must show the site-change screen in the trusted strip with the destination channel and how the navigation was started, and must proceed only on the user's confirmation there.
- **Reason:** The user must always see and agree to a change of channel.
- **Source:** Decision 9; architecture.
- **Verification:** Test for each kind of page-started navigation.

### Requirement 8.5: The site-change screen cannot be faked
- **Statement:** The site-change screen must be drawn only by the trusted strip. The window host must have no way to start, show, imitate or confirm it, and a confirmation must not be accepted from a click inside the page area or from pasted input.
- **Reason:** A page image can draw anything, and the window host is untrusted.
- **Source:** Decisions 4 and 24.
- **Verification:** Test; review.

### Requirement 8.6: Cancelling returns to the committed page
- **Statement:** Cancelling the site-change screen must return the tab to its committed page unchanged.
- **Reason:** Declining must have no effect.
- **Source:** Architecture.
- **Verification:** Test.

---

## 9. Rendering and showing pages

### Requirement 9.1: Pages are rendered in the decoder
- **Statement:** Pages must be rendered by the decoder into 8-bit palette indices at the native raster, 480 by 500 or wider with side panels, with a palette of at most 32 colours.
- **Reason:** Rendering untrusted pages belongs in the process with no rights.
- **Source:** Decision 6.
- **Verification:** Test.

### Requirement 9.2: Frames are checked before encoding
- **Statement:** The trusted core must check every frame: known tab and page, dimensions within the raster limits, palette of at most 32 entries, every index within the palette.
- **Reason:** A compromised decoder can send false frames.
- **Source:** Architecture.
- **Verification:** Test with frames breaking each rule.

### Requirement 9.3: Frames are lossless indexed images
- **Statement:** The trusted core must encode each frame as an indexed Portable Network Graphics image with `neoteletext-image`'s writer. No lossy format may be used.
- **Reason:** Colours must be exact, and the webview's image decoder must see only files our encoder produced.
- **Source:** Decisions 5 and 6.
- **Verification:** Test: decoding the image gives back the exact indices and palette.

### Requirement 9.4: Unchanged frames are not sent again
- **Statement:** The trusted core must compare frames by a hash of indices and palette and must not send a frame identical to the last one sent for that page.
- **Reason:** Many pages on screen must cost only what changes.
- **Source:** Decision 7.
- **Verification:** Test.

### Requirement 9.5: Only visible pages are rendered and sent
- **Statement:** The window host must report which pages are visible, and the trusted core must request and send frames only for those pages and a bounded margin around them.
- **Reason:** A wall of hundreds of pages must cost what is visible.
- **Source:** Decision 7.
- **Verification:** Test: a wall of 800 pages with 20 visible causes frames for those 20 and the margin only.

### Requirement 9.6: Whole-frame replacement
- **Statement:** Each new frame must replace the page's whole image. The window host must decode the new image away from the screen before showing it, must drop an older frame for the same page if a newer one arrives during decoding, and must release the previous image's memory after the swap.
- **Reason:** Simple, self-contained frames, with no blank frame and no memory growth.
- **Source:** Decision 8.
- **Verification:** Test: no blank frame during rapid updates; memory stays bounded over a long run.

### Requirement 9.7: Flash without decoding again
- **Statement:** For a flashing page the trusted core must keep both phases and alternate them on the flash timer, without asking the decoder again.
- **Reason:** Flash is a palette change; repeating the decode wastes work.
- **Source:** Architecture.
- **Verification:** Test.

### Requirement 9.8: Scaling and pixel shape in the interface
- **Statement:** Pages must be scaled and their pixel shape corrected by Cascading Style Sheets in the window host, never by rendering again at other sizes.
- **Reason:** One raster per page keeps rendering simple and exact.
- **Source:** Architecture.
- **Verification:** Review.

### Requirement 9.9: Frame size is measured
- **Statement:** The size and encoding time of page images must be measured on real channels before the first release and recorded.
- **Reason:** The design assumes a few kilobytes per page; the assumption must be checked.
- **Source:** Architecture.
- **Verification:** Recorded measurement.

---

## 10. The window host and its webview

The process model is decided (decision 18, amended by decision 19). The interface is a React application running in the window host.

**These requirements are hygiene, not security boundaries.** Requirements 10.2, 10.3 and 10.4 reduce the chance that the interface is compromised. No security property relies on them: requirement 3.9 requires every property to hold with a hostile interface. Requirements 10.5 to 10.10 configure the webview so that even a compromised interface stays inside its process.

### Requirement 10.1: No teletext data reaches the webview
- **Statement:** The webview must receive pages only as Portable Network Graphics images produced by the trusted core, and page text only as plain, cleaned, bounded text. No byte received from the network may reach the webview.
- **Reason:** This keeps the webview's parsers and decoders away from attacker-controlled input.
- **Source:** Decision 4.
- **Verification:** Review of every message type sent to the window host; test that the webview makes no network request.

### Requirement 10.2: Page text is rendered only as text
- **Statement:** Every piece of text that came from a page, a channel or a server (titles, row 0, link labels, accessible page text, developer tools data) must be rendered by React as text nodes, never as markup. No component may build markup from strings.
- **Reason:** React escapes text nodes, so text rendered this way cannot become markup or script. This replaces the earlier requirement for markup generated in Rust (decision 19).
- **Source:** Decisions 4 and 19.
- **Verification:** Test: hostile page text containing markup, script and entity sequences appears literally on screen; review.

### Requirement 10.3: Run-time JavaScript dependencies are a fixed, approved list
- **Statement:** The JavaScript that runs in the webview must consist of the project's own TypeScript and these packages only: React, React DOM and the Tauri interface package (`react`, `react-dom`, `@tauri-apps/api`). Adding any other run-time package needs the owner's approval and a recorded security review. All packages must be bundled into the application at build time from the committed lockfile; nothing may be loaded from a network at run time. Package installation scripts must stay blocked. `npm audit` must pass. Build tools may be third-party but must contribute no code to what runs beyond these packages.
- **Reason:** Third-party packages are the most likely route to compromising the interface. Decision 19 accepts React; keeping the list fixed and audited keeps that exposure small, and the window host's isolation keeps its consequences small.
- **Source:** Decisions 4 and 19.
- **Verification:** Check: the run-time dependency list in `package.json` equals the approved list; the built bundle contains no package outside it; `npm audit` passes; review of every lockfile change.

### Requirement 10.4: Forbidden interfaces
- **Statement:** The interface code must not use React's `dangerouslySetInnerHTML`, or `innerHTML`, `outerHTML`, `insertAdjacentHTML`, `document.write`, `eval`, `Function`, or string arguments to `setTimeout` and `setInterval`.
- **Reason:** Each turns a string into markup or code.
- **Source:** Decision 4.
- **Verification:** Check: a lint rule fails the build on any of them.

### Requirement 10.5: Content Security Policy
- **Statement:** The webview's Content Security Policy must be no looser than: `default-src 'none'; script-src 'self'; style-src 'self'; img-src 'self' blob:; connect-src ipc: http://ipc.localhost; font-src 'self'; object-src 'none'; base-uri 'none'; frame-src 'none'; form-action 'none'`, with Trusted Types required where the engine supports them. No `'unsafe-inline'` and no `'unsafe-eval'`. React's `style` property is applied through each element's style object, which this policy permits; components must not render `<style>` elements or `style` attributes as markup.
- **Reason:** Even an injected fragment of markup must not be able to load or run anything.
- **Source:** Decision 4.
- **Verification:** Check: a test reads the configuration and fails if the policy is looser.

### Requirement 10.6: Navigation of the webview is locked
- **Statement:** The webview must refuse every navigation to anything other than the application's own pages, and must refuse every new window.
- **Reason:** Teletext links are intents, never webview navigations; the webview must never show remote content.
- **Source:** Decision 4.
- **Verification:** Test: navigation and new-window attempts are refused.

### Requirement 10.7: Capabilities name webview labels only
- **Statement:** Every Tauri capability must name webview labels, never window labels, must grant only the application's own commands and the core window and event permissions needed, must not grant any file system, shell, process, opener or remote-access plugin, and must not allow creating webviews or windows. Every command must also check its caller's webview label.
- **Reason:** Tauri grants a command when either the window label or the webview label matches, so naming a window would cover every webview inside it.
- **Source:** Prototype finding.
- **Verification:** Check: a test reads the capability files and fails on any window label, any plugin outside the allow-list, or any webview-creation permission.

### Requirement 10.8: No inspector or remote debugging in release builds
- **Statement:** Release builds must not enable the webview inspector, remote debugging, or the global Tauri object (`withGlobalTauri`).
- **Reason:** Each gives control of the interface to anything that reaches it.
- **Source:** Decision 4; prototype finding (Chromium Embedded Framework trial).
- **Verification:** Check on the release build's configuration.

### Requirement 10.9: Dropped files go to the trusted core
- **Statement:** Files dropped on a window must be handled by the trusted core, which opens them through a decoder. The webview must never read them.
- **Reason:** A dropped file is untrusted input.
- **Source:** Architecture.
- **Verification:** Test.

### Requirement 10.10: Linux webview version check
- **Statement:** On Linux the application must check the WebKitGTK version at start-up and warn when it is older than a recorded release with known security fixes.
- **Reason:** Linux distributions often lag behind upstream security fixes.
- **Source:** Architecture.
- **Verification:** Test with a mocked version.

### Requirement 10.11: Page code cannot use the network
- **Statement:** Page code must be unable to reach any network, through: (1) the Content Security Policy of requirement 10.5; (2) WebRTC's interfaces (`RTCPeerConnection`, `webkitRTCPeerConnection`, `RTCDataChannel`, `RTCIceCandidate`, `RTCSessionDescription`, `RTCRtpSender`, `RTCRtpReceiver`, `RTCRtpTransceiver`) removed in every frame before any page script runs, because the Content Security Policy does not govern WebRTC; (3) the webview's own switches for media capture, geolocation, service workers and prefetching, wherever the platform offers them; (4) the platform's sandbox of the web content process, which on macOS allows it no network sockets. Where the operating system can also deny the window host the network without breaking the webview, it must: on Linux it can, with an empty network namespace that WebKitGTK's processes inherit (`docs/proofs.md` section 6.1; requirement 3.10); on macOS it cannot (section 2); on Windows it is not yet proven.
- **Reason:** On macOS, WKWebView renders nothing without the network entitlement, and with it the App Sandbox does not restrict destinations; the protection must therefore come from the layers that do work, measured in Proof A.
- **Source:** Decisions 4, 20 and 27.
- **Verification:** Test on each platform, inside the real webview: fetch, XMLHttpRequest, WebSocket, Server-Sent Events, beacon, WebRTC (directly and through a blank iframe), image, font and stylesheet loads, prefetch, iframes and navigation are all attempted, and none reaches a listening test server. A control run without the protections must show that the same probe does reach the server.

---

## 11. The trusted strip

### Requirement 11.1: The trusted strip shares the webview's window and is out of page code's reach
- **Statement:** Every window must have a trusted strip, drawn natively by the window host in the same window as the webview but outside the webview's area, showing only the state the trusted core sends. The strip must never be a window of its own. Its input must reach the trusted core as strip input, a message kind that page intents can never produce.
- **Reason:** A strip in a window of its own can be covered by any process's window, even a sandboxed one; a strip in the webview's window cannot be reached by page code, and on macOS the web content process is denied the window server (`docs/proofs.md` section 3). On Linux the web content process is given the display even inside WebKitGTK's sandbox, so there the strip is out of page code's reach but not out of a compromised content process's (section 6.9). On Linux the strip is therefore drawn by the host of the window host's nested display, outside it, by T42 Browser's own display host (decisions 29 and 30, requirement 3.11).
- **Source:** Decisions 4, 18 and 27.
- **Verification:** Test: a page that draws fixed elements above the viewport at the highest stacking order, opens windows and requests full screen leaves the strip untouched; the hostile interface harness cannot produce strip input; review.

### Requirement 11.2: What the trusted strip holds
- **Statement:** The trusted strip must hold the address field, the channel's identity, whether the channel is encrypted, Back, Forward, Reload and Home, the site-change screen, and every setting that affects security.
- **Reason:** Everything that decides or shows trust is in one place the attacker cannot reach.
- **Source:** Decision 4.
- **Verification:** Review.

### Requirement 11.3: The address field
- **Statement:** In display mode the address field must show the address as formatted by the address display rules (scheme, emphasised host, internationalised names shown safely). In edit mode it must be a native text field whose text is sent, on Enter, to the trusted core's strict address parser, which navigates or returns an error shown in the strip.
- **Reason:** The address is the user's main security indicator.
- **Source:** Decision 9.
- **Verification:** Test with deceptive and internationalised addresses.

### Requirement 11.4: Always visible
- **Statement:** The trusted strip must be visible in every layout. Where a full-screen or zoomed layout hides it, it must reappear on any interaction.
- **Reason:** A hidden indicator protects nothing.
- **Source:** Decisions 4 and 24 (the address field is always visible).
- **Verification:** Review of every layout.

### Requirement 11.5: Page text is cleaned before display
- **Statement:** Text taken from pages (row 0, titles, link labels, accessible page text) must have control characters, bidirectional overrides and zero-width characters removed and be cut to a fixed length before it is shown anywhere.
- **Reason:** These characters can make text read as something it is not.
- **Source:** Decision 4.
- **Verification:** Test.

---

## 12. Shell features

### Requirement 12.1: Tabs
- **Statement:** The window host must draw a tab strip with drag to reorder, close buttons, overflow scrolling, and a live thumbnail of the tab's page on hover. The list and order of tabs, their creation and closing, and session restore must be decided by the trusted core.
- **Reason:** The tab strip is decorative; tab state is trusted.
- **Source:** Decision 3.
- **Verification:** Test.

### Requirement 12.2: Page walls and transitions
- **Statement:** The window host must show pages as grids, lists and stacks of images, with Cascading Style Sheets transforms and view transitions for zooming, subpage stacks and magazine overviews, and must respect the reduced-motion preference.
- **Reason:** The animated navigation of many pages at once is the purpose of the interface.
- **Source:** Decisions 3 and 7.
- **Verification:** Review; test with reduced motion.

### Requirement 12.3: Clicks on pages are cell clicks
- **Statement:** A click on a page image must be converted to a cell (column and row) from the pointer position in the image's own coordinates, and sent as an intent. The trusted core must decide, with the decoder, whether the cell is a link.
- **Reason:** A page image can draw fake buttons; the click must mean only "this cell".
- **Source:** Decision 4; prototype finding on pointer coordinates.
- **Verification:** Test under every transform the interface uses.

### Requirement 12.4: Keys work in the tab
- **Statement:** With the page area focused, page-number keys, Fastext colour keys, reveal, hold and mix must work. The window host must forward normalised key presses to the trusted core's input router, which maps them to functions as in `neoteletext/docs/hmi.md`. A key must only select among pages already held (requirement 4.7), and a page not yet received must show that it is being searched for, except on channels with only separate page files, where selecting a page not yet held fetches it (decision 23). A key that would leave the channel must go through the site-change screen.
- **Reason:** Teletext is driven by these keys, and they must work where the user is looking. Requirement 4.7 makes their source irrelevant to security.
- **Source:** Decision 22.
- **Verification:** Test each key with the page area focused; test with forged keys from the hostile interface harness that no request results.

### Requirement 12.5: Menus and shortcuts
- **Statement:** The trusted core must provide a native menu bar (File, Edit, View, History, Window) carrying every shortcut of `neoteletext/docs/desktop-browser.md` section 10. Menu items and buttons must send the same intents.
- **Reason:** Shortcuts must work whatever has focus.
- **Source:** Prototype finding.
- **Verification:** Test each shortcut with the page area focused and with the address field focused.

### Requirement 12.6: Internal pages
- **Statement:** Bookmarks, history, presets and settings must be internal pages drawn from data the trusted core supplies, except settings that affect security, which must be changed only in the trusted strip.
- **Reason:** Security settings must not be reachable from the untrusted interface.
- **Source:** Decision 4.
- **Verification:** Test.

### Requirement 12.7: Developer tools
- **Statement:** A docked developer tools panel must show, for the current channel: the packet log, the page store, network requests (address, role, declared media type, status, size, time), cookies and cached resources, frame statistics (rate, image size, encoding time, visibility) and engine warnings. All of it must come from the trusted core and be shown as text.
- **Reason:** Diagnosis without exposing the trusted core.
- **Source:** Architecture.
- **Verification:** Test that hostile packet contents appear as text.

### Requirement 12.8: Windows
- **Statement:** Each window must have one user interface view and one trusted strip. Moving a tab between windows must be a change of state in the trusted core. Frames for a page must go only to the window showing it.
- **Reason:** Windows share one trusted state.
- **Source:** Architecture.
- **Verification:** Test.

### Requirement 12.9: Accessibility
- **Statement:** The shell must be usable with a screen reader. Each page image must carry an accessible label holding the page's cleaned text.
- **Reason:** Teletext must be readable without sight.
- **Source:** Architecture.
- **Verification:** Test with VoiceOver, Narrator and Orca on the desktops.
- **Linux:** The window host has no accessibility bus (requirement 3.10). The display host must publish the window's accessibility tree itself, from state the trusted core sends: the strip and its address, and each page image labelled with its cleaned text (decision 32). Accessibility requests reach the display host, never the window host, and the display host must act on them only as it would on the user's own input in the strip.
- **Status note:** The display host's smallest form published a correct tree, followed the user's focus and acted on a screen reader's focus request; Orca read the tree on starting but did not follow later moves in the headless test, which is not explained (`docs/proofs.md` section 6.16).

---

## 13. The application update mechanism

### Requirement 13.1: No failure leaves the application unusable
- **Statement:** No failure at any point of an update (lost connection, full disk, power loss, crash, malicious package) may leave the user without a complete, working version of the application.
- **Reason:** The owner requires an update mechanism with no scope for failure.
- **Source:** Decision 15.
- **Verification:** The tests of requirement 13.12.

### Requirement 13.2: The running version is never modified
- **Statement:** A new version must be installed beside the running version, in its own complete directory. The running version's files must never be written.
- **Reason:** A failed install must leave the old version untouched.
- **Source:** Decision 15.
- **Verification:** Test: file hashes of the running version before and after an update.

### Requirement 13.3: Verify before switching
- **Statement:** A new version must not be switched to until it is fully downloaded, verified against The Update Framework metadata and against the platform's code signature, and installed.
- **Reason:** Nothing unverified may ever run.
- **Source:** Decisions 15 and 16.
- **Verification:** Test with every kind of bad package.

### Requirement 13.4: The switch is atomic
- **Statement:** Switching versions must be one atomic file system operation: the rename of a pointer.
- **Reason:** After a power failure the pointer must name a complete version, old or new.
- **Source:** Decision 15.
- **Verification:** Test with simulated power loss.

### Requirement 13.5: Health check and automatic rollback
- **Statement:** The previous version must stay installed until the new version has started and passed its health check. If the new version fails the check or fails to start twice, the launcher must switch back on the next start.
- **Reason:** A version that passes verification can still fail on a particular machine.
- **Source:** Decision 15.
- **Verification:** Test with a deliberately broken version.

### Requirement 13.6: Profiles are never migrated in place
- **Statement:** A version that needs a new profile format must copy the profile and migrate the copy, leaving the old profile untouched.
- **Reason:** A rolled-back version must find its own data intact; data migration is the commonest real cause of unusable applications.
- **Source:** Decision 15.
- **Verification:** Test: migrate, roll back, and use the old version.

### Requirement 13.7: The launcher
- **Statement:** A small launcher must read the pointer, start the chosen version, count failed starts and perform rollback. It must be small enough to review completely and must change rarely. It must be updated only by an atomic exchange of directories (`renamex_np` with `RENAME_SWAP` on macOS, `renameat2` with `RENAME_EXCHANGE` on Linux) or, on Windows, by a fixed stub that never changes choosing between two launcher copies.
- **Reason:** The launcher is what recovers from a bad update; it must itself be safe to update.
- **Source:** Decision 15.
- **Verification:** Review of the whole launcher; the tests of requirement 13.12 applied to launcher updates.

### Requirement 13.8: The Update Framework
- **Statement:** Updates must be verified with The Update Framework: separate root, targets, snapshot and timestamp roles; a threshold of root keys; expiring metadata; version numbers that only increase. The root public keys must be built into the launcher. The implementation must be `tough` (decision 24).
- **Reason:** It survives a compromised update server and a stolen key.
- **Source:** Decision 16.
- **Verification:** Test with expired metadata, an older version, a wrong signature, a below-threshold root and a replaced root.

### Requirement 13.9: Key custody
- **Statement:** The root and targets keys must be kept offline. The root keys must be three hardware security keys held by three separate people, one of them kept in a safe, with a threshold of two (decision 24).
- **Reason:** Online keys can be stolen from a compromised machine.
- **Source:** Decision 16.
- **Verification:** Review of the key procedure, recorded.

### Requirement 13.10: One launcher scheme on every desktop
- **Statement:** macOS, Windows and Linux must use the launcher scheme, so that The Update Framework verification is on every update path. Operating-system package managers that would bypass it must not be the update path (decision 24).
- **Reason:** A second update path would be a second, weaker way in.
- **Source:** Decision 16.
- **Verification:** Review.

### Requirement 13.11: Hosting and the updater
- **Statement:** Updates must be served as static files from `update.neoteletext.com`, a subdomain used for nothing else (decision 25), over Transport Layer Security. The updater must be the only process allowed to contact that subdomain and must contact nothing else.
- **Reason:** The host is untrusted by design; the updater must not widen the browser's network access.
- **Source:** Decision 17.
- **Verification:** Test: the updater's network access is limited to the subdomain; the fetcher cannot reach it.

### Requirement 13.12: Update tests
- **Statement:** On every desktop platform, automated tests must show that: killing the updater at every step and between every file write leaves the application able to start; writes cut off part-way are never used; corrupt, truncated, wrongly signed, expired, older and mismatched packages are refused; a full disk fails cleanly; a broken new version is rolled back; a migrated profile leaves the old profile usable.
- **Reason:** "No scope for failure" must be proved, not assumed.
- **Source:** Decision 15.
- **Verification:** The tests themselves, run in the full check.

### Requirement 13.13: Phones use their stores
- **Statement:** On Android and iOS updates must come only from the application stores. The application must contain no updater there.
- **Reason:** Those platforms forbid replacing an application's own code, and their stores install atomically.
- **Source:** Architecture.
- **Verification:** Review.

---

## 14. Build, dependencies and verification

### Requirement 14.1: One check script and hooks
- **Statement:** One local check script must run every check in this document. Git hooks must run the fast checks before each commit and the full checks before each push, and must not be bypassed.
- **Reason:** Requirements that are not checked drift.
- **Source:** Owner's baseline instructions.
- **Verification:** Review of the hooks.

### Requirement 14.2: Strict compilation and linting
- **Statement:** Rust code must build with `unsafe` code forbidden outside a named interface crate, the Clippy lint group `pedantic` enabled and warnings treated as errors. TypeScript must build in strict mode with the linter's strict rules and no warnings.
- **Reason:** Warnings are faults found early.
- **Source:** Owner's baseline instructions.
- **Verification:** Check.

### Requirement 14.3: Dependencies
- **Statement:** Lockfiles must be committed and their changes reviewed. `cargo-deny` and `cargo audit` must pass. Package installation scripts must stay blocked. Every new library needs the owner's approval.
- **Reason:** The supply chain is a route to compromise.
- **Source:** Decision 4; owner's baseline instructions.
- **Verification:** Check.

### Requirement 14.4: Runtime attribution test before any runtime change
- **Statement:** Before any change of Tauri version or webview runtime, a test must show that every command is attributed to the webview that called it, with two or more windows open.
- **Reason:** The Chromium Embedded Framework trial attributed calls to the wrong webview.
- **Source:** Prototype finding.
- **Verification:** The test itself.

### Requirement 14.5: Spoofing review
- **Statement:** Every new layout, zoom level or full-screen mode must be reviewed to confirm that the trusted strip remains visible and that nothing in the page area can be mistaken for it, and the review must be recorded.
- **Reason:** An interface built for animation can hide or imitate security indicators.
- **Source:** Decision 4.
- **Verification:** Recorded review.

---

## 15. Platforms

### Requirement 15.1: Desktop platforms
- **Statement:** The application must run on macOS, Windows and Linux with every requirement in this document met.
- **Reason:** The first version is desktop only (decision 24).
- **Source:** Architecture.
- **Verification:** The full check on each platform.

### Requirement 15.2: Phones (after the first version)
- **Statement:** When phones are added (decision 24: after the first version), Android must run the decoder in an isolated service process inside `wasmi`, and iOS must run the decoder inside `wasmi` in the application process. Every requirement not tied to process separation must still be met.
- **Reason:** Phones limit process separation; the WebAssembly wall remains.
- **Source:** Architecture.
- **Verification:** The full check on each phone platform.
