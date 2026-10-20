# NeoTeletext: architecture

| | |
|---|---|
| **Status** | Proposed. Nothing in this document is implemented except where it says "prototype". |
| **Date** | 15 October 2026 |
| **Companion documents** | [`requirements.md`](requirements.md) states every requirement in full, numbered, with its reason and how it is verified. [`security-analysis.md`](security-analysis.md) analyses this architecture's security. |
| **Engine documents** | The teletext engine's own design lives in `~/devel/neoteletext-engine`: `ARCHITECTURE.md`, `docs/desktop-browser.md`, `docs/channels.md`, `docs/web-discovery-security.md`, `docs/font-security.md`. This document refers to them by path. |

**Abbreviations.** This project avoids abbreviations, because they hide meaning. Where a technical term is normally known by its initials, the full name is written out, followed by the initials in brackets only when the initials are a literal identifier in code, configuration or a standard's title. Section 18 is a glossary of every technical term used.

---

## Contents

1. What NeoTeletext is
2. The owner's decisions so far
3. Concepts and terms
4. Design principles
5. The teletext engine and what is reused
6. Processes and isolation
7. Network access
8. Content detection
9. The security model of a tab
10. How a page reaches the screen
11. The browser shell
12. The application update mechanism
13. Platforms
14. Code layout
15. The prototype and what it taught
16. The Tauri 3 and Chromium Embedded Framework trial
17. Decisions still open
18. Glossary

---

## 1. What NeoTeletext is

NeoTeletext is a desktop and phone application for browsing teletext.

- **Teletext** is the page-based information service defined by the European Telecommunications Standards Institute standard EN 300 706, "Enhanced Teletext specification". A copy is at `~/devel/teletext-reference/teletext-standards/en_300706v010201o.pdf`.
- **Every tab is one teletext channel.** A teletext channel is one teletext service. The standard lets one service carry many pages: eight magazines, numbered 1 to 8, each with pages numbered 00 to FF in hexadecimal, and each page with subpages.
- **Every image on screen is one teletext page,** always drawn at the native teletext raster size and then scaled by the interface.
- **The user enters a web address.** The server at that address returns teletext data. The application fetches it, recognises it, decodes it and renders the pages in Rust.
- **The interface shows many pages at once and animates between them.** This is the main reason for choosing a HyperText Markup Language and Cascading Style Sheets interface over the engine's existing egui interface.
- **Security is paramount.** The owner has stated that there are no circumstances under which being hacked is acceptable. Every design choice in this document is made in that light. Where a choice trades capability against security, security wins.

---

## 2. The owner's decisions so far

Each decision below was made by the owner on the date given. Requirements in `requirements.md` cite the decision they come from.

| Number | Date | Decision |
|---|---|---|
| Decision 1 | 15 October 2026 | The teletext engine is `neoteletext`, kept as it is. Its egui shell (`neoteletext-gui` and the browser wrapper described in `neoteletext/docs/desktop-browser.md`) is to be replaced by a Tauri shell. |
| Decision 2 | 15 October 2026 | Tauri version 2. The Tauri 3 alpha with the Chromium Embedded Framework runtime was tried and stopped (section 16). Its work is kept on branch `tauri3`. It is not reported upstream. |
| Decision 3 | 15 October 2026 | The interface is drawn with HyperText Markup Language and Cascading Style Sheets, because it allows a richly animated interface for navigating many teletext pages at once. |
| Decision 4 | 15 October 2026 | Security is paramount. Being hacked is not acceptable under any circumstances. |
| Decision 5 | 15 October 2026 | Pages are shown as images. The viewer is always the NeoTeletext application, never an ordinary web browser, so images travel inside the application and never over a local network port. |
| Decision 6 | 15 October 2026 | The page palette is 32 colours: the Colour Map of teletext Presentation Level 2.5, which is four Colour Look-Up Tables of eight colours each. Colours must be reproduced exactly, so no lossy image format is used. |
| Decision 7 | 15 October 2026 | Many pages are shown on screen at once. |
| Decision 8 | 15 October 2026 | A new frame of a page replaces the whole image. Changed-rectangle patches are added only if measurement shows they are needed. |
| Decision 9 | 15 October 2026 | A tab must have the same security behaviour and properties as a tab in a traditional web browser: origins, cookies, isolation. |
| Decision 10 | 15 October 2026 | Plain HyperText Transfer Protocol, without Transport Layer Security, is allowed. |
| Decision 11 | 15 October 2026 | Two tabs on the same channel share cookies and a decoder, as same-site tabs do in traditional browsers. |
| Decision 12 | 15 October 2026 | The application loads only teletext resources from the network. It connects with the HyperText Transfer Protocol and WebSocket, and supports the standard server push technologies. |
| Decision 13 | 15 October 2026 | The media type a server declares is never used to accept or reject a resource, because teletext files may be served with any media type. The content decides. |
| Decision 14 | 15 October 2026 | Channel descriptors, contents files, fonts and HyperText Markup Language discovery must all be supported, as parts of the teletext service. |
| Decision 15 | 15 October 2026 | The application must have an update mechanism with no scope for failure or for leaving the application unusable. |
| Decision 16 | 15 October 2026 | Update metadata uses the most secure option: The Update Framework, with threshold signatures and expiring metadata. |
| Decision 17 | 15 October 2026 | Updates are hosted on a dedicated subdomain (named by decision 25). |
| Decision 18 | 15 October 2026 | The hardened process model of section 6 is adopted: a trusted core with no web engine; the interface drawn with web technology runs in its own window host with no rights and no secrets; every security indicator and decision is in a native trusted strip. As first adopted it also required markup generated in Rust and a single hand-written script module; decision 19 replaced that with React. |
| Decision 19 | 15 October 2026 | The interface is a HyperText Markup Language, Cascading Style Sheets and TypeScript application running in the webview, inside the untrusted window host, with React as its framework. Blitz is rejected, because it has no JavaScript engine and cannot run such an application. Decision 18 is amended accordingly: React and its run-time dependencies are allowed in the window host; process isolation, the native trusted strip and every other part of decision 18 stand. |
| Decision 20 | 15 October 2026 | Whether the interface is written with React or with plain HyperText Markup Language, Cascading Style Sheets and JavaScript must make no difference to security. Every security property must hold even if the window host runs code written by an attacker. |
| Decision 21 | 15 October 2026 | Superseded the same day by decision 22. It had closed the covert channel between channels by accepting page requests only from trusted input, which would have stopped page-number and Fastext keys from working in the tab. |
| Decision 22 | 15 October 2026 | Fetching never depends on viewing. A channel's pages are fetched in an order fixed by the channel's own data, and refreshed on a fixed schedule or by the channel's push feed, never according to which page is shown, selected or requested. Page-number and Fastext keys work in the tab and only select among pages already held, so no key and no interface request ever causes, delays, reorders or prioritises a network request. Navigations to other channels still go through the trusted strip. This replaces `neoteletext`'s fetching of a page's own file when it is viewed and its prefetching of Fastext targets and neighbouring pages (its requirement REQ-NAV-004) in NeoTeletext. |
| Decision 23 | 15 October 2026 | Channels that publish only separate page files, with no bundle (`index.t42`) and no contents file (`contents.txt`), have each page fetched when it is selected, as any browser fetches what is asked for. This is a knowingly accepted exception to decision 20 for those channels only: if the window host were compromised, its forged key presses could cause page requests whose pattern carries information to another channel's server, and those channels' servers can see which pages are read. Bundled channels and channels with a contents file keep decision 22 in full. |
| Decision 24 | 15 October 2026 | Every recommendation for open decisions 1 and 3 to 13 is accepted: (1) `neoteletext-browser` is reused for navigation, history, site-change and address rules; (3) the site-change screen is native, in the trusted strip only; (4) the address field is always visible; (5) both conventional tabs and a zoomable overview of tabs, magazines and pages; (6) a certificate that fails its checks is a hard stop with no way to proceed; (7) private windows come after the first version; (8) no per-tab containers; (9) page images, with a palette-index canvas only if measurement shows images limit very large walls; (10) `tough` implements The Update Framework; (11) one launcher scheme on every desktop; (12) desktop first, phones later; (13) three hardware security keys for the update root, held by three separate people, one kept in a safe. |
| Decision 25 | 15 October 2026 | Updates are hosted at `update.neoteletext.com`, a subdomain used for nothing else. |
| Decision 26 | 15 October 2026 | Every limit on fetching, refreshing and input is defined once, in neoteletext, and NeoTeletext uses neoteletext's constants. The fetching policy of decisions 22 and 23 is implemented in neoteletext as well, not in NeoTeletext. What NeoTeletext needs from neoteletext is listed in section 5.1 and in `neoteletext/docs/neoteletext-browser-open-questions.md`. |
| Decision 27 | 15 October 2026 | Proofs A and B (`docs/proofs.md`) showed that on macOS a webview cannot be denied the network by the operating system without rendering nothing, and that a strip in a separate process's window can be covered by any other window. The interface process of decision 18 is therefore replaced by a **window host**: a process that owns the window, hosts the system webview and draws the native trusted strip in that same window. It holds the network entitlement only because the webview requires it, never uses it, and holds no secrets. The trusted core stays a separate process, holding every secret and making every decision. Page code is confined by the webview's own content process sandbox, the Content Security Policy and the removal of WebRTC in every frame. |
| Decision 28 | 15 October 2026 | On Linux the window host is confined from outside (option 1 of `docs/proofs.md` section 6.7). It is started in an empty network namespace, with private `/run` and `/tmp` that hold only the socket of the display in use, under a Landlock rule set that grants no home directory and no data directory. WebKitGTK's processes inherit all three. WebKitGTK's own sandbox is not turned on, so `wry` is not patched. The private directories were added after measurement showed that Landlock alone left the user's session bus reachable, and through it a way to run programs outside the confinement (`docs/proofs.md` section 6.8). On Linux the trusted strip holds against page code but not against a compromised web content process, which keeps the display; that is recorded as a remaining risk, and what to do about the display is open decision 19. |
| Decision 29 | 15 October 2026 | Security is critical, so on Linux the window host never receives the user's display. It draws on a nested display of its own, hosted by a process outside its confinement, on every Linux session, X11 or Wayland (option 1 of `docs/proofs.md` section 6.10). No session is supported by warning instead. Measured with cage as the nested compositor: the confined window host reached none of the user's display sockets or local services, and could not type into the user's terminal (`docs/proofs.md` section 6.11). The same measurement showed that a stock compositor still lets a compromised content process forge strip input, so the strip on Linux must be drawn by the nested display's host. How was open decision 20, closed by decision 30. |
| Decision 30 | 15 October 2026 | On Linux the nested display of decision 29 is hosted by **NeoTeletext's own display host** (option 1 of `docs/proofs.md` section 6.13), not by a stock or patched compositor. It is a small Wayland compositor, run outside the window host's confinement as a client of the user's desktop. It offers the window host only core window protocols: surfaces, shared memory and graphics buffers, and the desktop shell's windows. It offers no input injection, no screen copy, no clipboard access without focus and no surfaces above other windows. It takes input only from the user's real devices. It draws the trusted strip natively, outside the nested display, and sends strip input straight to the trusted core. Which compositor library it is built on was open decision 21, closed by decision 31. |
| Decision 31 | 15 October 2026 | The display host of decision 30 is written in Rust, on Smithay (version 0.7.0 in the proof). Its smallest form, `prototypes/spikes/display-host`, passed every test of `docs/proofs.md` section 6.14 under both an X11 and a Wayland desktop. |
| Decision 32 | 15 October 2026 | On Linux, input methods and screen readers are handled by the display host, outside the window host's confinement; nothing new enters the confinement. The user's input method talks only to the display host, which serves the text-input protocol itself and delivers only composed text to the window host; it never offers the input-method protocol. The display host publishes the window's accessibility tree to the user's screen reader with AccessKit, built from state the trusted core sends: the strip, and each page image labelled with its cleaned text. The window host never reaches an accessibility bus. Measured in `docs/proofs.md` section 6.16. |

---

## 3. Concepts and terms

| Term | Meaning in this project |
|---|---|
| **Teletext page** | One screen of teletext: 40 columns by 25 rows of character cells, row 0 being the header. Identified by magazine (1 to 8), page number (00 to FF) and subcode. |
| **Subpage** | One of several versions of a page carrying the same page number, distinguished by subcode, usually shown in rotation. |
| **Channel** | One teletext service reached over the web. In `neoteletext` decision E.70 a channel is identified by its **origin** and its **folder path** (`neoteletext/docs/channels.md`). A channel is the unit of trust and isolation in this design: it plays the part a "site" plays in a traditional browser. |
| **Origin** | The combination of scheme, host and port of an address, as in traditional browsers. |
| **Tab** | One view onto one channel at a time, with its own history. |
| **Trusted core** | The process that holds everything the user trusts: which channel each tab shows, history, bookmarks, cookies, settings. It contains no web engine. |
| **Display host** | On Linux only: NeoTeletext's own small Wayland compositor, written in Rust on Smithay (decision 31), outside the window host's confinement. It hosts the window host's nested display, draws the trusted strip natively beside it, and sends strip input to the trusted core (decisions 29 and 30). It is also the window's side of the user's input method and the window's publisher to the user's screen reader (decision 32). |
| **Window host** | The process that owns each window, hosts Tauri and the system webview, and draws the native trusted strip in the same window (decision 27); on Linux the display host draws the strip instead (decision 30). It holds no secrets and makes no decision; it renders the state the trusted core sends and passes on what the user did. The webview's page code runs in the webview's own content process, outside the window host. |
| **Web content process** | The system webview's own process in which page code runs, sandboxed by the platform. On macOS its sandbox denies everything by default, including the window server and network sockets (`docs/proofs.md` section 3.2). |
| **Trusted strip** | A small native area at the top of each window, drawn natively by the window host from state the trusted core sends, in the same window as the webview. It shows the address and security state of the current tab and carries every security decision. Page code cannot draw over it or read it (`docs/proofs.md` section 3.2). |
| **Fetcher** | The process that holds network connections and Transport Layer Security sessions, and runs the strict web protocol readers. |
| **Decoder** | The process that recognises, decodes and renders one channel's teletext. It has no network access, no file access and no secrets, and runs inside the WebAssembly interpreter `wasmi`. |
| **Frame** | One rendered page: palette indices plus its palette, produced by the decoder. |
| **Resource role** | The purpose for which the application requested a resource: teletext data, channel descriptor, contents file, font, or discovery. A response is read only by the reader for the role it was requested for. |
| **Site-change screen** | The screen shown when a tab is about to leave its channel for another. The engine design calls it the "tuning screen". |

---

## 4. Design principles

1. **Assume every part can be compromised, and make each compromise worthless.** No architecture can promise never to be hacked. This one is arranged so that compromising any single component gains an attacker nothing of value: no secrets, no navigation without the user, no network, no files.
2. **Untrusted bytes are read only in processes that have no rights.** Everything a server sends is read by strict, bounded readers in the fetcher, the decoder or a confined font helper. The trusted core routes bytes and re-checks results as typed data; it never parses network formats itself.
3. **The interface drawn with web technology is untrusted, and its code is irrelevant to security** (decision 20). Every security property holds even if the window host runs an attacker's code. React or plain markup and script is an engineering choice, not a security one. The system webview is a very large engine. Its page code runs in the webview's own sandboxed content process; the window host that holds the webview has no secrets and makes no decision (decision 27). Security decisions and security indicators live in the trusted strip, which page code cannot reach.
4. **The webview never receives teletext data.** Pages reach it only as Portable Network Graphics images that the trusted core encoded itself from palette indices, and as plain text that the trusted core extracted, cleaned and bounded.
5. **Every rule lives in Rust, once.** Navigation rules, address display rules, cookie rules and site-change rules are written once in Rust and tested once, for every platform.
6. **Nothing is downloaded and executed.** No page, channel or server ever supplies code. The only code that runs is the code signed and shipped in the application.
7. **Updates can fail without harm.** Every failure during an update leaves a complete, working version of the application.

---

## 5. The teletext engine and what is reused

The engine is `neoteletext` (`~/devel/neoteletext-engine`), a Rust implementation of World System Teletext, European Telecommunications Standards Institute EN 300 706, Presentation Levels 1.0, 1.5, 2.5 and 3.5. Its core library is `no_std` (it does not depend on the Rust standard library) and performs no input or output of its own: it is fed bytes and returns results.

**The engine does not depend on egui.** This was checked on 15 October 2026 with `cargo tree` on every crate. Only `neoteletext-gui`, the egui application, depends on egui or any graphics toolkit.

| Crate | What it does | Reused by NeoTeletext |
|---|---|---|
| `neoteletext-core` | Packet decoding, page store, Presentation Levels 1.0 to 3.5, rendering a page to palette indices, the input router | Yes, inside the decoder |
| `neoteletext-formats` | Readers for T42 packet files, Teletext Text Interchange page files, transport streams and other formats; content-based format detection; the strict Glyph Bitmap Distribution Format font reader | Yes, inside the decoder and the font helper |
| `neoteletext-slicer` | Recovering teletext from sampled video | Yes, for captures |
| `neoteletext-web` | The strict address parser, origins, the HyperText Transfer Protocol version 1.1, WebSocket, Server-Sent Events, the cookie store, channel descriptors, contents files, discovery, forms, push subscriptions, the zxnet live feed | Yes, in the fetcher (protocol readers) and in the trusted core (cookie store, address parser) |
| `neoteletext-image` | Palette indices to colour formats, Portable Network Graphics writer | Yes, in the trusted core |
| `neoteletext-app` | Starting and sandboxing processes, the framed pipe protocol between processes | Yes, in the trusted core |
| `neoteletext-browser` | Navigation, history, the site-change screen's rules, address display, as platform-free logic | Yes, in the trusted core (decision 24) |
| `neoteletext-cli` | The `neottx` command-line program | Not part of the application |
| `neoteletext-gui` | The egui interface | **Replaced** by the NeoTeletext shell |

---

### 5.1 Changes required in neoteletext

NeoTeletext defines no limit and no fetching behaviour of its own (decision 26). Everything below is work in neoteletext, recorded with its questions in `neoteletext/docs/neoteletext-browser-open-questions.md`. That document also records where NeoTeletext's decisions conflict with decisions already made in neoteletext, for the owner to resolve there.

| Change | Where in neoteletext | NeoTeletext decisions and requirements |
|---|---|---|
| A fetching policy in which fetching never depends on viewing: whole channel in an order fixed by the channel's data, refreshes only by schedule or push, no prefetch by viewing, and on-demand fetching only for channels with only separate page files | `neoteletext-web`, `neoteletext-browser` | Decisions 22 and 23; requirements 4.7, 5.15 and 5.16 |
| A grace period during which a closed tab's channel keeps refreshing | `neoteletext-browser` | Requirement 5.17 |
| A threshold of malformed packets after which a live feed is closed | Live input handling in `neoteletext-app` | Requirement 6.5 |
| A default refresh schedule for a channel that declares none, if none exists | `neoteletext-web`, beside its refresh bounds | Requirement 5.16 |
| The cookie store separable from the web protocol state machines, so the trusted core can own it | `neoteletext-web`, `neoteletext-app` | Requirement 7.5 |
| The decoder as its own program, compiled to WebAssembly and run in `wasmi`; a fetcher with no decoder code | `neoteletext-app`, `neoteletext-core` | Requirements 3.3 and 3.4 |
| Sandboxes for the fetcher and decoder on macOS and Windows | `neoteletext-app` | Requirements 3.7 and 3.8 |
| A page's text, cleaned of control, bidirectional and zero-width characters and bounded, for display and accessibility | `neoteletext-core` or `neoteletext-browser` | Requirements 11.5 and 12.9 |

**Limits NeoTeletext takes from neoteletext.** Among them, as of 15 October 2026: `contents::MAX_PAGES` (800 pages), `contents::MAX_BYTES` (64 kilobytes), `DEFAULT_PAGE_LIMIT` (256 kilobytes), `DEFAULT_PUSH_LIMIT`, `DEFAULT_DESCRIPTOR_LIMIT`, `DEFAULT_HEADER_LIMIT`, `DEFAULT_FORM_LIMIT`, `DEFAULT_FETCH_INTERVAL_US` (one request every 0.2 seconds), `DEFAULT_CONNECT_TIMEOUT_US` (10 seconds), `DEFAULT_RESPONSE_TIMEOUT_US` (30 seconds), and `MIN_REFRESH_SECONDS` and `MAX_REFRESH_SECONDS` (5 seconds and 24 hours). The full table is in `neoteletext/docs/neoteletext-browser-open-questions.md` section 3.

---

## 6. Processes and isolation

### 6.1 The process model

This model was adopted by the owner on 15 October 2026 (decision 18).

```
┌──────────────────────────── trusted core (Rust, no web engine) ────────────────────────────┐
│ owns: tabs, history, bookmarks, settings, sessions, the cookie store, address parsing and   │
│ display, site-change rules, routing of bytes between fetcher and decoders, frame checking,  │
│ Portable Network Graphics encoding, the state of the trusted strip                        │
└──────┬───────────────────────────────┬───────────────────────────────┬─────────────────────┘
       │ framed pipe                    │ framed pipe                    │ framed pipe
       │ requests out, bytes in         │ bytes and commands in,         │ frames and view state out,
       │                                │ frames and events out          │ intents in
┌──────▼──────────────────┐   ┌─────────▼──────────────────┐   ┌─────────▼────────────────────────┐
│ fetcher                  │   │ decoder, one per channel    │   │ window host (decision 27)         │
│ network connections,     │   │ content detection, readers, │   │ Tauri and the system webview      │
│ Transport Layer Security,│   │ decoding, Presentation      │   │ hosts the webview; draws the      │
│ protocol readers         │   │ Levels 1.0 to 3.5, render   │   │ trusted strip (on Linux the       │
│ network only             │   │ inside wasmi; no network,   │   │ display host draws it, 6.6)       │
│                          │   │ no files, no secrets        │   │ no secrets                        │
└──────────────────────────┘   └─────────────────────────────┘   └───────────────────────────────────┘
       plus: the webview's own sandboxed content process, where page code runs;
             font helper (confined), updater and launcher (section 12);
             on Linux, the display host, which hosts the window host's nested
             display and draws the trusted strip (decision 30)
```

| Process | Holds | May access | If compromised, the attacker gains |
|---|---|---|---|
| **Trusted core** | Everything the user trusts | Its data directory, starting the other processes | Everything; this is why it reads no network format and contains no web engine |
| **Fetcher** | Network connections, Transport Layer Security sessions, the cookies attached to the requests it is currently making | The network only | The ability to send and receive on the network, and the cookies of requests in flight; no files, no history, no other channels' cookies |
| **Decoder** | One channel's pages | Nothing | Nothing outside its own memory, which holds one channel's public pages |
| **Font helper** | One font being read | Nothing | Nothing; its output is checked by hash and by shape |
| **Display host** (Linux only, decision 30) | The window host's nested display, and the trusted strip | The user's display, and the pipe to the trusted core for strip state and strip input | The user's display, and with it the trusted strip; it reads the window host's display protocol, so it is kept small, reads only core window protocols, and is fuzzed |
| **Window host** | What is on screen, and the state of the trusted strip | Its windows, the pipe to the trusted core, and on macOS the network entitlement the webview requires, which it never uses. On Linux no network, no files beyond its own, no local service, and only a nested display of its own (decisions 28 and 29) | The ability to draw anything in its windows, including a false trusted strip, and to send intents and strip input, which the trusted core checks; no secrets. Reaching it requires escaping the web content process's sandbox. |
| **Web content process** (the webview's own) | The page code | Nothing beyond what the platform's sandbox allows; on macOS no window server and no network sockets. On Linux it has no sandbox of its own under Tauri and inherits the window host's confinement, which leaves it only the nested display (decisions 28 and 29) | The page's own area of the window, and requests through the webview's networking process, which the Content Security Policy and WebRTC removal stop for page code |
| **Updater** | The update being downloaded | The update subdomain only, and the versions directory | Nothing that survives: every update is checked against The Update Framework metadata before use |

### 6.2 Why bytes pass through the trusted core

The fetcher never talks to a decoder directly. The trusted core routes every response to the decoder of the channel that asked for it, because only the trusted core knows which tab shows which channel and which navigations the user confirmed. The trusted core never parses those bytes; it passes them on.

### 6.3 Decoders per channel

- **Tabs on the same channel share one decoder and one cookie store** (decision 11).
- **Leaving a channel** sends the tab to that channel's decoder, starting one if needed. A decoder is stopped when no tab and no history entry uses its channel any more.
- **A local file or a capture** gets its own decoder, named by the file.
- **In the first version** one decoder per tab is acceptable, because the protocol is the same; moving to one per channel is a later step.

### 6.4 Messages between processes

All processes communicate over the framed pipe protocol of `neoteletext-app/src/pipe.rs`: one kind byte, a 32-bit length, and a payload, parsed strictly with bounded lengths. Every message type has a fixed upper size. A process that sends a malformed message is closed, and the tabs it served show an error page.

**From the trusted core to a decoder**, as defined in `neoteletext/docs/desktop-browser.md` section 3.4:

| Message | Meaning |
|---|---|
| Resource | A fetched resource for this channel: identifier, address, role, bytes |
| Push data | Bytes from a live feed |
| Show | Make this page the tab's current page |
| Key | A function from the input router, after the trusted core's own handling |
| Tick | The current time, for flash and time-outs |
| Settings | Display settings: presentation level, reveal, mix, font |
| Close | Forget a tab's state |

**From a decoder to the trusted core:**

| Message | Meaning |
|---|---|
| Frame | A rendered page: width, height, palette and 8-bit palette indices, 480 by 500, wider with side panels. Sent only when the page changes or flash changes phase. |
| Links | The page's links: Fastext keys, page-catching targets and page links, so the destination can be shown before it is followed |
| Wants | A page the decoder needs: a followed link or an enhancement dependency |
| Form | The page has a form: its fields and destination |
| Status | Searching, found, stale, and the page title from row 0 |

**Between the trusted core and the window host** (new in NeoTeletext):

| Direction | Message | Meaning |
|---|---|---|
| Core to interface | View state | The decorative state of each tab and window: tab titles as cleaned text, layout, which pages are shown. Never cookies, never addresses for security display (the trusted strip shows those). |
| Core to interface | Frame | One page image: tab, magazine and page, subcode, frame version, Portable Network Graphics bytes |
| Core to interface | Page text | The cleaned, bounded text of a page, for accessibility labels |
| Interface to core | Intent | What the user did in the page area: show a page, scroll, change layout, switch tab, close tab, a key, a click on a page cell, which pages are visible. Never a navigation to another channel. |
| Core to window host (on Linux, to the display host) | Strip state | What the trusted strip shows: the address as formatted by the display rules, the channel's identity, the encryption state, the site-change screen when one is pending. |
| Window host to core (on Linux, display host to core) | Strip input | What the user did in the trusted strip: a typed address, Back, Forward, Reload, Home, a site-change confirmation or cancellation, a security setting. Strip input is a separate message kind from page intents, and only the native strip produces it. |

**Between the trusted core and the display host** (Linux only, decisions 30 and 32), besides strip state and strip input above:

| Direction | Message | Meaning |
|---|---|---|
| Core to display host | Accessibility state | What the window's accessibility tree holds: the strip's state, and each shown page image with its cleaned, bounded text. The window host's own tree is never published on Linux. |
| Display host to core | Accessibility request | What the user's screen reader asked for: focus the strip or a page, set the address, press a strip control. The trusted core treats it as strip input or as an intent, exactly as if the user had done it. |

### 6.5 Isolation on each platform

| Platform | Trusted core | Fetcher | Decoder | Window host |
|---|---|---|---|---|
| macOS | Service, App Sandbox, no network | Service with the outgoing-network entitlement only | Service with no entitlements, and inside wasmi | The main application, holding the outgoing-network entitlement only because WKWebView renders nothing without it (`docs/proofs.md` section 2), and no file access beyond its bundle; the trusted core is a separate service |
| Windows | Main process | AppContainer with the internet-client capability only | AppContainer without it, low integrity, job object, and inside wasmi | AppContainer; whether WebView2 can run without the internet-client capability is not yet proven |
| Linux | Main process | Own process, Landlock and seccomp filter | Own process, Landlock and seccomp filter, and inside wasmi | Own process, confined from outside (decision 28): an empty network namespace; private `/run` and `/tmp` holding only the display's socket; a Landlock rule set with no home directory and no data directory. WebKitGTK's processes inherit all three (`docs/proofs.md` section 6.8). WebKitGTK's own content process sandbox stays off. Draws on a nested display of its own, never the user's (decision 29); the strip is drawn outside it, by NeoTeletext's own display host (decision 30). No system call filter yet (open decision 22). Section 6.6 summarises the Linux design and section 6.7 the alternatives considered |
| Android | Main application process | Separate service process | Isolated service process with seccomp filter, and inside wasmi | The application's webview (see section 13) |
| iOS | The application | Inside the application process | Inside the application process, inside wasmi only | The application's webview (see section 13) |

Today, in `neoteletext`, the fetcher and decoder are separate processes on macOS but are not yet sandboxed there (`neoteletext/docs/user-guide.md`, "Phase 2"). The macOS sandboxing is required work.

### 6.6 Linux: how the isolation is built

On macOS the system's web engine confines its own content process, and the operating system keeps that process from the window server (`docs/proofs.md` sections 2 and 3). On Linux neither holds under Tauri (`docs/proofs.md` section 6), so NeoTeletext builds the confinement itself, in three parts:

1. **Confinement from outside (decision 28, requirement 3.10).**
   - The trusted core starts the window host in an empty network namespace, with private `/run` and `/tmp` that hold only the nested display's socket, under a Landlock rule set with no home directory and no data directory.
   - WebKitGTK's processes inherit all three.
   - Measured: no network, no user files, and none of the machine's local services.
2. **A nested display of the window host's own (decision 29, requirement 3.11).**
   - The window host never receives the user's display, under X11 or Wayland.
   - Measured: under X11 a confined process could otherwise type commands into the user's terminal.
3. **NeoTeletext's own display host (decisions 30 and 31).**
   - **What it is:** a small Wayland compositor in Rust, on Smithay, running outside the confinement as an ordinary window on the user's desktop.
   - **Its own drawing:** it draws the trusted strip itself, outside the nested display. Nothing inside can cover the strip or type into it, not even an escape into the window host.
   - **What it offers the window host:** only core window protocols and text input.
   - **Input methods and screen readers (decision 32):** it is the user's input method's side of the window, and the window's publisher to the user's screen reader.

**Still open:** a system call filter for the window host (open decision 22), and the Wayland security-context protocol as a further layer (open decision 23). Both come from the research in section 6.7.

### 6.7 Other ways to organise security in a Linux application

Researched on 15 October 2026, to check the design of section 6.6 against what others do. For each: what it is, how it relates to NeoTeletext, and what NeoTeletext takes from it.

| Approach | What it is | Relation to NeoTeletext | What NeoTeletext takes |
|---|---|---|---|
| 1. Chromium's sandbox | Two layers [1]. Layer 1: the renderer runs in empty namespaces from unprivileged user namespaces, which also allows cutting off the network. Layer 2: a seccomp-bpf system call filter, "designed to shelter the kernel from malicious code executing in userland". Chromium's own AppArmor and SELinux layers are deprecated. | Section 6.6's confinement is layer 1. **NeoTeletext has no layer 2 for the window host.** | A system call filter for the window host, as for the fetcher and decoder (open decision 22) |
| 2. WebKitGTK's own sandbox | Bubblewrap with a seccomp filter, and a filtering proxy for the session bus | Measured (`docs/proofs.md` sections 6.3 to 6.5): real, but it gives the content process the display, and Tauri cannot turn it on without a patched `wry` | Not used (decision 28) |
| 3. Flatpak and portals | The whole application runs under Bubblewrap; trusted "portal" services outside it act for it, such as choosing files or capturing the screen, after asking the user | A proven pattern, but Flatpak would become the update path, against decision 24. It has had escapes: before 1.10.4 and 1.12.0, an application holding the Wayland or PipeWire socket could make portals treat it as unsandboxed [2]. | The portal idea, already present: the trusted core and display host act for the window host |
| 4. The Wayland security-context protocol | A sandbox tags its Wayland connections, and the user's compositor withholds dangerous protocols from them [3][4]. One compositor withholds screen capture, virtual input, the layer shell, clipboard control and the protocol itself from tagged clients [5]. | The standard way to limit a sandbox's display. It depends on the user's compositor, does nothing under X11, and does not move the strip out of reach. | A further layer on desktops that support it (open decision 23) |
| 5. Sandboxing libraries in the process: RLBox and wasm2c | Firefox compiles risky libraries to WebAssembly and back to C, so a fault cannot leave the library: Graphite, Hunspell, Ogg, Expat and Woff2 [6] | The same idea as the decoder running in wasmi | Already used for the decoder; candidates are the font helper and the image encoder |
| 6. A virtual machine per component: Qubes OS, Spectrum, crosvm | Each component runs in its own lightweight virtual machine. Its windows reach the host's compositor through virtio-gpu's cross-domain mode and a proxy compositor [7][8]. | The strongest isolation, by hardware virtualisation; section 6.6's display host is the same idea without the virtual machine | Not now: far heavier to run and ship. The upgrade path if process isolation proves insufficient. |
| 7. A user-space kernel: gVisor | System calls are answered by a kernel written in Go in user space, so the host kernel is barely reached [9] | Built for servers and containers; slow for input and output; awkward for graphical applications | Not used |
| 8. systemd's sandboxing settings | One launch applies settings such as a private network, a protected home directory, restricted socket families and a system call filter | Could replace the hand-written launcher of section 6.6, but needs systemd, and the user's systemd is the escape route section 6.6 closes | A fallback if the launcher proves hard to maintain |
| 9. AppArmor or SELinux profiles | A policy for each program, enforced by the operating system | Which one exists depends on the distribution, and Chromium deprecated both as sandbox layers [1] | An extra layer where present, never relied on |
| 10. A separate user account per component (privilege separation, as in OpenSSH) | The risky part runs as its own unprivileged user, so ordinary file permissions keep it from the user's files | Needs a privileged helper or a system-wide install; under X11 it still shares the display | Not used: section 6.6 gets the same separation without privilege |

**Conclusions.**
- Section 6.6 matches the strongest practice short of virtual machines: Chromium's layer 1, an in-process WebAssembly wall as in Firefox, and a trusted display host as in the virtual-machine systems.
- The one gap is a system call filter for the window host, which Chromium treats as essential (open decision 22).
- Rows 8 to 10 rest on general knowledge and were not verified in this research.

**Sources.**
1. Chromium, "Linux Sandboxing": https://chromium.googlesource.com/chromium/src/+/71.0.3578.80/docs/linux_sandboxing.md
2. CVE-2021-41133 (Flatpak): https://nvd.nist.gov/vuln/detail/CVE-2021-41133
3. Wayland security context protocol: https://wayland.app/protocols/security-context-v1
4. The protocol's addition to wayland-protocols: https://gitlab.freedesktop.org/wayland/wayland-protocols/commit/12c063088e97
5. The Umbriel compositor's policy for sandboxed clients: https://docs.noctalia.dev/umbriel/security/
6. Mozilla, "WebAssembly and Back Again: Fine-Grained Sandboxing in Firefox 95": https://hacks.mozilla.org/2021/12/webassembly-and-back-again-fine-grained-sandboxing-in-firefox-95/
7. crosvm, Wayland forwarding: https://crosvm.dev/book/devices/wayland.html
8. Spectrum OS, virtual machine Wayland support: https://lists.spectrum-os.org/archives/list/devel@spectrum-os.org/thread/BCXJ3JE7DK4SHDP37EMCMLWLDMCEPCYG/
9. gVisor: https://gvisor.dev/docs

---

## 7. Network access

### 7.1 What may be fetched: resource roles

The application loads only resources that are part of a teletext service (decisions 12 and 14). Each is fetched in exactly one role, and the response is read only by the reader for that role. Nothing a server sends can move a response into a different role.

| Role | When it is requested | Read by | Read in | What the trusted core receives |
|---|---|---|---|---|
| **Teletext data**: T42 packets, Teletext Text Interchange page files and the other formats of `neoteletext-formats`, and live feeds | Pages of a channel | Content detection and the format readers | Decoder | Frames, links, forms and status |
| **Channel descriptor** (JavaScript Object Notation) | At the channel's descriptor location | The descriptor reader of `neoteletext-web` | Fetcher | A bounded, typed description: aliases, structure, contents location |
| **Contents file** | When the channel declares one | The contents reader of `neoteletext-web` | Fetcher | A bounded, typed list of pages |
| **Font** (Glyph Bitmap Distribution Format) | Only when a page or channel names a font | The strict font reader of `neoteletext-formats` | Confined font helper | A fixed glyph table and its SHA-256 hash, checked again by the decoder (`neoteletext/docs/font-security.md`) |
| **Discovery** (HyperText Markup Language) | Only for an address the user typed that did not answer with teletext | The discovery reader of `neoteletext-web`: `Link` headers and `<link>` elements in the first 8,192 bytes, nothing else | Fetcher | At most a small number of candidate channel addresses |

Discovery has its own security analysis in `neoteletext/docs/web-discovery-security.md`. Its result is never loaded directly: each candidate address is a page-initiated navigation and goes through the site-change rules (section 9.5).

### 7.2 Transports

- **The HyperText Transfer Protocol version 1.1**, with or without Transport Layer Security. Plain, unencrypted connections are allowed (decision 10). The trusted strip shows "not encrypted" for a channel reached without Transport Layer Security.
- **WebSocket**, for two-way connections and live feeds.
- **Server-Sent Events**, for server push.
- **The zxnet live feed**, already supported by `neoteletext-web`.
- **Not supported:** the push mechanism of the HyperText Transfer Protocol version 2, which traditional browsers have removed. "Standard server push" in decision 12 means Server-Sent Events and WebSocket.

### 7.3 Rules that apply to every request

- **Addresses on the local network are refused** unless the user allows a specific address, as `neoteletext` already does (requirement REQ-SEC-006 of `neoteletext`).
- **Compression is transport, not type.** A response's `Content-Encoding` is undone by the fetcher with a fixed limit on the decompressed size, so a small compressed response cannot expand without bound.
- **The declared media type (`Content-Type`) is ignored** for acceptance (decision 13). It is shown in the developer tools for diagnosis only.
- **Every response has limits** on size, on time to first byte, on total time and on the number of redirects.
- **Redirects** are followed within these limits. A redirect that leads to another channel is a page-initiated navigation and goes through the site-change rules.
- **No `Referer` header is sent**, as `neoteletext-web`'s user agent profile already does.
- **The fetcher sends `neoteletext-web`'s minimal user agent profile** and nothing that identifies the user or the device beyond it.

---

### 7.4 Fetching never depends on viewing

Decision 22. The pattern of network requests must reveal nothing about which pages are viewed, so that neither a hostile interface nor a watching server can learn or signal anything through it.

| What the channel publishes | How its pages are fetched |
|---|---|
| A bundle, `index.t42` | The whole bundle when the channel opens, as `neoteletext` already does (`neoteletext/docs/channels.md` section 3.3) |
| A contents file, `contents.txt` | Every page file it lists, in the order it lists them, at a polite rate, when the channel opens |
| Neither: only separate page files (`100.tti`, `101.tti` and so on) | Each page is fetched when it is selected, as any browser does (decision 23). This is the one exception to this section: for these channels the pattern of requests does follow viewing. |

- **Refreshing** follows the channel's declared refresh schedule in `channel.json`, a fixed default when it declares none, or the channel's push feed. It never follows which page is shown.
- **Live feeds** already deliver every page.
- **Keys and interface requests only select among pages already held.** They never cause, delay, reorder or prioritise a request, except on channels with only separate page files, where selecting a page not yet held fetches it (decision 23).
- **Closing a tab** would stop its channel's refreshes, which the channel's server could notice. A closed tab's channel therefore keeps refreshing for a fixed grace period, defined in neoteletext (section 5.1).
- **Limits:** the maximum channel size and request rate are neoteletext's constants (section 5.1). The policy itself is implemented in neoteletext (decision 26).
- **What this replaces:** in `neoteletext`, a page's own file is fetched when the page is viewed, and the Fastext targets and neighbouring pages are prefetched (its requirement REQ-NAV-004). NeoTeletext does neither.
- **Privacy gain:** a channel's server cannot see which of its pages a viewer reads.

## 8. Content detection

A response in the teletext data role is recognised by its content alone, by `neoteletext-formats/src/detect.rs` (requirement REQ-INP-005 of `neoteletext`):

1. Signatures are tried first: edit.tf links, Teletext Text Interchange files, EP1 files, packetised elementary streams.
2. Then the headerless binary layouts, such as T42 packets, are scored over the first several hundred packets. A layout is accepted only when at least two in three of its packets carry addresses that decode correctly under the Hamming 8/4 error protection of the standard. Random bytes manage about one in three.
3. Detection waits until it has enough bytes, so its decision does not depend on how the network split the data.

**Rules added by this architecture:**

- **Detection runs in the decoder**, the process with no rights, because detection reads untrusted bytes.
- **"Teletext only" means "the decoder recognised it".** A response that detection rejects, such as a web server's error page, an image or a document, becomes a refusal page showing the HyperText Transfer Protocol status. It is never cached, never passed on and never shown.
- **A file that is valid in two formats** is read by the first match in detection's fixed order. Only teletext readers ever see it, so no confusion can lead anywhere else.
- **A live feed** is detected once, at its start, then read strictly. A feed that keeps producing malformed packets past a fixed threshold is closed and reported.
- **Data crafted to pass detection is teletext by definition,** and is then subject to every bound of the readers and of the decoder.

---

## 9. The security model of a tab

The aim is the same security behaviour as a tab in a traditional browser (decision 9). Teletext pages run no scripts, so several browser protections become simpler or stricter here. All of them are implemented in Rust, because the webview never fetches any site.

### 9.1 Origins and channels

- **Origin:** scheme, host and port, as in traditional browsers (`neoteletext-web`).
- **Channel:** origin plus folder path (`neoteletext` decision E.70). The channel is the unit of isolation. It is finer than a traditional browser's site, so no list of public suffixes is needed.

### 9.2 The equivalent of the same-origin policy

Traditional browsers stop scripts from reading other origins' data. Teletext has no scripts, so the policy becomes a set of rules about data:

- A channel's pages, cookies, caches and history are never mixed with another channel's.
- A decoder receives only its own channel's bytes. The trusted core routes every response to the decoder that asked for it.
- A page cannot cause a request to another channel. Anything that would do so is a navigation and goes through the site-change rules.

### 9.3 Cookies

The cookie store is `neoteletext-web`'s, which follows the cookie specification draft RFC 6265bis of 30 September 2026 as browsers implement it, made stricter:

| Behaviour | Traditional browser | NeoTeletext |
|---|---|---|
| Scope | The host, or a whole domain with the `Domain` attribute | **The host only, always.** The `Domain` attribute is ignored. |
| `Secure` attribute | Sent only over Transport Layer Security | The same. Plain connections are allowed but never receive `Secure` cookies. |
| `__Host-` and `__Secure-` name prefixes | Enforced | Enforced |
| Expiry (`Expires`, `Max-Age`) | Enforced | Enforced |
| `HttpOnly` attribute | Hides the cookie from scripts | Has no effect, because there are no scripts. Every cookie is effectively `HttpOnly`. |
| `SameSite` attribute | Withholds cookies on cross-site requests | The same, applied to form submissions that go to another channel |
| Third-party cookies | Blocked or partitioned | **There is no third-party context at all.** A page cannot cause a credentialed request to another origin. |
| Sharing between tabs | Same-site tabs share cookies | Tabs on the same channel share cookies (decision 11) |

**Where the cookie store lives.** In `neoteletext` today the web state machine, cookie store included, runs inside the fetcher process (`neoteletext-app/src/fetch.rs`). In NeoTeletext the **trusted core owns the cookie store**. The fetcher is given, for each request, only the cookies that request carries, and returns any `Set-Cookie` headers to the trusted core, which applies them. A compromised fetcher therefore sees only the cookies of requests in flight, never the whole store. This is a change to the current arrangement and is listed as required work.

### 9.4 Storage and caches

- The decoder's page store belongs to one channel.
- The trusted core's frame cache and back-and-forward cache are keyed by tab and channel.
- The network cache is keyed by channel. Because there are no third-party loads, one key level is enough; traditional browsers need two.
- The webview stores nothing that belongs to a channel. It has no cookies, no site storage and no network cache of channel content.
- Page code cannot use the network: the Content Security Policy stops every request it governs, WebRTC is removed in every frame before page scripts run, and the web content process has no network sockets of its own. Measured on macOS: nothing reached a test server (`docs/proofs.md` section 2). The operating system cannot deny the window host the network on macOS, because the webview then renders nothing (requirement 10.11).

### 9.5 Navigation and the site-change screen

As in `neoteletext/docs/desktop-browser.md` sections 6 to 8:

- **Navigations the user starts:** the address field in the trusted strip, bookmarks, presets, Back, Forward, Home.
- **Navigations a page starts:** Fastext keys, page catching, page numbers that follow a page link, redirects, discovery results, and a decoder's request for a page on another channel.
- **The address shown is always the committed address** until a new page commits.
- **Any navigation that leaves the channel goes through the site-change screen,** which shows the destination and needs the user's confirmation. It is drawn in the trusted strip (section 11.3). The window host cannot start, show, fake or confirm it.

### 9.6 Forms

- A page's form is sent only to its declared destination, through the fetcher.
- A form whose destination is another channel goes through the site-change screen, and the destination channel's `SameSite` cookies are withheld, as a traditional browser does.
- No values are filled in automatically.

### 9.7 Transport Layer Security

- Certificates are checked by the fetcher.
- The trusted strip shows whether the current channel arrived encrypted.
- A certificate that fails its checks is a hard stop, with no way to proceed (decision 24).

### 9.8 Local files

Each local file or capture is its own origin, with no cookies. Links from it to web addresses are page-initiated navigations and go through the site-change screen.

### 9.9 Private windows and containers

Private windows (a separate in-memory cookie store, no history, discarded on close) come after the first version. Per-tab containers are not provided: tabs on the same channel share cookies by decision 11 (decision 24).

---

## 10. How a page reaches the screen

1. **The decoder renders a frame:** 8-bit palette indices, 480 by 500 (40 columns of 12 dots, 25 rows of 20 lines), wider with side panels, and a palette of at most 32 colours (decision 6). It sends a frame only when the page changes or flash changes phase.
2. **The trusted core checks the frame:** known tab and page, dimensions within the raster limits, palette of at most 32 entries, every index within the palette.
3. **The trusted core encodes an indexed Portable Network Graphics image** with `neoteletext-image`'s writer, using a palette sized to the colours present. Frames are compared by a hash of indices and palette, so an unchanged page is never sent again.
4. **The trusted core sends the frame to the window host** on the frame pipe: a fixed binary header (tab, magazine and page, subcode, frame version, image length) followed by the image.
5. **The window host shows it in that page's `<img>` element:** it makes a `blob:` address for the bytes, decodes the image away from the screen with `HTMLImageElement.decode()`, then swaps the visible image and releases the previous address. If a newer frame for the same page arrives while one is decoding, the older one is dropped.
6. **Flash** is a palette change in the engine. The trusted core keeps both phases of a flashing page and alternates them on the flash timer, without asking the decoder again.

**Showing many pages at once (decision 7):**

- The window host reports which pages are visible, using an intersection observer on each page image plus a margin for scrolling. The trusted core asks decoders for frames, and sends frames, only for those pages. A wall of 800 pages costs what is visible.
- Every page is rendered at its native raster. The interface scales images with Cascading Style Sheets: `image-rendering: pixelated` when enlarged, smooth when reduced to a thumbnail.
- Teletext dots are not square on a 4:3 display. A horizontal scale in Cascading Style Sheets corrects the shape without rendering again.
- A mostly flat page as an indexed Portable Network Graphics image is expected to be a few kilobytes. This is to be measured.

**Why images and not a canvas.** Images keep the interface simple and accessible, and the browser engine's image decoder only ever sees well-formed files produced by our own encoder. If measurement shows that decoding images limits very large walls, a canvas fed palette indices, with flash done on the graphics processor, is the alternative (decision 24).

---

## 11. The browser shell

The shell is everything around the pages: tabs, the address field, the toolbar, menus, panels and dialogs.

**How the shell is split:**

- **The trusted strip,** drawn natively by the window host in the same window as the webview, from state the trusted core sends, holds the address field, the channel's identity, the security state and every security decision. Page code cannot reach it (decision 27). On Linux the display host draws it instead, outside the window host's nested display (decision 30).
- **The window host** draws everything decorative: page walls, transitions, the tab strip, panels, internal pages. It is a React application written in TypeScript (decision 19). It renders only from the state the trusted core sends, shows frames, forwards input and plays transitions. Page text is rendered only as React text nodes, which escape every value; nothing builds markup from strings. Its run-time JavaScript dependencies are a fixed, approved list: React, React DOM and the Tauri interface package.
- **Rust owns all state.** The window host renders what it is told and reports what the user did. It decides nothing about navigation, channels, addresses or security.

### 11.1 State flow

```
trusted core ──view state, frames, page text──► window host
trusted core ◄─────────────── intents ───────── window host

trusted core ──strip state: address, channel, security──► trusted strip (native, in the window host)
trusted core ◄──strip input: typed address, confirmations── trusted strip (native, in the window host)

on Linux the trusted strip is in the display host instead (decision 30):
trusted core ──strip state, accessibility state──────────► display host
trusted core ◄──strip input, accessibility requests─────── display host
```

- The trusted core sends the window host a full snapshot of each tab's decorative state whenever it changes, so the interface never merges partial updates.
- An intent that is not allowed changes nothing. The interface keeps showing the last state.
- The trusted strip shows only what the trusted core sends, and its input is a separate message kind from page intents. Page code cannot produce strip input; only an escape from the web content process into the window host could forge it (`security-analysis.md`).

### 11.2 Tab strip

| Drawn by the window host | Decided by the trusted core |
|---|---|
| Tabs, drag to reorder, close buttons, overflow scrolling, open and close animations, a live thumbnail of the tab's page on hover | The list and order of tabs, creating and closing tabs, which decoder serves each tab, restoring sessions |

A tab shows the channel's name as cleaned text from the trusted core, a loading indicator, and a small coloured mosaic derived from the channel's address.

### 11.3 The trusted strip: address field, identity and security decisions

The trusted strip is a native area along the top of every window, drawn by the window host in the same window as the webview but outside the webview's area, so page code cannot draw over it or read it (`docs/proofs.md` section 3.2). On Linux it is drawn by the display host, outside the window host's nested display, so not even the window host can reach it (decision 30, `docs/proofs.md` section 6.14). It shows only what the trusted core sends and passes every input to the trusted core as strip input.

- **Display mode:** the channel's address as formatted by the address display rules (scheme, host emphasised, internationalised names shown safely), whether the channel arrived encrypted, and the channel's name.
- **Edit mode:** a native text field. On Enter the text goes to the trusted core, which parses it with `neoteletext-web`'s strict address parser and either navigates or shows an error inside the strip.
- **Suggestions** from history and bookmarks are computed by the trusted core and shown by the trusted strip.
- **The site-change screen** is shown by the trusted strip: the destination channel, how the navigation was started, and a confirmation. It is shown in the trusted strip only, never in the page area, so a page image cannot imitate it (decision 24).
- **The trusted strip stays visible in every layout**, including full-screen and zoomed views. If a layout hides it, it reappears on any interaction.
- **Native text input** gives the on-screen keyboard, input methods for every script, selection and pasting on every platform. On Linux the display host is the input method's side of the strip and of the window host's text fields (decision 32).

### 11.4 Toolbar and navigation

- Back, Forward, Reload and Home are buttons in the trusted strip, and swipe gestures in the page area. A swipe is an intent that the trusted core checks.
- The trusted core keeps one navigation controller per tab with a flat history list (`neoteletext/docs/desktop-browser.md` section 6).
- Recent frames are kept so that going back shows the previous page at once, before the decoder answers.

### 11.5 Page area and page input

- **Page walls:** grids, lists and stacks of page images, with Cascading Style Sheets transforms and view transitions for zooming, subpage stacks and magazine overviews. This carries over from the prototype's size slider, list view and the Tilt and Fan transforms.
- **Clicks on a page:** the window host converts the pointer position on the image (`offsetX` and `offsetY`, which are correct under any Cascading Style Sheets transform; verified in the prototype under three-dimensional tilt in WebKit and Chromium) into a cell, column and row, and sends a click intent. The trusted core asks the decoder what is in that cell and decides whether it is a link. A link to another channel goes through the site-change screen.
- **Keys:** digits enter page numbers, colour keys follow Fastext links, and reveal, hold and mix work, with the page area focused, in the tab (decision 22). The window host forwards normalised key presses to the trusted core's input router (`neoteletext/docs/hmi.md`), which maps them to functions, so the keyboard map is in one place. On bundled channels and channels with a contents file, a key can only select among pages the channel's decoder already holds and never causes a network request; a page not yet received shows that it is being searched for, as a television does while waiting for the carousel. On channels with only separate page files, selecting a page not yet held fetches it (decision 23). A key that would leave the channel, such as a Fastext link to another channel, goes through the site-change screen in the trusted strip.

### 11.6 Menus and shortcuts

- A native menu bar, drawn by the window host and acted on by the trusted core: File, Edit, View, History, Window. The Edit menu provides copy, paste and undo in text fields.
- Shortcuts live on menu items, so they work whatever has focus: new tab, close tab, open location, reload, back, forward, next and previous tab, developer tools, and the rest of `neoteletext/docs/desktop-browser.md` section 10.
- Menu items and toolbar buttons send the same intents to the same code.

### 11.7 Internal pages

- Bookmarks, history, presets and settings are internal pages drawn by the window host from data the trusted core supplies.
- **Settings that affect security are changed only in the trusted strip,** never in the window host.
- Refusal and error pages ("this address does not serve teletext", "certificate rejected") are chosen by the trusted core.

### 11.8 Developer tools

A docked panel, toggled from the menu and the toolbar. All its data comes from the trusted core and is shown as text.

| Section | Shows | Source |
|---|---|---|
| Packets | The packet log of the current channel: magazine, row, page, errors corrected or found | Decoder |
| Pages | The channel's page store: pages, subpages, versions, enhancement data present | Decoder |
| Network | Requests made for this channel: address, role, declared media type, status, size, time | Trusted core and fetcher |
| Storage | The channel's cookies and cached resources | Trusted core |
| Frames | For each page: frame rate, image size, encoding time, whether visible | Trusted core |
| Console | Warnings and errors from the engine | Decoder and trusted core |

### 11.9 Windows

- Each window has one window host view and one trusted strip.
- Tab state lives in the trusted core, so moving a tab to another window is a change of state and a redraw.
- Frames for a page go only to the window showing it.

### 11.10 Accessibility

- The shell's markup is accessible to screen readers directly.
- Each page image carries an accessible label holding the page's text, extracted by the trusted core from the decoded rows and passed as plain, cleaned text.
- The trusted strip uses each platform's native accessibility.
- **On Linux** the window host has no accessibility bus (decision 28). The display host publishes the window's tree itself, from the trusted core's accessibility state: the strip, and each page image with its text (decision 32).

### 11.11 Phones

- The same shell adapts by layout: the trusted strip at the bottom, a tab overview of live page thumbnails, swipes between tabs and subpages, the system back gesture as Back.
- Tauri on Android and iOS gives one webview per window, which this design uses.
- Process separation on phones is limited (section 13).

---

## 12. The application update mechanism

### 12.1 What "no scope for failure" means

No mechanism can stop downloads from failing, disks from filling or power from failing. What this design guarantees is that **no failure at any point leaves the user without a complete, working application**, and that **no update that has not been verified is ever run**.

### 12.2 Invariants

1. **The running version is never modified.** A new version is installed beside it, in its own complete directory.
2. **Nothing switches until the new version is fully downloaded, verified against The Update Framework metadata and the platform's code signature, and installed.**
3. **The switch is a single atomic operation:** one rename of a pointer that the file system either completes or does not. After a power failure the pointer names either the old or the new version, and both are complete.
4. **The previous version stays installed** until the new version has started and passed its health check.
5. **Automatic rollback:** if the new version fails its health check, or fails to start twice, the launcher switches the pointer back on the next start.
6. **User data is never migrated in place.** A version that needs a new profile format copies the profile and migrates the copy. The old profile stays untouched, so a rolled-back version finds its own data intact.
7. **A small launcher that changes rarely** reads the pointer, starts the chosen version, counts failed starts and performs rollback. It is small enough to review completely.

### 12.3 Verification: The Update Framework

The Update Framework (decision 16) is designed to survive a compromised update server and a stolen key:

| Role | Signs | Keys | Kept |
|---|---|---|---|
| Root | The keys of every other role | Threshold, for example two of three | Offline, held by separate people |
| Targets | The hash and size of every release file | One or more | Offline |
| Snapshot | The versions of all metadata | One | Offline or on a signing machine |
| Timestamp | The latest snapshot, with a short expiry | One | May be online, because it can only say "nothing has changed" |

- **Expiry** stops an attacker from serving old metadata to hold users on a vulnerable version: stale metadata is refused, and the application shows that updates are unavailable.
- **Version numbers only increase**, so an attacker cannot roll users back to an older, vulnerable release.
- **Root keys can be replaced** by a new root signed with the old threshold, so a lost or exposed key does not require reinstalling the application.
- **The Rust implementation is `tough`** (decision 24), maintained by Amazon Web Services for the Bottlerocket operating system's updates.
- **The platform's code signature is checked as well:** Apple notarisation and code signing on macOS, Authenticode signing on Windows. This is a second, independent check.
- **The root public keys are built into the launcher.**

### 12.4 Packaging: one launcher scheme on every desktop

**Decided (decision 24),** so that The Update Framework verification is on every update path with no exception. Operating-system package managers such as MSIX on Windows and Flatpak on Linux download and install by themselves and trust only the package's code signature or repository signature, which would bypass The Update Framework. 

| Platform | Arrangement |
|---|---|
| macOS | A launcher application, plus versioned application bundles in the application's support directory. Every bundle is signed and notarised. Files the updater writes carry no quarantine attribute. The bundle's signature is checked before launch. |
| Windows | A small fixed stub that never changes, which chooses between two launcher copies; versioned application directories; every executable Authenticode-signed and checked by the updater in addition to The Update Framework. |
| Linux | Versioned application directories or AppImages in the user's data directory, and a pointer switched by atomic rename. No administrator rights needed. |
| Android, iOS | The application stores install updates atomically. Applications cannot replace their own code there. No custom updater. |

**Updating the launcher itself** is rare and uses the operating system's atomic exchange of two directories where it exists: `renamex_np` with `RENAME_SWAP` on macOS, `renameat2` with `RENAME_EXCHANGE` on Linux. On Windows, which has no atomic directory exchange, the fixed stub chooses between two launcher copies.

### 12.5 Hosting

- **`update.neoteletext.com`**, a subdomain used for nothing else (decisions 17 and 25).
- **The host is not trusted.** The Update Framework metadata decides what is accepted, so a compromised server can only refuse service or serve stale metadata, which expiry turns into a visible "updates unavailable". It cannot cause a malicious build to run.
- **Static files only:** metadata and release files, served over Transport Layer Security.
- **The updater is the only component allowed to contact the update subdomain,** and it contacts nothing else. The browser's rule that only teletext resources are fetched is unaffected.

### 12.6 Proving it

The update mechanism is accepted only when these automated tests pass on every desktop platform:

- The updater is killed at every step and between every file write, and the application still starts.
- Writes are cut off part-way through a file, simulating power loss; nothing partially written is ever used.
- Corrupt, truncated, wrongly signed, expired, older-version and mismatched packages are all refused.
- A full disk makes the update fail cleanly, and the current version keeps working.
- A deliberately broken new version is rolled back automatically.
- A profile migrated by a new version leaves the old profile usable after rollback.

---

## 13. Platforms

| Platform | Webview in the window host | Notes |
|---|---|---|
| macOS | WKWebView (WebKit) | Updated with the operating system |
| Windows | WebView2 (Chromium) | Updated by Microsoft independently of the application |
| Linux | WebKitGTK | Updated by the distribution; often weeks to months behind upstream security fixes. A start-up check of the WebKitGTK version, with a warning below a known-fixed release, is required. The window host is confined from outside (decision 28) and draws on a nested display of its own, never the user's (decision 29). The trusted strip is drawn by NeoTeletext's own display host, outside the nested display (decision 30). |
| Android | Android System WebView (Chromium) | Updated through the Play Store. The webview cannot be moved into a separate process of the application's choosing in the same way as on desktops; the decoder's isolated service process and wasmi remain. |
| iOS | WKWebView (WebKit) | One application process. The decoder's only wall is wasmi, as in `neoteletext/docs/desktop-browser.md` section 3.6. WebKit's own content process still separates script execution from the application. |

**The first version is desktop only; phones come later** (decision 24).

---

## 14. Code layout

```
neoteletext/
  launcher/                 the small launcher and update stub (section 12)
  updater/                  the updater process
  src-tauri/                the window host (Tauri)
    src/strip/              the native trusted strip on macOS and Windows (decision 27)
  display-host/             Linux only: the display host, in Rust on Smithay (decisions 30 to 32):
                            nested display, trusted strip, input methods, accessibility tree
  core/                     the trusted core (Rust, no web engine)
    src/state/              tabs, history, bookmarks, settings, sessions
    src/routing/            starting, confining and routing the other processes,
                            including the Linux confinement of the window host (decision 28)
    src/frames/             frame checking, de-duplication, image encoding
    src/cookies/            the cookie store (from neoteletext-web)
    src/policy/             navigation, site-change and address display rules
  ui/                       the React interface (TypeScript), run in the window host
  docs/                     this document, requirements, security analysis
```

The fetcher, decoder and font helper binaries are built from `neoteletext` crates.

---

## 15. The prototype and what it taught

The prototype on branch `main`, tag `tauri2`, is a Tauri 2 shell with one child webview per tab, each with its own WebKit data store, showing egui applications compiled to WebAssembly and drawn with WebGL. It is not the target design. It established these facts on this codebase:

1. **Tauri grants a command when either the window label or the webview label matches a capability** (`tauri/src/ipc/authority.rs`, function `resolve_access`). A capability that names a window also covers every webview inside it. Capabilities must name webview labels, and every command must check its caller's label as well.
2. **Key presses made while a native webview has focus never reach any other webview.** Shortcuts belong in the native menu.
3. **Native child webviews sit above everything their parent webview draws.** This is one reason the trusted strip is native and outside the webview's area.
4. **Cookies set by pages served from the `tauri://` scheme are dropped.** In this design cookies are held in Rust and never in the webview.
5. **A newly created webview can start at zero size.** Interface code must not fix its sizes from the first viewport it sees.
6. **Resizing a canvas clears it, and resize observers run after a frame's animation callbacks.** A redraw deferred to the next animation frame paints a blank frame. Redraw inside the resize callback.
7. **The pointer position on a transformed element, `offsetX` and `offsetY`, is reported in the element's own untransformed coordinates** in both WebKit and Chromium, including under three-dimensional transforms. This makes clicks on transformed page images reliable.

---

## 16. The Tauri 3 and Chromium Embedded Framework trial

Tried on 15 October 2026 with `tauri` 3.0.0-alpha.4, `tauri-runtime-cef` 3.0.0-alpha.5 and Chromium 152.

- It built, rendered, and ran Chromium's multi-process model with a separate profile per tab. The application grew from 11 megabytes to 334 megabytes.
- **Calls between the webview and Rust were attributed to the wrong webview** when one window held several webviews: the shell's calls arrived labelled as the newest tab's. The runtime maps a request to a webview through a registry lookup and falls back to an arbitrary entry when it cannot identify the browser (`tauri-runtime-cef`, `cef_impl/request_handler.rs`). Permissions are checked against that attribution, so the reverse error would give a tab the shell's permissions.
- A separate defect: once remote debugging had been disabled in a profile, enabling it in a later run was refused.
- The work is kept on branch `tauri3`, stash `stash@{0}`. It is not reported upstream (decision 2).
- This design uses one webview per window, which avoids the multi-webview case, but attribution must be tested again before any change of runtime (requirement in `requirements.md` section 13).

---

## 17. Decisions still open

| Number | Question | Recommendation |
|---|---|---|
| Open decision 1 | Closed by decision 24. | — |
| Open decision 2 | Closed by decision 19: Blitz is rejected because it cannot run the React interface. | — |
| Open decision 3 | Closed by decision 24. | — |
| Open decision 4 | Closed by decision 24. | — |
| Open decision 5 | Closed by decision 24. | — |
| Open decision 6 | Closed by decision 24. | — |
| Open decision 7 | Closed by decision 24. | — |
| Open decision 8 | Closed by decision 24. | — |
| Open decision 9 | Closed by decision 24. | — |
| Open decision 10 | Closed by decision 24. | — |
| Open decision 11 | Closed by decision 24. | — |
| Open decision 12 | Closed by decision 24. | — |
| Open decision 13 | Closed by decision 24. | — |
| Open decision 14 | Closed by decision 25: `update.neoteletext.com`. | — |
| Open decision 15 | Moved to neoteletext by decision 26: the live-feed threshold is defined there. | — |
| Open decision 16 | Closed by decision 22: fetching never depends on viewing. | — |
| Open decision 17 | Closed by decision 23: channels with only separate page files fetch each page when it is selected, as an accepted exception. | — |
| Open decision 18 | Moved to neoteletext by decision 26: the fetching limits, default refresh and grace period are defined there. | — |
| Open decision 19 | Closed by decision 29: a nested display for the window host on every Linux session. | — |
| Open decision 20 | Closed by decision 30: NeoTeletext's own display host. | — |
| Open decision 21 | Closed by decisions 31 (Rust, Smithay) and 32 (input methods and screen readers through the display host). | — |
| Open decision 22 | Should the Linux window host also run under a seccomp system call filter, as Chromium's renderer does (section 6.7, approach 1)? The fetcher and decoder already have one (section 6.5); the window host has only the confinement of decision 28, so a fault in the web engine faces the whole kernel interface. The filter must allow what WebKitGTK and GTK need, which must be measured. | Yes: measure what WebKitGTK and GTK need under the confinement, and filter to that. |
| Open decision 23 | Should the display host, on Wayland desktops that support the security-context protocol, also tag its own connection to the user's compositor, so the compositor withholds dangerous protocols from it too (section 6.7, approach 4)? It costs little and narrows what an escape into the display host could reach; it is unavailable under X11 and on compositors without the protocol. | Yes, where supported, as an extra layer that is never relied on. |

---

## 18. Glossary

| Term | Meaning |
|---|---|
| AccessKit | A Rust library that publishes an application's accessibility tree to each platform's screen readers; the display host uses it (decision 32). |
| App Sandbox | The macOS sandbox for applications: a process runs with only the entitlements it is given. |
| AppArmor | A Linux mechanism that enforces a security policy for each program, used by some distributions. |
| AppContainer | The Windows sandbox for applications: a process runs with only the capabilities it is given. |
| AppImage | A Linux packaging format: one self-contained executable file per application version. |
| AT-SPI | The Linux accessibility interface: applications publish their contents on the accessibility bus, and screen readers such as Orca read them there. |
| Authenticode | Microsoft's code signing for Windows executables. |
| Blitz | A Rust engine from the Dioxus project that lays out and draws HyperText Markup Language and Cascading Style Sheets without a JavaScript engine. |
| Bubblewrap | A Linux tool that starts a program in new namespaces with a chosen view of the file system; used by Flatpak and by WebKitGTK's own sandbox. |
| Cascading Style Sheets | The language that describes how markup is laid out and styled, including transitions and animations. |
| Chromium Embedded Framework | A framework for embedding the Chromium browser engine in an application. |
| Colour Look-Up Table, Colour Map | In teletext Presentation Level 2.5, four tables of eight colours each, together the 32-colour Colour Map. |
| Compositor | The program that combines windows into what a display shows and routes input to them; under Wayland it is the display server. |
| Entitlement | On macOS, a permission granted to a signed application or service, such as outgoing network access. |
| Fastext | Teletext's coloured links: four keys that jump to pages the broadcaster chose. |
| Flatpak | A Linux packaging and sandboxing system: each application runs under Bubblewrap and reaches the desktop through portals. |
| Glyph Bitmap Distribution Format | A text format for bitmap fonts, used for teletext fonts. |
| gVisor | A kernel written in Go that runs in user space and answers a sandboxed program's system calls, so the host kernel is barely reached. |
| Hamming 8/4 | The error protection teletext uses for addresses: each byte carries four data bits and can have one error corrected. |
| HyperText Markup Language | The markup language of web pages; here used only for the decorative interface, and read only for link pointers during discovery. |
| HyperText Transfer Protocol | The request and response protocol of the web. Version 1.1 is used. |
| Input method | The program that turns key presses into text that cannot be typed directly, such as Chinese, Japanese or Korean; on Linux, IBus or Fcitx. |
| JavaScript Object Notation | A text format for structured data, used by channel descriptors. |
| Landlock | A Linux mechanism by which a process restricts its own and its children's access to files. It does not govern connecting to a local socket. |
| Mount namespace | A Linux process's own view of the file system's mounts; directories can be replaced in it without affecting other processes. |
| MSIX | Microsoft's Windows application package format, with transactional installation. |
| Network namespace | A Linux process's own set of network interfaces; an empty one has only an unconnected loopback interface. |
| Notarisation | Apple's automated check of a signed application before macOS will run it without warnings. |
| Origin | Scheme, host and port of an address. |
| Portable Network Graphics | A lossless image format with indexed palettes; used for every page image. |
| Portal | A trusted service outside a sandbox that acts for a sandboxed application, such as choosing a file or capturing the screen, after asking the user. |
| Presentation Level | The teletext standard's feature levels: 1.0 and 1.5 (basic), 2.5 (enhanced colours and characters), 3.5 (further enhancements). |
| RFC 6265bis | The current draft of the Internet Engineering Task Force specification for cookies. |
| RLBox | Firefox's framework for running a library in a WebAssembly sandbox inside the browser process. |
| Same-origin policy | The traditional browser rule that a script may read data only from its own origin. |
| seccomp filter | A Linux mechanism by which a process restricts which system calls it may make. |
| Security-context protocol | A Wayland protocol by which a sandbox tags its connections so that the compositor can withhold dangerous protocols from them. |
| SELinux | A Linux mechanism that enforces a security policy for each program and file, used by some distributions. |
| Server-Sent Events | A standard way for a web server to push a stream of messages to a client over one HyperText Transfer Protocol response. |
| Session bus | The D-Bus message bus of a user's login session, through which desktop services, including the user's systemd, take requests. |
| SHA-256 | A cryptographic hash function from the Secure Hash Algorithm 2 family, producing a 256-bit digest. |
| Smithay | A Rust library for writing Wayland compositors; the display host is built on it (decision 31). |
| Tauri | A framework for building applications with a Rust core and a system webview interface. |
| Teletext Text Interchange | A text file format for teletext pages, used by page editors. |
| Text-input protocol | The Wayland protocol (`zwp_text_input_v3`) through which a compositor gives a window text an input method composed. |
| The Update Framework | A specification for secure software updates that survive compromised servers and stolen keys, using signed, expiring metadata in separate roles. |
| `tough` | A Rust implementation of The Update Framework. |
| Transport Layer Security | The protocol that encrypts and authenticates network connections. |
| Virtual machine | A whole computer emulated with hardware support, with its own kernel; the strongest separation between programs on one machine. |
| wasmi | A WebAssembly interpreter written in Rust, without just-in-time compilation; it runs the decoder. |
| Wayland | The newer Linux display protocol, in which a client normally reaches only its own windows; what else it may do depends on the protocols the compositor offers. |
| WebAssembly | A portable binary code format; the decoder is compiled to it and run inside wasmi so that a decoder fault stays inside its memory. |
| WebSocket | A protocol for a long-lived two-way connection over the web. |
| Webview | An operating system's embeddable web engine: WKWebView, WebView2, WebKitGTK or Android System WebView. |
| X11 | The older Linux display protocol, version 11 of the X Window System, in which any client can draw over, read and send input to every other client's windows. |
| XPC service | On macOS, a helper process with its own sandbox and entitlements, started and managed by the system. |
