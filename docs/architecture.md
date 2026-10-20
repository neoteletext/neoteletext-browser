# T42 Browser: architecture

| | |
|---|---|
| **Status** | Adopted 16 October 2026 (decision 33). Built in part, in `native/`; section 15 says what is built and what is not. |
| **Date** | 16 October 2026 |
| **Companion documents** | [`requirements.md`](requirements.md) states every requirement in full, numbered, with its reason, how it is verified and whether it is met. [`security-analysis.md`](security-analysis.md) analyses this architecture's security. [`hyperlinks.md`](hyperlinks.md) proposes hyperlinks (open decision 8) and [`hyperlinks-security.md`](hyperlinks-security.md) analyses their security. [`licensing.md`](licensing.md) says which licences may sit beside the browser's code. [`native-interface-design.md`](native-interface-design.md) records how this architecture was chosen, the prototypes it was chosen from, its review against the earlier design, and what was built when. |
| **The earlier design** | The Tauri design that came before, with its own architecture, requirements, security analysis and proofs, is kept in [`../prototypes/docs/`](../prototypes/docs/), beside the prototypes that tested it. This document replaces it; where the two differ, this one holds. |
| **Engine documents** | The teletext engine's own design lives in `~/devel/neoteletext`: `ARCHITECTURE.md`, `docs/channels.md`, `docs/web-discovery-security.md`, `docs/font-security.md`. This document refers to them by path. |

**Abbreviations.** This project avoids abbreviations, because they hide meaning. Where a technical term is normally known by its initials, the full name is written out, followed by the initials in brackets only when the initials are a literal identifier in code, configuration or a standard's title. Section 17 is a glossary of every technical term used.

---

## Contents

1. What T42 Browser is
2. The owner's decisions
3. Concepts and terms
4. Design principles
5. The teletext engine and what is reused
6. Processes and isolation
7. Network access
8. Content detection
9. The security model of a tab
10. How a page reaches the screen
11. The browser window
12. The application update mechanism
13. Platforms
14. Code layout
15. What is built and what is not
16. Decisions still open
17. Glossary

---

## 1. What T42 Browser is

T42 Browser is an application for browsing teletext, on desktops, then phones, and with features cut on microcontrollers.

- **Teletext** is the page-based information service defined by the European Telecommunications Standards Institute standard EN 300 706, "Enhanced Teletext specification". A copy is at `~/devel/teletext-reference/teletext-standards/en_300706v010201o.pdf`.
- **Every tab is one teletext channel.** A teletext channel is one teletext service: eight magazines, numbered 1 to 8, each with pages numbered 00 to FF in hexadecimal, and each page with subpages. T42 Browser shows pages 100 to 899.
- **A channel comes from the web or from a file.** On the web a channel is a folder on a server (`neoteletext/docs/channels.md`); a file is a recording or a folder of page files on this computer.
- **Every image on screen is one teletext page,** rendered by the teletext engine at the native raster and scaled by the interface.
- **The interface is drawn by T42 Browser's own Rust code, with no web engine.** It draws on the processor, so it needs no graphics processor, and it shows many pages at once and animates between them.
- **Security is paramount.** The owner has stated that there are no circumstances under which being hacked is acceptable (decision 4). Where a choice trades capability against security, security wins.

---

## 2. The owner's decisions

Each decision was made by the owner on the date given. Requirements in `requirements.md` cite the decision they come from. Decisions 1 to 32 were recorded with the earlier design (`../prototypes/docs/architecture.md` section 2), where their full text stays; their numbers are kept here so that every citation still holds.

| Number | Date | Decision | Now |
|---|---|---|---|
| Decision 1 | 15 October 2026 | The teletext engine is `neoteletext`, kept as it is; its egui shell is replaced. | **Kept**, except that the replacement is the native interface of decision 33, not a Tauri shell |
| Decision 2 | 15 October 2026 | Tauri version 2. | **Retired** by decision 33: there is no Tauri |
| Decision 3 | 15 October 2026 | The interface is drawn with HyperText Markup Language and Cascading Style Sheets, for a richly animated interface over many pages. | **Superseded** by decision 33. Its goal, a richly animated interface over many pages, is kept |
| Decision 4 | 15 October 2026 | Security is paramount. Being hacked is not acceptable under any circumstances. | **Kept** |
| Decision 5 | 15 October 2026 | Pages are shown as images; images travel inside the application and never over a local network port. | **Kept** |
| Decision 6 | 15 October 2026 | The page palette is 32 colours (the Colour Map of Presentation Level 2.5); colours are exact, so no lossy image format. | **Kept** |
| Decision 7 | 15 October 2026 | Many pages are shown on screen at once. | **Kept** |
| Decision 8 | 15 October 2026 | A new frame of a page replaces the whole image. | **Kept** |
| Decision 9 | 15 October 2026 | A tab has the same security behaviour as a tab in a traditional browser: origins, cookies, isolation. | **Kept** |
| Decision 10 | 15 October 2026 | Plain HyperText Transfer Protocol, without Transport Layer Security, is allowed. | **Kept** |
| Decision 11 | 15 October 2026 | Two tabs on the same channel share cookies and a decoder. | **Kept** |
| Decision 12 | 15 October 2026 | Only teletext resources are loaded, over the HyperText Transfer Protocol and WebSocket, with the standard server push technologies. | **Kept** |
| Decision 13 | 15 October 2026 | The declared media type is never used to accept or reject a resource; the content decides. | **Kept** |
| Decision 14 | 15 October 2026 | Channel descriptors, contents files, fonts and HyperText Markup Language discovery are supported. | **Kept** |
| Decision 15 | 15 October 2026 | An update mechanism with no scope for failure or for leaving the application unusable. | **Kept** |
| Decision 16 | 15 October 2026 | Update metadata uses The Update Framework, with threshold signatures and expiring metadata. | **Kept** |
| Decision 17 | 15 October 2026 | Updates are hosted on a dedicated subdomain. | **Kept** |
| Decision 18 | 15 October 2026 | The hardened process model: a trusted core with no web engine, the interface in a process with no rights and no secrets, security indicators in a trusted strip. | **Kept in shape**, with the interface process of section 6 in place of the web-engine window host |
| Decision 19 | 15 October 2026 | The interface is a React application in the webview. | **Superseded** by decision 33 |
| Decision 20 | 15 October 2026 | How the interface is written must make no difference to security; every property holds if the interface runs an attacker's code. | **Kept**, with the exceptions section 6.1 states for the trusted strip |
| Decision 21 | 15 October 2026 | Superseded the same day by decision 22. | — |
| Decision 22 | 15 October 2026 | Fetching never depends on viewing. | **Kept** |
| Decision 23 | 15 October 2026 | Channels with only separate page files fetch each page when it is selected, as an accepted exception to decision 20. | **Kept** |
| Decision 24 | 15 October 2026 | The recommendations for open decisions 1 and 3 to 13 are accepted. | **Kept**, except item (1), reuse of `neoteletext-browser`, superseded by decision 36, and item (9), the palette-index canvas, which decision 33 fulfils |
| Decision 25 | 15 October 2026 | Updates are hosted at `update.neoteletext.com`. | **Kept** |
| Decision 26 | 15 October 2026 | Every limit on fetching, refreshing and input is defined once, in neoteletext, as is the fetching policy of decisions 22 and 23. | **Kept** |
| Decisions 27 to 32 | 15 October 2026 | The window host holding the system webview, its Linux confinement, the nested display, T42 Browser's own display host on Smithay, and input methods and screen readers through the display host. | **Retired** by decision 33: each existed only to contain a web engine. Their measurements stay in `../prototypes/docs/proofs.md` as the record of why no web engine was kept |
| Decision 33 | 16 October 2026 | **The native architecture being built in `native/` is the architecture going forward.** The interface is drawn by T42 Browser's own Rust code with no web engine: an interface core that draws only through its own drawing interface, with back ends for the processor (`vello_cpu` and Parley) and for microcontrollers (`embedded-graphics`); a separate interface process with no secrets, which draws the window and the trusted strip; the trusted core, one decoder per channel and one fetcher per web channel, each a process of its own; frames on the pipe as indexed Portable Network Graphics images. The earlier approaches and their prototypes are kept in `prototypes/`. | — |
| Decision 34 | 15 October 2026 | **Targets and constraints:** microcontrollers are a target, with features cut; every target must work with no graphics processor, a graphics processor being a welcome extra; every library must be under a totally open licence. | — |
| Decision 35 | 15 October 2026 | **The window,** built to the owner's mockups: the window frame drawn as Chrome draws it; labels in English; text in Roboto, in small capitals; page banks, a page list and a page area, with an overview of every page along the bottom; pages drawn flat only; a logarithmic page-size control that shrinks the pages over its whole travel and shows a list only at its left edge; a click on a page zooms it, and at the control's right end the wheel moves from page to page. Section 11 describes the window. | — |
| Decision 36 | 15 October 2026 | **The engine is maintained in its own repository** and used by a pinned version, never copied or patched here. Only its core is used: `neoteletext-core`, `neoteletext-formats`, `neoteletext-image` and `neoteletext-web`. Its browser shell (`neoteletext-gui`, `neoteletext-browser` and its application crate) is not used. | Amended by decision 37: its audio crates are used too |
| Decision 37 | 19 October 2026 | **Sound,** decided while the owner was away, as the owner had instructed. The engine's audio profile (its area AUD, decisions E.84 to E.86) is played on macOS. The decoder runs the engine's audio receiver (`neoteletext-audio`), which takes each packet before the teletext decoder; the trusted core passes on only the active tab's voice operations and checks each again; the interface mixes with the engine's `neoteletext-audio-mix` and plays through `cpal` 0.18.2 (Apache-2.0), a dependency on macOS only, because on Linux it links the system's LGPL `libasound` (open decision D3 of the engine's `audio-platforms.md`; decided by decision 40). A page's sounds play while it is open: the zoomed page, or else the page chosen by number or in the list. Page sounds are on by default; their setting, the volume and mute are not built yet. Section 6.4 lists the messages; requirements 16.1 to 16.6. | Amended by decision 38: every tab is heard; and by decision 40: sound on every desktop |
| Decision 38 | 19 October 2026 | **Every tab is heard, and each can be muted,** as in Chrome, decided while the owner was away, as the owner asked. The trusted core passes on every channel's voice operations, tagged with the channel; the interface mixes one engine mixer per channel and plays their sum. A tab whose channel plays sound shows a speaker that mutes it, and a muted tab a crossed-out one that unmutes it; a button beside the menu mutes every tab, or unmutes every tab once all are muted. The trusted core keeps each tab's mute, which follows the tab to its next channel. A background tab keeps the page it had open, and its sound. The window's title is the selected tab's, marked "Audio playing" or "Muted" as Chrome marks it. Live sounds (the engine's REQ-AUD-014, decision E.87) play too: the trusted core ticks every decoder, which keeps the live sounds' clock. Requirements 16.3, 16.7 to 16.10. | — |
| Decision 39 | 19 October 2026 | **A Dock menu on macOS, and the one crate allowed `unsafe` code,** decided while the owner was away. Right-clicking the Dock icon lists the tabs, the selected one ticked and those playing sound or muted marked, then "Mute All Tabs" or "Unmute All Tabs"; choosing a tab selects it. AppKit offers a Dock menu only through the application delegate's `applicationDockMenu:`, and winit owns the delegate, so the `dock-menu` crate adds that method to the delegate's class before the application launches, which takes Objective-C runtime calls. It uses the `objc2` 0.5.3, `objc2-foundation` 0.2.2 and `objc2-app-kit` 0.2.2 that winit already uses, so no crate is added to the build. The crate denies rather than forbids `unsafe_code`, allows it in its macOS module alone, and gives every unsafe block its reason; every other crate still forbids it (requirement 10.12, amended). It runs in the interface process, which holds no secrets. | — |
| Decision 40 | 19 October 2026 | **Sound on every desktop.** The owner accepted linking the system's LGPL `libasound` on Linux (the engine's open decision D3), dynamically, as `cpal` does. `cpal` is now a dependency on every platform: Core Audio on macOS, WASAPI on Windows, ALSA on Linux, where building needs the ALSA headers (`libasound2-dev` on Ubuntu). With no output device the browser still runs silent. | — |
| Decision 41 | 20 October 2026 | **Every open recommendation is accepted as a decision.** Open decisions 1 to 8 of section 16, as recommended there: Transport Layer Security from the engine or else `rustls`; `wasmi` as the decoder's wall on iOS, its cost measured on desktops first; the trusted strip stays in the interface process, revisited if fuzzing finds faults; our own widgets; the microcontroller build as section 13; the libraries of `native/Cargo.toml` approved as built; the browser licensed under the MIT licence, as the engine; and hyperlinks adopted as `hyperlinks.md` proposes, with its ten decisions as recommended, its phase 1 to be built next. Requirement 4.5 is amended so a link may start a page-started navigation, which leaves the channel only through the site-change screen. | — |
| Decision 42 | 20 October 2026 | **The only underlining is for links, and the software draws it, never the page.** Teletext's underline attribute is not drawn on any page the browser shows: the engine's `underline` render option (its decision E.88) is turned off, so no page code, TTI line, DRCS glyph or attribute can produce the browser's link underline. The link underline is drawn by the interface process over the page image, at screen resolution, as a hairline finer than any teletext dot, so that no page can produce it either; it is not drawn by the engine's renderer into the teletext image, which a page could imitate dot for dot. The trusted core decides what is a link; the engine decides nothing about links. | — |
| Decision 43 | 20 October 2026 | **Hyperlinks built, with what building them changed,** for the owner to confirm (`hyperlinks.md` section 19). Chief among them: links are drawn when a teletext dot is at least one and a half device pixels, not two, since two showed no links on a page at its own size on an ordinary screen; the pointer over a link that leaves is the system's shortcut shape; the link blue is `#3D8BFF`; and on Linux the hand-off uses `xdg-open` until a D-Bus crate is approved. Not built: Fastext, screen readers, and offering the web browser when a teletext address turns out not to be teletext. | — |

---

## 3. Concepts and terms

| Term | Meaning in this project |
|---|---|
| **Teletext page** | One screen of teletext: 40 columns by 25 rows of character cells, row 0 being the header. Identified by magazine (1 to 8), page number (00 to FF) and subcode. |
| **Subpage** | One of several versions of a page carrying the same page number, distinguished by subcode. |
| **Channel** | One teletext service. On the web it is identified by its **origin** and its **folder path** (`neoteletext` decision E.70, `neoteletext/docs/channels.md`): a folder holding `channel.json`, `contents.txt`, optionally a bundle `index.t42`, and page files such as `101.tti`. A file channel is a recording or a folder of page files. The channel is the unit of trust and isolation: it plays the part a site plays in a traditional browser. |
| **Origin** | The combination of scheme, host and port of an address. |
| **Tab** | One view onto one channel at a time, with its own history. |
| **Bank** | A hundred pages, such as 100 to 199: teletext's convention gives each section a hundred, with the section's index on its first page. The window shows the eight banks 100 to 899. |
| **Trusted core** | The process that holds everything the user trusts: which channel each tab shows, history, bookmarks, settings, and the decisions about addresses. It contains no web engine and draws nothing. |
| **Interface process** | The process that draws the window: tabs, the trusted strip, the banks, the page list, the page area and the overview. It holds no secrets, opens no network connection and no file, and reads only what the trusted core sends. |
| **Interface core** | The library, without the standard library, that holds the window's state, layout, animation, hit-testing and input handling, and draws only through its drawing interface. |
| **Drawing back end** | An implementation of the drawing interface: `vello_cpu` and Parley on the processor for desktops and phones, `embedded-graphics` for microcontrollers. |
| **Trusted strip** | The toolbar along the top of the window: Back, Forward, the encryption state, the address with its host emphasised, the bookmark star and the menu. It shows only what the trusted core sends, and its input reaches the trusted core as strip input, message kinds that page intents never use. It is drawn by the interface process (section 6.1). |
| **Fetcher** | The process that holds one web channel's network connections and reads its network formats through the engine's web module. |
| **Decoder** | The process that recognises, decodes and renders one channel's teletext. It has no network and no secrets. |
| **Frame** | One rendered page: palette indices plus its palette, produced by the decoder. |
| **Resource role** | The purpose for which a resource was requested: teletext data, channel descriptor, contents file, font, or discovery. A response is read only by the reader for the role it was requested for. |
| **Site-change screen** | The screen shown in the trusted strip when a tab is about to leave its channel for another because of something a page did. |

---

## 4. Design principles

1. **Assume every part can be compromised, and make each compromise worth as little as possible.** No architecture can promise never to be hacked. This one is arranged so that compromising a single component gains an attacker no secrets, no files and no network it did not already have.
2. **Untrusted bytes are read only in processes with no rights.** Everything a server or a file sends is read by strict, bounded readers in the fetcher or the decoder. The trusted core routes bytes and re-checks results as typed data; it never parses network or teletext formats itself.
3. **Nothing in the interface parses data from outside.** The interface process receives only images our own encoder made from checked palette indices, text the trusted core cleaned and bounded, and state. It contains no web engine, no script engine and no interpreter.
4. **The interface's code matters to security only where it draws the trusted strip** (section 6.1). Every other property holds even if the interface process runs an attacker's code (decision 20).
5. **Every rule lives in Rust, once.** Navigation rules, address display rules and the limits of every message are written once and tested once, for every platform. Every limit on fetching and input is the engine's (decision 26).
6. **Nothing is downloaded and executed.** No page, channel or server ever supplies code.
7. **One interface core for every target.** The window is written once, without the standard library, and drawn through a small drawing interface, so the same code runs on desktops, phones and microcontrollers.
8. **Drawing on the processor is the minimum.** Every target works with no graphics processor (decision 34).
9. **Updates can fail without harm.** Every failure during an update leaves a complete, working version of the application.

---

## 5. The teletext engine and what is reused

The engine is `neoteletext` (`~/devel/neoteletext`), a Rust implementation of World System Teletext, EN 300 706, Presentation Levels 1.0, 1.5, 2.5 and 3.5. Its core is `no_std` (it does not depend on the Rust standard library) and performs no input or output of its own: it is fed bytes and returns results.

**It is a separate project** (decision 36). T42 Browser depends on its crates at one pinned commit, `f341e94`, through a `git` source in the engine's local repository, because that commit is not yet published; Cargo reads the commit, never the engine's working tree. When the commit is published, the source becomes the engine's own repository address at the same commit. Changes the engine needs are made in the engine's repository by its own process, never patched here.

| Crate | What it does | Used by T42 Browser |
|---|---|---|
| `neoteletext-core` | Packet decoding, page store, Presentation Levels 1.0 to 3.5, rendering a page to palette indices, flash timing | In the decoder; its flash timing constants in the trusted core; its page addresses in the fetcher. Built with the engine's recommended `modern-classic` profile (its decision E.71) |
| `neoteletext-formats` | Readers for T42 recordings, Teletext Text Interchange page files, transport streams and sliced records; content detection | In the decoder, with the `tti`, `transport` and `sliced` features |
| `neoteletext-image` | Palette indices to colour formats; the Portable Network Graphics writer | The writer in the trusted core; the palette in the decoder |
| `neoteletext-web` | The strict address parser, origins, the HyperText Transfer Protocol version 1.1, Server-Sent Events, WebSocket, the cookie store, channel descriptors, contents files, push subscriptions, and the user agent that opens and fetches a channel, with no network input or output of its own | In the fetcher, with the `push` feature |
| `neoteletext-slicer` | Recovering teletext from sampled video | Not yet |
| `neoteletext-browser`, `neoteletext-gui`, the application crate | The engine's own browser shell | **Not used** (decision 36) |

### 5.1 Changes required in neoteletext

T42 Browser defines no limit and no fetching behaviour of its own (decision 26). Everything below is work in neoteletext, recorded with its questions in `neoteletext/docs/t42-browser-open-questions.md`.

| Change | T42 Browser requirements |
|---|---|
| A grace period during which a closed tab's channel keeps refreshing | Requirement 5.17 |
| A threshold of malformed packets after which a live feed is closed | Requirement 6.5 |
| A default refresh schedule for a channel that declares none, if none exists | Requirement 5.16 |
| The cookie store separable from the user agent, so the trusted core can own it | Requirement 7.5 |
| A page's text, cleaned of control, bidirectional and zero-width characters and bounded, for display and accessibility | Requirements 11.5 and 12.9 |
| Transport Layer Security for the engine's network port, or a statement of which library T42 Browser should supply it with | Requirement 5.8 |

**Limits T42 Browser takes from neoteletext.** Among them: `contents::MAX_PAGES` (800 pages), `contents::MAX_BYTES` (64 kilobytes), `DEFAULT_PAGE_LIMIT` (256 kilobytes), `DEFAULT_PUSH_LIMIT`, `DEFAULT_DESCRIPTOR_LIMIT`, `DEFAULT_HEADER_LIMIT`, `DEFAULT_FETCH_INTERVAL_US` (one request every 0.2 seconds), `DEFAULT_CONNECT_TIMEOUT_US` (10 seconds), `DEFAULT_RESPONSE_TIMEOUT_US` (30 seconds), and `MIN_REFRESH_SECONDS` and `MAX_REFRESH_SECONDS`. The full table is in `neoteletext/docs/t42-browser-open-questions.md` section 3.

---

## 6. Processes and isolation

### 6.1 The process model

Decision 18's shape, with the interface process in place of the web-engine window host (decision 33).

```
┌──────────────────────── trusted core (Rust, no web engine, draws nothing) ────────────────────────┐
│ owns: tabs, history, bookmarks, settings, address rules, which pages are visible, frame checking, │
│ image encoding, the state of the trusted strip; reads file channels and passes them on unread     │
└──────┬─────────────────────────────┬──────────────────────────────────┬───────────────────────────┘
       │ framed pipe                  │ framed pipe                       │ framed pipe
       │ contents, page bytes,        │ bytes, shown pages, ticks in;     │ tabs, strip, channel, frames out;
       │ notices in                   │ arrivals, updates, frames out     │ intents and strip input in
┌──────▼───────────────────┐  ┌───────▼─────────────────────────┐  ┌──────▼──────────────────────────────┐
│ fetcher, one per web      │  │ decoder, one per channel         │  │ interface process                    │
│ channel                   │  │ content detection, readers,      │  │ the window, tabs, trusted strip,     │
│ the engine's user agent;  │  │ decoding, Presentation Levels    │  │ banks, page list, page area,         │
│ network connections and   │  │ 1.0 to 3.5, rendering            │  │ overview; interface core drawn by    │
│ protocol readers          │  │ no network, no file writes,      │  │ vello_cpu and Parley                 │
│ network, no file writes   │  │ no secrets                       │  │ no network, no file writes, no       │
│                           │  │                                  │  │ secrets                              │
└───────────────────────────┘  └──────────────────────────────────┘  └──────────────────────────────────────┘
       still to come: updater and launcher (section 12); a font helper when fonts are fetched (requirement 3.5)
```

| Process | Holds | May access | If compromised, the attacker gains |
|---|---|---|---|
| **Trusted core** | Everything the user trusts | Files the user opens as channels; starting the other processes | Everything; this is why it reads no network or teletext format and contains no web engine |
| **Fetcher** | One channel's network connections and that channel's cookies | The network, which is its job | The ability to send and receive on the network, and that channel's cookies; no other channel's cookies, no history, no screen |
| **Decoder** | One channel's pages | Nothing it needs; see section 6.5 for what the sandbox still allows | Pictures of its choosing in that channel's page images, which its server could draw anyway |
| **Interface process** | What is on screen | The window, input and the audio output; see section 6.5 | Everything on screen, including a false trusted strip, and the ability to send intents and **strip input**: it can navigate a tab to any address the trusted core's rules accept, go back and forward, and set bookmarks. No secrets, because none are sent to it |

**The trusted strip and the interface process.** The strip is drawn by the interface process, in its own window, outside the page area (design section 5.2, option 1). That keeps the strip out of reach of every page image and every other process's window. It does not keep it out of reach of a fault in the interface process itself: a compromised interface process can draw a false strip and send strip input. Reaching it needs a fault in T42 Browser's own interface code or its drawing libraries, fed only by our own images and cleaned text (`security-analysis.md` section 5.6). Requirement 3.9's hostile interface harness tests every other property with an attacker's interface in its place.

### 6.2 Why bytes pass through the trusted core

The fetcher never talks to a decoder directly. The trusted core passes every page the fetcher delivers to the decoder of that fetcher's channel, because only the trusted core knows which tab shows which channel. It never parses those bytes: it cuts them into pieces within the message limit and passes them on. File channels are the same: the trusted core reads the file and passes its bytes, unread, in pieces of 64 packets every 0.2 seconds, the engine's fetch interval.

### 6.3 Decoders and fetchers per channel

- **Every channel has its own decoder,** and every web channel its own fetcher, so no two channels share a process that reads their data.
- **Decision 11 says tabs on the same channel share one decoder and one cookie store.** Today every tab and every navigation opens its own channel, with its own decoder and fetcher; the protocol is the same either way, so sharing is a later step.
- **A decoder and its fetcher are stopped** when no tab uses their channel any more.

### 6.4 Messages between processes

Every process speaks the same framing, the engine's: one kind byte, a 32-bit big-endian length, then the payload. The framing and every message are defined once, in the `protocol` crate.

- **Every length is bounded before anything is allocated:** at most 256 kilobytes a message, the engine's `MAX_FRAME`, and tighter bounds per field (titles, addresses, page counts, palette sizes).
- **Every payload is parsed strictly** to its exact length.
- **Each side accepts only the kinds meant for it,** so no process can send what only another may say.
- **Text that came from outside** (page titles, bank names, notices, addresses) is cleaned on both sides: control characters, bidirectional controls and zero-width characters removed, and cut to its bound (requirement 11.5).
- **A process that breaks the protocol is closed.** A decoder or fetcher that does so is stopped and its channel's pages stay as they were; an interface that does so ends the application; a malformed message from the trusted core ends the interface.

**From the trusted core to a decoder** (`protocol::decoder`):

| Message | Meaning |
|---|---|
| Data | Bytes of the channel, at most 64 kilobytes, passed on unread |
| End | The channel's data has ended; finish what is buffered |
| Showing | The pages the interface shows now, to render |
| Tick | One flash unit has passed: render the shown pages again, and move the live sounds' clock on; sent to every decoder |
| Close | Stop |
| Opened | The page the user opened, or none: its sounds play while it is open |

**From a decoder to the trusted core:**

| Message | Meaning |
|---|---|
| Arrived | A page was received completely for the first time: its number and a title from its first text row |
| Updated | A page already held arrived again with different rows |
| Frame | A rendered page: 480 by 500 palette indices and a palette of at most 32 colours, every index checked against the palette on arrival |
| Audio | One voice operation of the channel's audio receiver: define a sound, its 16-bit samples (at most 32 768 a message), forget it, announce a live sound and give its next samples, or start, stop or ramp one of four channels; every field bounded (requirement 16.4) |
| Links | The links found in a page, before its image when they change: at most 32 runs (page numbers in its text and its `LT` lines, each a place and either a page number or a target as written, at most 256 bytes, never cut) and its `LK` lines; and whether an image follows (`hyperlinks.md` section 8.1) |

**From a fetcher to the trusted core** (`protocol::fetcher`):

| Message | Meaning |
|---|---|
| Contents | The channel's contents file as the engine read it: section headings and page entries with titles, at most 1,000 entries |
| Page | A page's bytes, fetched or pushed, passed to the decoder unread |
| Notice | A short report, such as an unreachable address, cleaned and shown in the trusted core's log |
| Links | The descriptor's aliases (at most 16) and named services (at most 16), as written, for the trusted core to parse |

**Between the trusted core and the interface process** (`protocol`):

| Direction | Message | Meaning |
|---|---|---|
| Core to interface | Tabs | Each tab's cleaned title, colour, channel and whether it is muted, and which is active |
| Core to interface | Strip | What the trusted strip shows: the address as the address rules formatted it, the channel's identity, whether it is encrypted or a local file, whether Back and Forward can go anywhere, whether it is bookmarked |
| Core to interface | Channel | The active channel: each page's status and title, and the banks' names |
| Core to interface | Page | A page arrived, with its cleaned title |
| Core to interface | Updated | A page changed |
| Core to interface | Frame | One page image: channel and page, frame version, Portable Network Graphics bytes |
| Core to interface | Audio | A voice operation of a channel, tagged with the channel and checked again by the trusted core; or, which only the trusted core may send, a reset when the channel is gone, or the channel's mute |
| Core to interface | Link: page links | Where each link of one image of a page lies and what kind of place it leads to, never its target; and the page numbers it remaps |
| Core to interface | Link: pointing, prompt | What the strip shows of the link under the pointer or the focus; the site-change screen or the web-browser prompt, with its number |
| Core to interface | Link: show, aliases | Open a page a link led to; the page numbers the channel's descriptor links elsewhere |
| Core to interface | Close | The application is ending |
| Interface to core | Visible | The pages the interface's layout puts on screen |
| Interface to core | Cell | A click on a page's cell: page, the version of its image on screen, column and row |
| Interface to core | Point, typed, focused | The link cell under the pointer or the focus, or none; a page number the page or channel links elsewhere; whether the window has the keyboard focus |
| Interface to core | Select tab, close tab, new tab | Tab changes |
| Interface to core | Opened | The page the user opened in the active tab, passed to that channel's decoder |
| Interface to core | Mute tab, mute all | Mute or unmute one tab; mute or unmute every tab |
| Interface to core | Strip input: navigate, back, forward, bookmark, confirm, cancel | What the user did in the trusted strip, including the answer to its question, which names the question. These are message kinds of their own; a page intent reaches another channel only through the site-change screen |

### 6.5 Isolation on each platform

What is built today is in **bold**; the rest is required.

| Platform | Trusted core | Fetcher | Decoder | Interface process |
|---|---|---|---|---|
| macOS | **Not sandboxed.** Required: the App Sandbox, with access only to its data directory and files the user chooses | **`sandbox-exec` profile: network allowed, file writes denied.** Required: file reads denied too | **`sandbox-exec` profile: network and file writes denied.** Required: file reads denied too | **The decoder's profile: network and file writes denied.** Required: file reads limited to its own resources and fonts |
| Windows | Not built | AppContainer with the internet-client capability only | AppContainer without it, low integrity, job object | AppContainer without the internet-client capability |
| Linux | Not built | Landlock and a seccomp filter | Landlock and a seccomp filter | Landlock, an empty network namespace and a seccomp filter; the display is the only service it needs |
| Android | Main application process | Separate service process | Isolated service process with a seccomp filter | The application's activity |
| iOS | One application process | Inside it | Inside it; the WebAssembly interpreter `wasmi` as its wall (open decision 2) | Inside it |
| Microcontrollers | One program; a single-purpose device, with its own analysis (section 13) | | | |

**The macOS profiles today start from "allow everything" and deny network and file writes.** They stop the two things that matter most, sending data away and changing the user's files, but they still allow reading the user's files. A profile that starts from "deny everything" is required (requirement 3.7).

**On Linux there is no confinement of the interface beyond the platform's,** because there is no web engine to contain: the interface process is T42 Browser's own code, and it needs the display as any application does. Under X11 any application can send input to other applications' windows (`../prototypes/docs/proofs.md` section 6.8); that is the exposure of every X11 application, not a containment problem.

---

## 7. Network access

### 7.1 What may be fetched: resource roles

The application loads only resources that are part of a teletext service (decisions 12 and 14). Each is fetched in exactly one role and read only by the reader for that role.

| Role | Read by | Read in | What the trusted core receives |
|---|---|---|---|
| **Teletext data**: page files, bundles, the other formats of `neoteletext-formats`, and pushed pages | Content detection and the format readers | Decoder | Arrivals, updates and frames |
| **Channel descriptor** (`channel.json`) | The engine's descriptor reader | Fetcher | Nothing yet: the fetcher uses it to find the contents file and the push feed |
| **Contents file** (`contents.txt`) | The engine's contents reader | Fetcher | A bounded, typed list of sections and pages |
| **Font** | The engine's strict font reader | A confined font helper, not built | — |
| **Discovery** (HyperText Markup Language) | The engine's discovery reader | Fetcher, not built | — |

Discovery has its own security analysis in `neoteletext/docs/web-discovery-security.md`. Its results are never loaded directly: each is a page-initiated navigation through the site-change rules.

### 7.2 Transports

- **The HyperText Transfer Protocol version 1.1**, plain today. Transport Layer Security is not built, so `https` addresses are reported unreachable; it needs a library, which is the owner's choice (open decision 1). The trusted strip shows "Not secure" for a channel reached without Transport Layer Security (decision 10).
- **Server-Sent Events** carry the channel's push feed. Built: the fetcher subscribes to all of a channel's pages and passes each pushed page to its decoder.
- **WebSocket** push is in the engine and not yet wired to the fetcher's sockets.
- **Not supported:** the push mechanism of the HyperText Transfer Protocol version 2, which traditional browsers have removed.

### 7.3 Rules that apply to every request

These are the engine's user agent's rules, which the fetcher uses as they are:

- **Addresses on this computer and the local network are refused** unless the user names a specific address, as the engine's requirement REQ-SEC-006 requires. Today the user names it with `--allow-local` on the command line; required is a setting in the trusted strip.
- **Compression is undone with a fixed limit** on the decompressed size.
- **The declared media type is ignored** for acceptance (decision 13).
- **Every response has limits** on size, time and redirects, all the engine's constants.
- **No `Referer` header is sent,** and only the engine's minimal user agent profile.

### 7.4 Fetching never depends on viewing

Decision 22. The pattern of network requests must reveal nothing about which pages are viewed.

| What the channel publishes | How its pages are fetched |
|---|---|
| A bundle, `index.t42` | The whole bundle when the channel opens |
| A contents file, `contents.txt` | Every page file it lists, in the order it lists them, one every 0.2 seconds, when the channel opens. **Built**, by the engine's user agent |
| Only separate page files | Each page when it is selected (decision 23), the one exception |
| A file on this computer | All of it, in its own order, one piece every 0.2 seconds, whatever is on screen. **Built** |

- **Refreshing** follows the channel's push feed, its declared schedule, or a fixed default, never which page is shown.
- **Keys and interface requests only select among pages already held.** The interface's only influence on the decoder is which pages it renders, which never reaches the network.
- **Closing a tab** must keep its channel refreshing for a grace period defined in neoteletext (section 5.1). Not built: closing a tab stops its fetcher at once.

---

## 8. Content detection

A response in the teletext data role is recognised by its content alone, by the engine's `AutoReader` and `neoteletext-formats` detection (the engine's requirement REQ-INP-005):

1. Signatures are tried first: Teletext Text Interchange files and the other signed formats.
2. Then the headerless binary layouts, such as T42 packets, are scored; a layout is accepted only when most of its packet addresses decode correctly under the standard's Hamming 8/4 protection.
3. Detection waits until it has enough bytes, so its decision does not depend on how the data was split.

**Rules of this architecture:**

- **Detection runs in the decoder,** the process with no rights.
- **"Teletext only" means "the decoder recognised it".** Bytes the decoder does not recognise produce no page; nothing else ever reads them.
- **Data crafted to pass detection is teletext by definition,** and is then subject to every bound of the engine's readers and decoder.

---

## 9. The security model of a tab

The aim is the security behaviour of a tab in a traditional browser (decision 9). Teletext pages run no scripts, so several browser protections become simpler or stricter.

### 9.1 Origins and channels

- **Origin:** scheme, host and port, as the engine defines it.
- **Channel:** origin plus folder path (`neoteletext` decision E.70), the unit of isolation. It is finer than a traditional browser's site, so no list of public suffixes is needed.

### 9.2 The equivalent of the same-origin policy

- A channel's pages, cookies and history are never mixed with another channel's.
- A decoder receives only its own channel's bytes, and a fetcher fetches only its own channel.
- A page cannot cause a request to another channel. Anything that would is a navigation through the site-change rules.

### 9.3 Cookies

The cookie store is the engine's, which follows the cookie specification draft RFC 6265bis as browsers implement it, made stricter: every cookie scoped to its host only, `Secure` cookies never sent over plain connections, the `__Host-` and `__Secure-` prefixes and expiry enforced, and no third-party context at all.

**Where it lives.** Today each fetcher holds its own channel's cookie store, inside the engine's user agent, in memory, with the engine's default sizes. A compromised fetcher therefore holds only its own channel's cookies. Required: the trusted core owns the store and gives each request only its cookies (requirement 7.5); that needs the engine change of section 5.1.

### 9.4 Storage

- The decoder's page store belongs to one channel.
- The trusted core's record of frames sent is keyed by channel and page.
- The interface process stores nothing: no cookies, no files, no cache.
- **History and bookmarks are kept in memory only** today; nothing is saved between runs.

### 9.5 Navigation and the site-change screen

- **Navigations the user starts:** the address field, Back, Forward, a new tab, and in time bookmarks and Home. Built.
- **Navigations a page starts:** Fastext keys, page links, redirects, discovery results. Not built; a click on a page's cell is reported to the trusted core, which does not yet follow links.
- **Any navigation a page starts that leaves the channel** must go through the site-change screen, in the trusted strip, with the user's confirmation.
- **The address shown is always the committed address.**

### 9.6 Addresses

The trusted core's address rules (`core/src/address.rs`) decide what may be opened and how the strip shows it:

- `https` and `http` web addresses, with `https` assumed when no scheme is typed;
- plain hosts only, letters, digits and hyphens, until internationalised names can be shown safely (requirement 11.3);
- no user name before the host, the classic way to make an address look like another site's;
- a port, if given, must be a number from 0 to 65535;
- no hidden characters anywhere;
- `file://` addresses and absolute paths for files on this computer, shown as "File";
- at most the address bar's limit in length.

### 9.7 Transport Layer Security

Not built (section 7.2). When it is: certificates are checked by the fetcher, a failed check is a hard stop with no way to proceed (decision 24), and the strip shows whether the channel arrived encrypted.

### 9.8 Local files

Each file is its own channel, with no cookies. It is read by the trusted core and decoded by its own decoder, never parsed by the trusted core. A file is limited to 64 megabytes, and a folder to 4,096 page files.

---

## 10. How a page reaches the screen

1. **The decoder renders a frame:** 480 by 500 palette indices (40 columns of 12 dots, 25 rows of 20 lines) and a palette of at most 32 colours (decision 6), in the engine's resolved-colour mode with its bundled font, Bedstead, at the engine's default presentation level. It renders only the pages the trusted core says are shown.
2. **The trusted core checks the frame:** a known channel and page, the exact raster size, a palette of at most 32 entries, every index within the palette. Frames for pages the interface does not show are dropped.
3. **The trusted core encodes an indexed Portable Network Graphics image** with the engine's writer, unless the frame is identical to the last one it sent for that page.
4. **The trusted core sends the image to the interface process,** with the page's key and a frame version.
5. **The interface process reads it** with a strict reader that accepts only what our encoder writes: indexed, 8 bits a dot, exactly 480 by 500, at most 32 colours, with a bound on the memory its decoding may take. It keeps the page's colours until a newer frame replaces them whole (decision 8), and draws the page scaled into its rectangle with nearest-neighbour sampling.
6. **Flash** is rendered by the decoder at the engine's timing: the trusted core sends it a tick every flash unit (one second in twelve, the engine's decision E.48), the decoder renders the shown pages again, and step 3 passes on only the frames that changed.

**Showing many pages at once (decision 7).** The interface knows exactly which pages its layout puts on screen and reports them; the trusted core asks the decoder only for those, of the active channel only. A channel of 800 pages costs what is visible.

**Why images, not raw indices, on the pipe.** A page image is about 6 kilobytes (`../prototypes/docs/proofs.md` section 4), already written by the engine, and the interface's reader sees only files our own encoder produced. In one process (iOS, microcontrollers) frames go from decoder to interface as indices, with no encoding.

---

## 11. The browser window

The window is decision 35's, drawn by the interface process from the interface core.

### 11.1 Layers

```
 interface core (ours, no standard library)
   tabs and strip · page banks · page list · page area · overview
   layout · animation · hit-testing · the address editor · input
   draws only through the drawing interface below
        │
 drawing interface (ours, small: rectangles, rounded rectangles,
                    shadows, images in rectangles, text runs, dimming)
        │
        ├── vello_cpu + Parley ── softbuffer ── winit     desktops and phones (the minimum)
        ├── Vello on the graphics processor ── winit       optional, where it helps (not built)
        └── embedded-graphics + bitmap fonts               microcontrollers, features cut
```

- **Redrawing is on demand:** only on input, a change of state, or while something animates.
- **Widgets are our own** (design section 4.3, option B): the address field's editor is in the interface core, so it works without the standard library; text is laid out by Parley. Masonry, the alternative, is not proven (open decision 4).
- **Text** is Roboto, shipped with the application under the SIL Open Font License 1.1, in small capitals: every letter a capital, letters written in lower case drawn 80% as large. Open Sans is shipped too, for comparison.

### 11.2 What the window shows

| Part | What it does |
|---|---|
| Tab strip | Coloured tabs (the colour chosen by the trusted core from the site), close buttons, a new-tab button. On macOS the system's window buttons sit at its left end; elsewhere the window draws its own, the strip drags the window and its edges resize it |
| Trusted strip | Back and Forward, dimmed when history has nothing; a padlock, "Not secure" or "File"; the address with its host in full ink; a bookmark star; a menu button. A click on the address selects it for typing; Enter sends it as strip input |
| Page banks | 100 to 800, each with its colour, range, name (from the channel's contents file) and page count |
| Page list | The chosen bank's pages, number and title; Up and Down step through it; three digits typed choose a page |
| Page area | The bank's pages as a grid, sized by a logarithmic control that shrinks them over its whole travel to 80 pixels a page and shows a list only at its left edge. A click zooms a page to fill the area over 300 milliseconds, the wall dimming behind it; Escape or the close button returns. At the control's right end the zoomed page is shown, and the wheel or Up and Down slide to the next page of the bank, snapping into place: each notch of a mouse wheel is one page, at most one per slide, because macOS reports a slow notch as a tenth of a line. A page under the pointer pops out slightly |
| Overview | All 800 pages in eight rows: empty, available (listed in the contents file but not yet arrived), loaded, shown; the counts. A page that has arrived pops up at its native size under the pointer, which becomes a hand, never covering the trusted strip. A chevron at its top right folds it into the footer, where it draws nothing else and asks for no redraws |
| Updates | A page whose content changes is marked in amber: a ring bursts from its cell in the overview and a dot stays for a minute; its row in the list flashes and shows how long ago it changed; its frame in the page area glows and fades over 3 seconds |

### 11.3 Input

- Pointer, wheel, keys and text from input methods arrive through winit.
- **Cell clicks** are computed from the pointer position and the rectangle the page was drawn in, which the interface core laid out itself, so they are exact.
- **Keys** that teletext needs (page numbers, Fastext colours, reveal) are to reach the trusted core's input handling unchanged; today the interface handles page-number keys in the list itself.

### 11.4 The trusted strip's rules

- It shows only what the trusted core sends: the address as the address rules formatted it, the identity, the encryption state.
- It stays visible in every layout, including the zoomed page.
- Its input is strip input, message kinds of their own (section 6.4).
- The site-change screen, when built, is drawn in the strip only, never in the page area, so a page image cannot imitate it in place.

---

## 12. The application update mechanism

Unchanged from the earlier design (`../prototypes/docs/architecture.md` section 12), and not built. In brief:

- **No failure leaves the user without a working application:** a new version is installed beside the running one, verified against The Update Framework metadata and the platform's code signature, switched to by one atomic rename of a pointer, and rolled back automatically if it fails its health check or fails to start twice. Profiles are copied and migrated, never changed in place.
- **Verification** is The Update Framework (decision 16), implemented by `tough` (decision 24): separate root, targets, snapshot and timestamp roles, a threshold of root keys held offline by three people, expiring metadata, versions that only increase.
- **One launcher scheme on every desktop** (decision 24), so no operating-system package manager bypasses The Update Framework. Phones update through their stores.
- **Hosting** is `update.neoteletext.com` (decision 25), used for nothing else and not trusted; the updater contacts nothing else.

With no web engine the application is far smaller: the interface process is about 5 megabytes with its fonts.

---

## 13. Platforms

| Platform | Window and input | Drawing | State |
|---|---|---|---|
| macOS | winit | `vello_cpu`, shown with softbuffer | **Built and run** on macOS 15.6, Apple silicon; **sound built and run** through `cpal` and Core Audio (decision 37) |
| Windows | winit | as above | **Built, tested and run** on Windows 11 (build 26200) in a virtual machine, 19 October 2026: the whole workspace's tests pass, a page's sound plays through WASAPI (decision 40), and no console window opens. No sandbox yet; the Dock menu has no Windows counterpart (a taskbar jump list) |
| Linux | winit (X11 and Wayland) | as above | **Built, tested and run** on Ubuntu 26.04, 19 October 2026, under X11: the whole workspace's tests pass, and a live sound plays through ALSA (decision 40). No sandbox yet |
| iOS, Android | winit | as above; the graphics-processor back end may matter most here | Later (decision 24); frame time at phone resolutions to be measured |
| Microcontrollers | the board's own display and input drivers | `embedded-graphics`, bitmap fonts | **The interface core drawn into a display in memory** by a test; no build for a microcontroller target yet |

**What the microcontroller build cuts** (to be decided with the owner): smooth text, screen readers, input methods, multiple tabs and windows, developer tools, and probably the network fetcher, depending on where its teletext comes from. It keeps pages, page-number and Fastext keys, flash and a simple page wall. Its security model is that of a single-purpose device and needs its own analysis.

**The display scale.** Every measurement so far was at a scale of 1; drawing on the processor costs about four times as much at a scale of 2.

---

## 14. Code layout

```
t42-browser/
  native/                   the application (a Cargo workspace)
    interface-core/         the window's state, layout, animation and input; no standard library
    draw-vello/             the drawing back end for desktops and phones: vello_cpu and Parley, with the fonts
    draw-embedded/          the drawing back end for microcontrollers: embedded-graphics
    protocol/               every message between processes: framing, bounds, kinds, text cleaning
    core/                   the trusted core (t42-browser): tabs, history, addresses, routing, frames,
                            and hearing.rs, which channel is heard and the checks on its sound
    decoder/                the decoder (t42-decoder): the engine as the decoder uses it
    fetcher/                the fetcher (t42-fetcher): the engine's user agent over the network
    interface/              the interface process (t42-interface): winit, softbuffer, the image reader,
                            and audio.rs, the mixer and the output device
    site/                   a development tool, not shipped (t42-site): builds and serves a test channel
    fuzz/                   fuzz targets for every reader of input from outside a process (nightly, outside the workspace)
  docs/                     this document, the requirements, the security analysis, the design record
  prototypes/               the earlier approaches, kept as the record (decision 33)
    docs/                   the earlier design's architecture, requirements, security analysis, proofs
    spikes/                 the interface and sandbox prototypes
    src/, src-tauri/        the Tauri and React prototype
    standin/                the stand-in engine the prototypes use
    egui-image/             the egui page renderer compiled to WebAssembly
  scripts/check.sh          the one check script: "commit" before each commit, "full" before each push
  scripts/try.sh            the edit loop: Clippy on every save, or one crate's Clippy and tests
  scripts/tools.sh          the pinned development tools, installed into the ignored tools/
  rust-toolchain.toml       the pinned compiler
```

Still to come: the launcher and updater (section 12).

---

## 15. What is built and what is not

**Built** (16 October 2026), with 67 tests:

- The trusted core, the decoder, the fetcher and the interface process, each its own program, communicating only over the bounded protocol of section 6.4.
- The engine at a pinned commit, decoding T42 recordings, Teletext Text Interchange page files, transport streams and sliced records.
- Web channels over plain HyperText Transfer Protocol: the contents file, fetching in its order at the engine's rate, push over Server-Sent Events, update marks in the window.
- File channels, fed at the same pace.
- The window of section 11, drawn on the processor; and the same interface core drawn by the microcontroller back end in a test.
- macOS sandbox profiles for the decoder, the fetcher and the interface.
- The address rules of section 9.6; tabs, history and bookmarks in memory.
- Hyperlinks, phases 1 and 2 of `hyperlinks.md` (20 October 2026), behind the `links` feature, on by default: page numbers found in the text and `LT`, `LK` and descriptor links, decided by the trusted core; the underline drawn by the interface; the destination, the site-change screen and the web-browser prompt in the strip; the hand-off to the system's web browser (`web-opener`); `#302` page addresses.

**Not built:**

- Transport Layer Security and `https` channels; WebSocket push.
- Fastext; the site-change screen for redirects and discovery results; sub-pages beyond the latest; reveal.
- The trusted core's own sandbox; sandbox profiles that deny file reads; any sandbox on Windows, Linux, Android or iOS.
- Sharing one decoder and cookie store between tabs on the same channel (decision 11); the trusted core owning the cookie store.
- The grace period for closed tabs; the live-feed threshold; refresh schedules.
- Saving history, bookmarks and sessions.
- Fonts from channels and the font helper; discovery.
- Screen readers, the menu bar and its shortcuts, the clipboard, multiple windows, developer tools, reduced motion.
- The hostile interface harness and fuzzing of the message readers.
- The updater and launcher.
- Builds on Windows, Linux, phones and microcontroller targets.

---

## 16. Decisions still open

None. Every decision below was taken on 20 October 2026 (decision 41), each as recommended; the questions are kept for reference.

| Number | Question | Decided (decision 41) |
|---|---|---|
| Open decision 1 | Which library gives the fetcher Transport Layer Security? | Ask the engine first (section 5.1); otherwise `rustls`, which is pure Rust and permissively licensed |
| Open decision 2 | Does the decoder also run inside the WebAssembly interpreter `wasmi`, as the earlier design required? It is the decoder's only wall on iOS, where there is one process | Yes on iOS at least; measure its cost on desktops before deciding there |
| Open decision 3 | Is the trusted strip drawn by the interface process, as now, or by a smaller separate process, so that a fault in the interface's drawing cannot forge strip input (`security-analysis.md` section 5.6)? A separate process's window can be covered by other windows (`../prototypes/docs/proofs.md` section 3), so it would have to draw in the same window | Keep it in the interface process; make the interface process as small and as tested as the trusted core's readers, and revisit if the harness or fuzzing finds faults |
| Open decision 4 | Widgets: our own (built), or Masonry, once it draws with `vello_cpu` in a release | Our own, as built; reconsider for screen readers and text fields of teletext forms |
| Open decision 5 | What does the microcontroller build cut, and where does its teletext come from? | Section 13 |
| Open decision 6 | The libraries of `native/Cargo.toml`, each by name (requirement 14.3) | Approve the list as built |
| Open decision 7 | The browser's own licence: it has no `LICENSE` file and no `license` field (`licensing.md` section 1) | MIT, as the engine |
| Open decision 8 | Hyperlinks: the proposal of `hyperlinks.md`, with its ten decisions (its section 16), the amendment of requirement 4.5 among them | Adopt it and build its phase 1 |

---

## 17. Glossary

| Term | Meaning |
|---|---|
| AccessKit | A Rust library that publishes an application's accessibility tree to each platform's screen readers. |
| App Sandbox | The macOS sandbox for applications: a process runs with only the entitlements it is given. |
| AppContainer | The Windows sandbox for applications: a process runs with only the capabilities it is given. |
| Bedstead | The teletext font the engine bundles. |
| Colour Map | In teletext Presentation Level 2.5, four tables of eight colours each, together 32 colours. |
| `embedded-graphics` | A Rust drawing library for small displays, with no standard library; the microcontroller back end. |
| Fastext | Teletext's coloured links: four keys that jump to pages the broadcaster chose. |
| Hamming 8/4 | The error protection teletext uses for addresses: each byte carries four data bits and can have one error corrected. |
| HyperText Transfer Protocol | The request and response protocol of the web. Version 1.1 is used. |
| Input method | The program that turns key presses into text that cannot be typed directly, such as Chinese, Japanese or Korean. |
| Landlock | A Linux mechanism by which a process restricts its own and its children's access to files. |
| Masonry | Linebender's widget library for Rust, an alternative to our own widgets. |
| Network namespace | A Linux process's own set of network interfaces; an empty one has only an unconnected loopback interface. |
| Origin | Scheme, host and port of an address. |
| Parley | A Rust library that lays out and shapes text, with full Unicode and bidirectional support. |
| Portable Network Graphics | A lossless image format with indexed palettes; used for every page image on the pipe. |
| Presentation Level | The teletext standard's feature levels: 1.0 and 1.5 (basic), 2.5 (enhanced colours and characters), 3.5 (further enhancements). |
| RFC 6265bis | The current draft of the Internet Engineering Task Force specification for cookies. |
| `sandbox-exec` | The macOS program that starts another program under a sandbox profile written in the system's sandbox language. |
| seccomp filter | A Linux mechanism by which a process restricts which system calls it may make. |
| Server-Sent Events | A standard way for a web server to push a stream of messages to a client over one HyperText Transfer Protocol response. |
| SIL Open Font License | A permissive licence for fonts. |
| softbuffer | A Rust library that shows pixels drawn on the processor in a window. |
| Teletext Text Interchange | A text file format for teletext pages, used by page editors; the page files of a web channel. |
| The Update Framework | A specification for secure software updates that survive compromised servers and stolen keys, using signed, expiring metadata in separate roles. |
| `tough` | A Rust implementation of The Update Framework. |
| Transmission Control Protocol | The connection protocol the HyperText Transfer Protocol runs over. |
| Transport Layer Security | The protocol that encrypts and authenticates network connections. |
| `vello_cpu` | A Rust two-dimensional renderer that draws on the processor; the desktop drawing back end. |
| wasmi | A WebAssembly interpreter written in Rust, without just-in-time compilation. |
| WebAssembly | A portable binary code format, which an interpreter can run so that a fault stays inside its memory. |
| WebSocket | A protocol for a long-lived two-way connection over the web. |
| winit | A Rust library that opens windows and delivers their input on every desktop and phone. |
| X11 | The older Linux display protocol, in which any client can draw over, read and send input to every other client's windows. |
