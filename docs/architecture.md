# NeoTeletext: architecture

| | |
|---|---|
| **Status** | Adopted 16 October 2026 (decision 33). Built in part, in `native/`; section 15 says what is built and what is not. |
| **Date** | 16 October 2026 |
| **Companion documents** | [`requirements.md`](requirements.md) states every requirement in full, numbered, with its reason, how it is verified and whether it is met. [`security-analysis.md`](security-analysis.md) analyses this architecture's security. [`hyperlinks.md`](hyperlinks.md) specifies hyperlinks (decision 41) and [`hyperlinks-security.md`](hyperlinks-security.md) analyses their security. [`interaction.md`](interaction.md) specifies channel push, typing back and the cursor (decision 44) and [`interaction-security.md`](interaction-security.md) analyses their security. [`licensing.md`](licensing.md) says which licences may sit beside the browser's code. [`native-interface-design.md`](native-interface-design.md) records how this architecture was chosen, the prototypes it was chosen from, its review against the earlier design, and what was built when. |
| **The earlier design** | The Tauri design that came before, with its own architecture, requirements, security analysis and proofs, was kept in `prototypes/docs/`, beside the prototypes that tested it, until they were removed on 21 October 2026 (decision 50); they are in git at the tag `prototypes-final`, and a path under `prototypes/` in this document is a path there. This document replaces it; where the two differ, this one holds. |
| **Engine documents** | The teletext engine's own design lives in `~/devel/neoteletext-engine`: `ARCHITECTURE.md`, `docs/spec/requirements/web.md` (channels, REQ-WEB-023 to REQ-WEB-026, and discovery) and `docs/spec/decisions.md`; its `docs/channels.md` and `docs/web-discovery-security.md` left the engine with its browser on 18 October 2026 (engine commit `94346b0`) and are in its history. This document refers to them by path. |

**Abbreviations.** This project avoids abbreviations, because they hide meaning. Where a technical term is normally known by its initials, the full name is written out, followed by the initials in brackets only when the initials are a literal identifier in code, configuration or a standard's title. Section 17 is a glossary of every technical term used.

---

## Contents

1. What NeoTeletext is
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

## 1. What NeoTeletext is

NeoTeletext is an application for browsing teletext, on desktops, then phones, and with features cut on microcontrollers.

- **Teletext** is the page-based information service defined by the European Telecommunications Standards Institute standard EN 300 706, "Enhanced Teletext specification". A copy is at `~/devel/teletext-reference/teletext-standards/en_300706v010201o.pdf`.
- **Every tab is one teletext channel.** A teletext channel is one teletext service: eight magazines, numbered 1 to 8, each with pages numbered 00 to FF in hexadecimal, and each page with subpages. NeoTeletext shows pages 100 to 899.
- **A channel comes from the web or from a file.** On the web a channel is a folder on a server (the engine's `docs/spec/requirements/web.md`); a file is a recording or a folder of page files on this computer.
- **Every image on screen is one teletext page,** rendered by the teletext engine at the native raster and scaled by the interface.
- **The interface is drawn by NeoTeletext's own Rust code, with no web engine.** It draws on the processor, so it needs no graphics processor, and it shows many pages at once and animates between them.
- **Security is paramount.** The owner has stated that there are no circumstances under which being hacked is acceptable (decision 4). Where a choice trades capability against security, security wins.

---

## 2. The owner's decisions

Each decision was made by the owner on the date given. Requirements in `requirements.md` cite the decision they come from. Decisions 1 to 32 were recorded with the earlier design (`prototypes/docs/architecture.md` section 2), where their full text stays; their numbers are kept here so that every citation still holds.

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
| Decision 14 | 15 October 2026 | Channel descriptors, contents files, fonts and HyperText Markup Language discovery are supported. | **Kept**, except fonts: amended by decision 66, no font is fetched from a channel |
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
| Decisions 27 to 32 | 15 October 2026 | The window host holding the system webview, its Linux confinement, the nested display, NeoTeletext's own display host on Smithay, and input methods and screen readers through the display host. | **Retired** by decision 33: each existed only to contain a web engine. Their measurements stay in `prototypes/docs/proofs.md` as the record of why no web engine was kept |
| Decision 33 | 16 October 2026 | **The native architecture being built in `native/` is the architecture going forward.** The interface is drawn by NeoTeletext's own Rust code with no web engine: an interface core that draws only through its own drawing interface, with back ends for the processor (`vello_cpu` and Parley) and for microcontrollers (`embedded-graphics`); a separate interface process with no secrets, which draws the window and the trusted strip; the trusted core, one decoder per channel and one fetcher per web channel, each a process of its own; frames on the pipe as indexed Portable Network Graphics images. The earlier approaches and their prototypes are kept in `prototypes/`. | Amended by decision 50 (the prototypes removed, kept at the tag `prototypes-final`) and decision 61 (page images in the browser's own form) |
| Decision 34 | 15 October 2026 | **Targets and constraints:** microcontrollers are a target, with features cut; every target must work with no graphics processor, a graphics processor being a welcome extra; every library must be under a totally open licence. | — |
| Decision 35 | 15 October 2026 | **The window,** built to the owner's mockups: the window frame drawn as Chrome draws it; labels in English; text in Roboto, in small capitals; page banks, a page list and a page area, with an overview of every page along the bottom; pages drawn flat only; a logarithmic page-size control that shrinks the pages over its whole travel and shows a list only at its left edge; a click on a page zooms it, and at the control's right end the wheel moves from page to page. Section 11 describes the window. | — |
| Decision 36 | 15 October 2026 | **The engine is maintained in its own repository** and used by a pinned version, never copied or patched here. Only its core is used: `neoteletext-core`, `neoteletext-formats`, `neoteletext-image` and `neoteletext-web`. Its browser shell (`neoteletext-gui`, `neoteletext-browser` and its application crate) is not used. | Amended by decision 37: its audio crates are used too; and by decisions 44 and 48: `neoteletext-qrkeyboard` and `neoteletext-input` |
| Decision 37 | 19 October 2026 | **Sound,** decided while the owner was away, as the owner had instructed. The engine's audio profile (its area AUD, decisions E.84 to E.86) is played on macOS. The decoder runs the engine's audio receiver (`neoteletext-audio`), which takes each packet before the teletext decoder; the trusted core passes on only the active tab's voice operations and checks each again; the interface mixes with the engine's `neoteletext-audio-mix` and plays through `cpal` 0.18.2 (Apache-2.0), a dependency on macOS only, because on Linux it links the system's LGPL `libasound` (open decision D3 of the engine's `audio-platforms.md`; decided by decision 40). A page's sounds play while it is open: the zoomed page, or else the page chosen by number or in the list. Page sounds are on by default; their setting, the volume and mute are not built yet. Section 6.4 lists the messages; requirements 16.1 to 16.6. | Amended by decision 38: every tab is heard; and by decision 40: sound on every desktop |
| Decision 38 | 19 October 2026 | **Every tab is heard, and each can be muted,** as in Chrome, decided while the owner was away, as the owner asked. The trusted core passes on every channel's voice operations, tagged with the channel; the interface mixes one engine mixer per channel and plays their sum. A tab whose channel plays sound shows a speaker that mutes it, and a muted tab a crossed-out one that unmutes it; a button beside the menu mutes every tab, or unmutes every tab once all are muted. The trusted core keeps each tab's mute, which follows the tab to its next channel. A background tab keeps the page it had open, and its sound. The window's title is the selected tab's, marked "Audio playing" or "Muted" as Chrome marks it. Live sounds (the engine's REQ-AUD-014, decision E.87) play too: the trusted core ticks every decoder, which keeps the live sounds' clock. Requirements 16.3, 16.7 to 16.10. | — |
| Decision 39 | 19 October 2026 | **A Dock menu on macOS, and the one crate allowed `unsafe` code,** decided while the owner was away. Right-clicking the Dock icon lists the tabs, the selected one ticked and those playing sound or muted marked, then "Mute All Tabs" or "Unmute All Tabs"; choosing a tab selects it. AppKit offers a Dock menu only through the application delegate's `applicationDockMenu:`, and winit owns the delegate, so the `dock-menu` crate adds that method to the delegate's class before the application launches, which takes Objective-C runtime calls. It uses the `objc2` 0.5.3, `objc2-foundation` 0.2.2 and `objc2-app-kit` 0.2.2 that winit already uses, so no crate is added to the build. The crate denies rather than forbids `unsafe_code`, allows it in its macOS module alone, and gives every unsafe block its reason; every other crate still forbids it (requirement 10.12, amended). It runs in the interface process, which holds no secrets. | Amended: `web-opener` (`hyperlinks.md` decision 7), `confine` (decisions 52 and 56) and `os-install` (decision 75) are permitted unsafe code too (requirement 10.12) |
| Decision 40 | 19 October 2026 | **Sound on every desktop.** The owner accepted linking the system's LGPL `libasound` on Linux (the engine's open decision D3), dynamically, as `cpal` does. `cpal` is now a dependency on every platform: Core Audio on macOS, WASAPI on Windows, ALSA on Linux, where building needs the ALSA headers (`libasound2-dev` on Ubuntu). With no output device the browser still runs silent. | — |
| Decision 41 | 20 October 2026 | **Every open recommendation is accepted as a decision.** Open decisions 1 to 8 of section 16, as recommended there: Transport Layer Security from the engine or else `rustls`; `wasmi` as the decoder's wall on iOS, its cost measured on desktops first; the trusted strip stays in the interface process, revisited if fuzzing finds faults; our own widgets; the microcontroller build as section 13; the libraries of `native/Cargo.toml` approved as built; the browser licensed under the MIT licence, as the engine; and hyperlinks adopted as `hyperlinks.md` proposes, with its ten decisions as recommended, its phase 1 to be built next. Requirement 4.5 is amended so a link may start a page-started navigation, which leaves the channel only through the site-change screen. | — |
| Decision 42 | 20 October 2026 | **The only underlining is for links, and the software draws it, never the page.** Teletext's underline attribute is not drawn on any page the browser shows: the engine's `underline` render option (its decision E.88) is turned off, so no page code, TTI line, DRCS glyph or attribute can produce the browser's link underline. The link underline is drawn by the interface process over the page image, at screen resolution, as a hairline finer than any teletext dot, so that no page can produce it either; it is not drawn by the engine's renderer into the teletext image, which a page could imitate dot for dot. The trusted core decides what is a link; the engine decides nothing about links. | — |
| Decision 43 | 20 October 2026 | **Hyperlinks built, with what building them changed,** for the owner to confirm (`hyperlinks.md` section 19). Chief among them: links are drawn when a teletext dot is at least one and a half device pixels, not two, since two showed no links on a page at its own size on an ordinary screen; the pointer over a link that leaves is the system's shortcut shape; the link blue is `#3D8BFF`; and on Linux the hand-off uses `xdg-open` until a D-Bus crate is approved. Not built: Fastext, screen readers, and offering the web browser when a teletext address turns out not to be teletext. | — |
| Decision 44 | 20 October 2026 | **Channel push, typing back and the cursor adopted** as `interaction.md` proposes, with its twelve decisions as recommended: requirement 4.7 amended for bounded key records on the channel's own WebSocket while a page of the channel is open; every key goes while such a page is open, as from a terminal, with no dialog; plain `ws://` allowed with "Not secure" in the strip; the key record and its table and the cursor record (packet 4/31, data channel 13, checked against EN 300 708 first) in the engine's area INT; rendering on arrival while such a page is open; the cursor a blinking bar or block in an amber chosen on screen; a phone's keys reach the address field only after the viewer puts the caret there; `rustls` and RustCrypto in a keyboard process, the no-PIN mode first; REQ-AUD-008 resolved as "binary is packets"; `neottx qr`, `neottx keyboard` and `neottx view` typing back; videotex page selection on input channels. Phase 1, push and latency, is next. | — |
| Decision 45 | 20 October 2026 | **Push, typing and the phone keyboard built** as `interaction.md` designs them, all three phases, with the departures its section 17 lists for the owner to confirm: the cursor record on data channel 12 (channel 13 is Datavideo's); `input` in the descriptor's `push` object; every fetched file and pushed message read whole and completed at once, and a fetched copy of a pushed page dropped; a host's cursor record naming another of its pages shows that page; the menu button opens the phone keyboard; Project Nayuki's `qrcodegen` in the engine for QR codes. Measured: a key to the channel's pushed page, median 7.4 ms on this computer; render, encoding and the interface's decoding, median 7.4 ms. | Confirmed by decision 46 |
| Decision 46 | 21 October 2026 | **Every departure of decision 45 is confirmed** by the owner, as recommended: all fourteen of `interaction.md` section 17, `qrcodegen` approved among them. The fetcher now wakes when a channel's bytes arrive rather than looking at its sockets every 5 ms, as the recommendation on item 6 asked: the round trip's median fell from 6.3 ms to 0.28 ms. The engine is pinned at `afe5f44`, which no longer decodes DRCS or draws the Level 3.5 font styles (its decisions E.95 and E.96); the link underline of decision 42 is unchanged, and the teletext underline attribute is still not drawn. The menu button stays the phone keyboard's until the browser has a menu, which then holds it. qrkeyboard's public relay is to run protocol 3; it does since 21 October 2026. | The menu came with decision 57 and holds the phone keyboard (`history-and-bookmarks.md` section 14 item 9). Amended by decision 99, 24 October 2026: outside a page open for typing, a phone's keys only move between pages, and reach no field, question or panel |
| Decision 47 | 21 October 2026 | **Waiting and scale, as the review of 21 October recommended** (`~/devel/neoteletext-common/reviews/2026-10-21-waiting-and-efficiency.md`), on the owner's instruction to fix what it found: the audio callback renders only the channels that are sounding; the decoder keeps its own clock and draws a shown page only when its reception moves, or for flash when it flashes and its phase changes, and the trusted core's flash timer and its tick to every decoder go; a background tab with nothing live sleeps: its decoder is closed, while a tab whose channel holds a push connection never sleeps, and no fetcher is closed or paused for being behind (decision 22); and a decoder's page store grows as it fills. The review's pause of a background tab's fetching was dropped on the owner's word that teletext is mostly pushed. The audio callback's lock was measured and kept: the longest an operation holds it is 0.37 ms. | — |
| Decision 48 | 21 October 2026 | **Input protocol version 2** (`~/devel/neoteletext-common/INPUT.md`, the engine's decision E.100), adopted by the owner with every requirement numbered and built: the interface sends intents (keys pressed, repeating and released, text, the mouse and the wheel, paste and composition) and never the protocol; the trusted core keeps one input session a channel in the engine's `neoteletext-input`, sends HELLO and waits for WELCOME, applies the channel's and the page's modes, the rate and the input count, and alone encodes; the decoder passes on control records and a page's `IM` line beside cursor records; the mouse goes only when the channel or the page asks; requirements 18.2 to 18.10 give way to REQ-INT-130 to REQ-INT-139, and `interaction.md` keeps push and its history. | Amended by decision 49: the clipboard and normalization are built |
| Decision 49 | 21 October 2026 | **Two libraries for the input system, approved by the owner** ("if those libraries are rust then approved"): `unicode-normalization` in the trusted core, which sends typed text, and a KEY's text, in Normalization Form C (INPUT.md section 5.7 rule 3, REQ-INT-116); and `arboard` in the interface, whose clipboard text the browser's paste shortcut sends as one PASTE while keys go to a channel (REQ-INT-136). `arboard` brings `clipboard-win` and `error-code` on Windows, under BSL-1.0, allowed for those two crates (`licensing.md` section 2). | — |
| Decision 50 | 21 October 2026 | **The prototypes are removed** (the owner: "why is tauri still in there we dropped it"): the Tauri application and its React front end, the stand-in engine, the egui page renderer for WebAssembly, the two Tauri proofs, the webview sandbox spikes and the interface toolkit spikes, all of the design decision 33 left behind. Their findings stay in this repository's documents; the code is in git at the tag `prototypes-final`, and a path under `prototypes/` in any document is a path there. The full gate no longer builds them, and the toolchain no longer needs the WebAssembly target. | — |
| Decision 51 | 21 October 2026 | **Settings, the home page, the developer console and proxies** (`settings-home-console-proxy.md`, with its security analysis), designed, specified and adopted on the owner's instruction to design, specify, number the requirements of and build each: settings and bookmarks kept by the trusted core in its own file and changed in a panel of the trusted strip; a Home button and a start page, `neoteletext:home`, served by the trusted core as teletext; a docked developer console of the channel's network activity, packets, pages and notices; HTTP and SOCKS5 proxies for every connection, with no fallback. Requirements 19.1 to 19.12; requirement 3.6 amended: the interface receives a security setting only as the text the strip shows, and acts on none. The engine gives proxies and a report of each exchange (its decision E.102). | — |
| Decision 52 | 22 October 2026 | **Every helper is sandboxed on every platform, from "deny everything"** (`sandboxing.md` section 5): macOS `sandbox-exec` profiles; on Linux each helper confines itself before any thread with Landlock, an empty network namespace where allowed and a required seccomp deny-list (crates `landlock` and `seccompiler`, as the engine's `neottx`); on Windows a job object, mitigation policies, an AppContainer for the decoder and fetcher and a restricted token for the interface (`windows-sys`); the one layer every machine has is required, the rest applied when the machine allows and reported. `confine` is the third crate with unsafe code (requirement 10.12). The trusted core stays unconfined until packaging (requirement 3.8). | Built; item 1's "degrade and report" rescinded by decision 58: every layer is required; the interface's restricted token on Windows replaced by decision 90, 24 October 2026; the rest for the owner to confirm |
| Decision 53 | 22 October 2026 | **The browser waits on what it needs and draws what is on screen** (the efficiency work of 21 October 2026): the sound device runs only while a channel sounds; the window asks for frames only for what is drawn; every process flushes its pipe once per event, not per message; the trusted core computes its next deadline instead of two timer threads, and writes every pipe through a queue with a writer thread (`outbox.rs`), replacing a decoder that stops reading and ending with an interface that does; the fetcher opens connections on their own threads under the agent's connect limit; the decoder wakes for flash only when a rate a shown page uses changes phase; the engine tells a port of magazine data, timers and stalls only when they change (its decision E.103). | Built; owner to confirm |
| Decision 54 | 22 October 2026 | **Installation and updates on every desktop** (`installation-and-updates.md`), adopted by the owner: the launcher, side-by-side versions and a state file switched by an atomic rename, a trial with a health check and rollback after two failed starts; a confined updater that only stages; two signatures by keys held apart (The Update Framework in the updater, the platform's pinned signer in the launcher); `tough` kept as the client; on macOS one bundle swapped whole, proved first by a spike; per-user installs with no administrator; the timestamp expires after 7 days. **Updates are suggested, never imposed:** the browser checks and offers a new version in the strip, and downloads and installs nothing until the user accepts. | Adopted; built 23 October 2026 (decisions 73 to 82) |
| Decision 55 | 22 October 2026 | **Sound plays in a helper process of its own** (`neoteletext-sound`), not in the interface: the interface no longer links the system's sound library, so it starts on a Linux without ALSA, and the browser runs silent there (requirement 16.6); the sound helper is confined to the sound output, and the interface loses its access to it. | Built |
| Decision 56 | 22 October 2026 | **The trusted core is confined on every platform, behind a broker** (`sandboxing.md` section 7): a small unconfined broker, the program the user starts, starts the core confined and starts every helper on the core's request, passing the core the helper's pipes; it also reads a file channel's bytes and hands a web address to the system's browser for it. The core then has no network, starts no program and reads nothing but its settings folder. Chosen over the App Sandbox with XPC services, which cannot give each channel its own decoder process without private API and whose helpers would otherwise inherit the core's wider sandbox. Amends decision 52 item 5. | Built |
| Decision 57 | 22 October 2026 | **Browser-style history and bookmarks** (`history-and-bookmarks.md`, with `history-and-bookmarks-security.md`), proposed on the owner's request: history kept 90 days as one row per page per day, in a panel of the strip with search and removal by row, channel and time range; "Remember history"; private tabs; 25 recently closed tabs; Back and Forward lists; address-field suggestions with inline completion only from typed addresses; bookmarks with names and folders, the star bookmarking the page shown, a manager, an optional bookmarks bar, kept in `bookmarks.txt`; requirements 20.1 to 20.20 and amendments to 3.6, 12.5, 12.6, 19.1, 19.2, 19.5 and 19.7 on adoption. Eleven items for the owner in its section 13, among them a time-zone crate. | Adopted and built 22 October 2026 while the owner was away, as the owner instructed; section 13's items as recommended but item 10 (local time from the broker, no crate); departures in `history-and-bookmarks.md` section 14, for the owner to confirm |
| Decision 58 | 22 October 2026 | **Security is paramount: there can be no weakening or downgrading of security.** Decided by the owner on 22 October 2026, in the owner's words, rescinding decision 52's rule that optional sandbox layers be applied "when the machine allows" and reported when missing. 1. Every protection the design names for a process is required: a process that cannot have all of it is not run, and nothing runs in its place with less. 2. No fallback to a weaker mode, and no switch, setting or variable that selects one (`NEOTELETEXT_CONFINE_NO_CONTAINER` removed). 3. On Linux the network namespace, Landlock enforced whole at the version the rules are written for (Landlock 5, Linux 6.10) and the seccomp filter are each required; on Windows the AppContainer of every process but the interface; on macOS the `sandbox-exec` profile, as before. 4. A machine that cannot give them gets a clear error and no browser. Reason: decision 4; a sandbox that quietly becomes less is a weakness an attacker can arrange. | Built 22 October 2026. Amended by decision 90, 24 October 2026: on Windows the interface too runs in an AppContainer |
| Decision 59 | 22 October 2026 | **The session is always kept and restored at start**, on the owner's instruction: "the system must always be able to restore the current state of windows and tabs, and this is what should happen when the application starts too. Settings must include to enable/disable restore previous session." The window's place and size and every tab but private ones, with their back-and-forward lists, kept in `session.txt` after every change; restored at start unless "Restore previous session" is off (`history-and-bookmarks.md` section 15, requirements 20.21 to 20.24). | Built 22 October 2026 |
| Decision 60 | 22 October 2026 | **The Colour Map of Presentation Levels 2.5 and 3.5 is shown exactly as EN 300 706 defines it**, on the owner's instruction: "color should not have been removed ... re-implement 2.5 and 3.5 extended color - exactly as according to the specifications". The engine is built with its `level35-32colors` feature (which turns on `level25-32colors`; the engine's decision E.105 split its `colours32` in two) beside the `modern-classic` profile, which had left it out (the engine's decision E.71, recorded on the owner's instruction of 12 October 2026 and never reconciled with decision 6): X/28/0 and M/29/0 redefine CLUTs 2 and 3, and at Level 3.5 X/28/4 and M/29/4 redefine CLUTs 0 and 1, never CLUT 1 entry 0, which stays transparent (clause 12.4, Tables 4, 10, 11 and 13). The screen colour (Table 4's Default Screen Colour, clause 12.3's Full Screen Colour) is drawn as a border one character cell wide around each page, as a television shows it. Transparent shows black, the picture behind on a television with none. Not part of this decision: the Additional Flash Functions, flash to the corresponding colour in an adjacent CLUT among them (Table 29, the engine's `additional-flash-functions` feature, named `animation` until its decision E.105); the colour tables of downloadable characters (X/28/1, M/29/1), which went with the downloadable characters (the engine's decision E.95). | Built 22 October 2026; the engine's own profile is unchanged |
| Decision 61 | 22 October 2026 | **Page images cross to the interface in a form of the browser's own**, not as Portable Network Graphics, on the owner's acceptance of the recommendation "replace PNG with runs for a whole page plus changes against the last frame of the same page": each frame carries its palette, its screen colour and its dots, raw, in runs, or as changes against the frame of the same page the interface holds, whichever is smallest (`protocol::frame`). Measured on 800 real pages: encoding a page takes 156 microseconds against 1,583 for PNG, a flashing page's next phase is 2.1 kilobytes against 9.5, and the interface reads it in 8 microseconds; the `png` crate leaves the interface. Amends requirements 9.3 and 9.6 and decision 5's choice of image format. | Built 22 October 2026 |
| Decision 62 | 22 October 2026 | **Cookies as a current browser keeps them, without the legacy** (`cookies.md`, with `cookies-security.md`), proposed on the owner's request while the owner was away, with its decisions made as recommended (its section 13): the trusted core owns one jar per channel and fetchers ask per request (requirements 7.4 and 7.5); private tabs have their own; persistent cookies kept only encrypted under a key in the platform's secret store, session cookies never; a cookies panel with no name or value; Keep, Delete when closed, Block all, and blocked channels; clearing with history; `Clear-Site-Data` for the channel alone; Chrome's limits; `Domain`, the Public Suffix List, third-party cookies and RFC 2965 dropped. Requirements 21.1 to 21.11. The encryption libraries are the owner's to choose (section 13 item 7). | Built 22 October 2026 on the owner's instruction to fully implement it, with section 13's items as recommended and item 7's libraries chosen (`ring`, already in the tree, and `keyring` 4.2.0); `cookies.md` section 15 lists what building settled; for the owner to confirm |
| Decision 63 | 23 October 2026 | **The window's controls, as the owner asked on 23 October 2026**: "middle mouse button must close a tab. cookie management should be in the developer tools. icons need a reload button right left reload home", then "we need popups to explain/identify UI elements". 1. A press of the middle button on a tab closes it (requirement 12.1). 2. Cookie management is a tab of the developer console, Cookies, after Network, Packets, Pages and Notices; the menu's "Cookies and site data" is removed, and the settings panel's "Manage…" opens the console at that tab; it still never shows a cookie's name or value (requirements 12.7, 19.8, 21.6). 3. The strip's buttons are Back (its arrow pointing left), Forward (pointing right), Reload and Home; Reload, Command-R on macOS and Control-R elsewhere open the active tab's channel afresh, with a new decoder and fetcher, where the tab is in its history, which does not change (requirements 11.2, 11.6). 4. Every control drawn as an icon, a symbol or text that may be cut short has a tip naming it, saying what it does and giving its shortcut, shown when the pointer rests on it for half a second (requirement 12.12). Taken without asking, for the owner to confirm: "right left" read as the two arrows in the usual order; "Manage…" kept as a way into the console; a reload recorded in history as a visit, as Back is; the half-second wait. | Built 23 October 2026 in rapid iteration and seen on macOS; its tests and reviews owed (`TECH-DEBT.md`). The owner answered the four choices the same day: the order Back, Forward, Reload, Home confirmed ("left/right/reload/home"); "Manage…" and the half-second wait confirmed; a reload "must behave same as firefox/chrome in this specific regard": it moves the address's last visit and counts no visit, as both count none (requirement 20.1, amended) |
| Decision 64 | 23 October 2026 | **History follows Chrome's rules**, on the owner's instruction "model the history rules on chrome", each read from Chrome's source (`history-and-bookmarks.md` section 16): 1. a reload, the address shown entered again, or a link to it, adds no entry and counts no visit or typed visit, but moves the last visit; 2. the Back and Forward lists hold 12 entries, then up to 5 chapter stops where the host changes, then "Show all history"; a tab keeps 50 entries; 3. removing history removes the matching entries from the tabs' lists, never the one a tab is at, and from the closed tabs; 4. the closed tabs are kept between runs; 5. clearing offers the last 15 minutes; 6. a history row's menu offers "More from this site" (a `host:` search) and "Remove from history". Kept where Chrome has nothing to model: the history file's bounds. "Remember history" and "Forget this channel", which Chrome lacks, and the host for chapter stops went to the owner (decision 65). Requirements 11.6, 20.4, 20.6, 20.9, 20.10, 20.21 and 20.23 amended. | Built 23 October 2026; the session file is version 2, so a session saved by an earlier build is not restored once; Chrome's "Remove bookmark" built the same day |
| Decision 65 | 23 October 2026 | **What Chrome lacks goes, and chapter stops change at each host**, the owner's answers to decision 64's questions ("1 no 2 no 3 you decide"): 1. "Remember history" is removed: every page of an ordinary tab is recorded, and a private tab is the way to keep none (requirement 20.7 withdrawn); 2. "Forget this channel" is removed: "More from this site" finds a site's rows to select and remove (requirement 20.6 amended); 3. left to the implementer: chapter stops keep changing at each host, since shipping the Public Suffix List for one menu adds data to keep current for no gain in safety (`cookies.md` section 1.1, `history-and-bookmarks.md` section 16). The protocol's codes for the two (setting change 8, history request 5) are not reused. | Built 23 October 2026 |
| Decision 66 | 23 October 2026 | **No fonts from channels**, on the owner's word: "completely remove this, we will not be supporting alternate fonts". Every page is drawn in the engine's own fonts; a font a channel names is never fetched; there is no font helper and no font role. Requirements 3.5 and 5.7 withdrawn, 3.1 and 5.1 amended; the engine's own requirements on channel fonts are the engine's to decide (its own process). | Done 23 October 2026 |
| Decision 67 | 23 October 2026 | **Fastext, reveal, hold, mix and sub-pages** (`page-controls.md`), on the owner's instruction "implement fastext, reveal, subpages", taken while the owner was away: 1. the coloured keys and Index from packet X/27/0, followed as a page's other links are, a key acting only while row 24 is shown; 2. reveal; 3. hold and mix come too (the owner had left them open), since the engine has them and rotating and subtitle pages need them; 4. every sub-page kept and listed, the latest shown unless one is chosen, and an address's sub-page honoured; 5. the engine viewer's keys (`r` `g` `y` `c` `i`, `?`, `H`, `m`, Left and Right on a page with sub-pages). Requirements 22.1 to 22.6. | Built 23 October 2026; for the owner to confirm; the receiver-drawn link row of `LN` pages drawn since the re-pin at `00c90ea` (the engine's E.108) |
| Decision 68 | 23 October 2026 | **The remote control** (`remote-control.md`), on the owner's request for "a teletext remote control which is on a sidebar that can pop in and out of the right hand side or undocked to float", taken while the owner was away: 1. a panel inside the window, docked at the right edge with a slide, or floating over the page area, never over the strip; not a window of its own, which another program can cover or imitate; 2. it holds the number pad, page up and down, Fastext and Index, sub-pages, and Reveal, Hold and Mix, each an input that already exists; 3. Command-Shift-R, the menu and a handle at the right edge show and hide it; 4. its place is kept in `settings.txt`. Requirements 23.1 to 23.4. | Built 23 October 2026; for the owner to confirm |
| Decision 69 | 23 October 2026 | **Dragging tabs, reordering and overflow** (`tab-strip.md`), on the owner's request "if you roll over the tab bar and there is overflow then a window (connected) must appear underneath and the tabs wrap into that", taken while the owner was away: 1. a tab dragged along the strip moves there, the order the trusted core's; 2. the strip shows the tabs that fit, the active one always, and a count; rolling onto the strip opens a panel joined under it into which the rest wrap, every tab usable there; 3. Command-1 to 9 and Command-Option-arrows choose tabs, as Chrome's. Requirements 12.13 to 12.15, 12.1 amended. | Built 23 October 2026; for the owner to confirm; dragging between windows built with decision 72 |
| Decision 70 | 23 October 2026 | **A console like a web browser's**, on the owner's request for a "console.log style tab in developer tools", taken while the owner was away: the Notices tab becomes Log, whose lines carry a level and the process they came from, filtered by level and searched; the browser's own log, until then only on standard error, is kept there too, so each helper's confinement can be read from the window on every desktop. A channel's own log lines (the engine's `LG` lines, E.109) join it since the re-pin of 23 October 2026, marked as the channel's. Requirements 19.8, 3.7 and 4.3 amended. | Built 23 October 2026; for the owner to confirm |
| Decision 71 | 23 October 2026 | **Bookmarks from other browsers, a "Move…" chooser, and new tabs from Command-click and the middle button**, the to-do list's bookmark items, taken while the owner was away. The manager's Import reads another browser's Netscape bookmark file as well as this browser's own: a decoder process, started for it with `--bookmarks` and confined as every decoder, reads the file's few tags and reports folders and bookmarks; the trusted core, which reads no outside format (requirement 3.2), keeps only addresses its rules accept, into Imported. The file is not written in that format. A row of the manager has "Move…", which opens a list of the folders to move it into. Command-click (Control-click elsewhere) and the middle button open a history row, a bookmark of the manager, the bar or its lists, or a suggestion in a new tab, as Shift-click did. Requirement 20.25 added; 20.4, 20.17 and 3.2 amended. | Built 23 October 2026; for the owner to confirm |
| Decision 72 | 23 October 2026 | **Multiple windows**, on the owner's request "must be able to open multiple windows", taken while the owner was away (`multiple-windows.md`). One interface process draws every window (a process for each would be a Dock icon for each on macOS, and adds no boundary); the trusted core sets the windows not at hand aside with their tabs and what they show, and two context messages, `Target` and `From`, say which window the messages that follow are for or from. New window (Command-N), New private window and Move tab to new window in the menu; a tab dragged off the strip opens a window of its own, or joins the window whose strip it is let go on. Closing a window closes its tabs; the last window's ends the browser. Every window is kept with the session (version 3) and restored; a closed window reopens whole. At most 16 windows and 64 tabs among them. Requirement 12.8 met. | Built 23 October 2026; for the owner to confirm |

### Decisions of the installer and updater work

Taken on 23 October 2026 while the owner was away, as the owner had instructed ("do everything, do not stop, make the decisions you need"), in the branch that built decision 54, and numbered after decision 72 when it was merged. Each is for the owner to confirm.

| Number | Date | Decision | Now |
|---|---|---|---|
| Decision 73 | 23 October 2026 | **The installer and the updater, built on every desktop** as decision 54 adopted them (`installation-and-updates.md`): the launcher (`native/launcher`), the updater (`native/tuf/updater`), the Windows stub and installer (`native/stub`, `native/installer`), the release tool (`native/tuf/release`), and the core's side of updates (`core/src/trusted_core/updates.rs`, and the broker's in `core/src/broker.rs`). The updater and the release tool are a workspace of their own, `native/tuf`, so `tough`'s dependencies never reach the browser's crates through Cargo's feature unification (a second `rustls` provider makes `ClientConfig::builder()` panic). The departures from the design are its section 14. Taken while the owner was away. | Built 23 October 2026; owner to confirm |
| Decision 74 | 23 October 2026 | **Libraries:** `tough` 0.24.0 (MIT or Apache-2.0) and what it brings, `tokio` on one thread, `aws-lc-rs`, `serde_json`, `jiff` among them, in `native/tuf` alone; `miniz_oxide` 0.9.1 (MIT, Zlib or Apache-2.0) for the release files' DEFLATE, with gzip's framing written to RFC 1952; `rustix` 1.1.5 (Apache-2.0 WITH LLVM-exception, Apache-2.0 or MIT), already in the tree, named for the folder exchange and a process group's end; `rcgen` 0.14.10 (MIT or Apache-2.0) in the release tool, for test certificates only. Every licence checked by `cargo deny`. Taken while the owner was away. | Owner to confirm |
| Decision 75 | 23 October 2026 | **A fourth crate with unsafe code, `os-install`** (requirement 10.12 amended): CoreFoundation's alert on macOS, for the launcher, which has no window; on Windows `WinVerifyTrust` with the signer certificate's public key pinned, the per-user uninstall entry in the registry, and a message box. `confine`'s Windows module, already permitted, gains the grant to `ALL APPLICATION PACKAGES` and the removal of the containers' profiles. Taken while the owner was away. | Owner to confirm |
| Decision 76 | 23 October 2026 | **The release's identity is built in** (`native/release-identity`): the update host and its port, a test host's certificate root, The Update Framework's first root, the Linux release key, the macOS designated requirement and the Windows signer's key, read at build time from `production/` or from a test identity's folder, never at run time (decision 58). Until the key ceremony the production identity names only `update.neoteletext.com:443`, so a production build checks, stages and accepts nothing. Taken while the owner was away. | Owner to confirm |
| Decision 77 | 23 October 2026 | **Linux and AppArmor** (`sandboxing.md` section 8.3, the owner's question of decision 58): a stated requirement, not a system-wide package. Every Linux version carries `neoteletext.apparmor`, which lets the browser's programs where a per-user installation keeps them make a user namespace and changes nothing else; `neoteletext --install` prints the two administrator commands that install it where unprivileged user namespaces are restricted; the browser still refuses to run without its namespace. Taken while the owner was away. | Owner to confirm. Amended by decision 95, 24 October 2026: the profile names only the four programs that make namespaces, at the installation's real root |
| Decision 78 | 23 October 2026 | **Windows packaging:** the installer grants `ALL APPLICATION PACKAGES` read and execute on `versions\` (no administrator; the user owns the folder), and the launcher copies each version it accepts into `versions\` rather than renaming it, since what the updater wrote carries its container's full control and the low integrity label; the helpers' `AppContainer` profiles are made at their first start and removed by the uninstaller; a fetcher's loopback exemption needs an administrator and is not given, so a channel on the same computer needs the owner's one command (`CheckNetIsolation LoopbackExempt -a -p=<SID>`, the fetcher container's SID, since the tool refuses the container's name); the interface keeps its restricted token at low integrity (`installation-and-updates.md` section 14). Taken while the owner was away. | Owner to confirm. Amended by decisions 87 and 90, 24 October 2026: the launcher copies each version it accepts on every desktop, not Windows alone, and the interface runs in an AppContainer of its own with no capabilities, not the restricted token |
| Decision 79 | 23 October 2026 | **macOS's update files are apart from the settings,** in `~/Library/Application Support/NeoTeletext Updates/`, since the confined trusted core may write its settings folder. Taken while the owner was away. | Owner to confirm |
| Decision 80 | 23 October 2026 | **The launcher and the broker:** the launcher stays the broker's parent and alone writes the state, committing a trial when the broker writes `healthy`; a trial that ends badly is counted and the choice made again in the same start; one that says nothing in 45 seconds is ended with every process it started; an exclusive lock guards every change; an unreadable state is rebuilt from what is installed. A version is healthy when its window's first frame reached the screen, its settings were read and no decoder ended in error, within 30 seconds. On macOS and Windows the version accepted is read from a mark in the broker's signed bytes. Taken while the owner was away. | Owner to confirm. Amended by decision 93, 24 October 2026: the broker says `started` before the core starts, the launcher waits up to 5 minutes for it and then a health deadline from it, in place of 45 seconds from the broker's start |
| Decision 81 | 23 October 2026 | **Updates in the browser:** "Check for updates" (`updates on` in `settings.txt`, on by default); the settings panel shows the version, the trusted core's line about updates and "Install <version>"; the strip says the offer once as a notice; nothing is downloaded until Install is pressed; "Updates unavailable" after the timestamp's 7 days without a successful check; the updater goes through the user's proxy. Taken while the owner was away. | Owner to confirm |
| Decision 82 | 23 October 2026 | **Removal** (requirement 13.20): `--uninstall` on the terminal on Linux and macOS (and on macOS a bundle moved to the Bin, which leaves the settings, as macOS does), asking before the settings go; on Windows from Settings › Apps, asking with message boxes, a copy of the launcher in `%TEMP%` doing the removal and staying there. Taken while the owner was away. | Owner to confirm |
| Decision 83 | 23 October 2026 | **The certificate-error screen** (`certificate-errors.md`), on the owner's request: a channel refused because of its certificate shows, in its page area, the host as the strip shows it, what failed in the trusted core's words, that there is no way past, and Try again, Go back and Close tab; the strip reads "Certificate refused". The fetcher reports only the kind of failure, from its TLS library's error. Requirement 7.13 added. | Built 23 October 2026 |
| Decision 84 | 23 October 2026 | **The menu bar** (`menu-bar.md`), on the owner's request: on macOS only, the menus NeoTeletext, File, Edit, View, History, Bookmarks and Window, each item sending the interface core the key it names; Command-H is the system's Hide. Cut, copy, paste and select all in every browser text field, on every desktop. Requirements 12.5 and 12.11 amended. | Built 23 October 2026 |
| Decision 85 | 23 October 2026 | **Full screen, the page only** (`full-screen.md`), on the owner's request: the open page alone fills the display at its own proportions in its screen colour; no strip, which returns with the pointer at the top edge and the browser's own keys; entered by the system's full-screen key, the menu or a double click, left by Escape, the key again, or anything needing the window, including leaving the channel; a trusted badge in the margin while keys go to a channel. Requirement 11.4 amended, 12.16 added. | Designed 23 October 2026, not built |

### Decisions of the security review of 24 October 2026

Taken on 24 October 2026 while the owner was away, as the owner had instructed, to settle what the security review of that day left open. Three reviews of the browser at 494aa91 and the engine at e7d8927 were merged into one plan of 100 findings, F-01 to F-100; a fourth, tool-driven review had not reported, and its findings are not among them. The principle throughout is the least privilege that keeps every helper working and can be verified on macOS or the Linux host. Each decision names the finding it answers and the alternatives not taken. Each was built, or its document corrected, the same day, as its "Now" column says; the requirements and documents each changes were changed with its code, and where its check needs Windows, the owner makes it by hand in the virtual machine (`DEVELOPMENT.md` section 12.1). The review's choices for the engine are the engine's decisions E.112 to E.116, among them a form redirected across origins (F-16) and the local names refused through a proxy (F-17); they reach the browser with the next re-pin.

| Number | Date | Decision | Now |
|---|---|---|---|
| Decision 86 | 24 October 2026 | **Linux helpers keep off the session's sockets** (finding F-01, helpers that may connect to any local socket, the session bus among them, where systemd starts any program unconfined): (1) the fetcher, the keyboard and the updater lose local sockets and keep IPv4, IPv6 and routing netlink alone, seccomp refusing every other netlink protocol; name resolution falls through glibc's modules to DNS over UDP; (2) the interface and the sound helper get a mount namespace, in the user namespace they already make, with `/run` and `/tmp` covered by a private tmpfs and only their own display or sound sockets bound back; (3) `DBUS_SESSION_BUS_ADDRESS` leaves the interface's environment, since nothing in it uses D-Bus (the keyring, in the unconfined broker, is the tree's only user); (4) seccomp also refuses `pidfd_getfd`. A mount that fails stops the helper (decision 58). Not taken: `xdg-dbus-proxy` in front of the interface's bus, a dependency for a need that does not exist. Connecting in the broker (decision 89) would also close item 1, but it is the larger change. | Taken 24 October 2026 while the owner was away; built 24 October 2026, verified on the Linux host, the browser run there with every helper confined; owner to confirm |
| Decision 87 | 24 October 2026 | **The launcher accepts a copy, on every desktop** (finding F-02, the Linux and macOS launcher checking a staged version where the updater wrote it and then renaming those same files into place, so a confined updater could change them afterwards and gain an unconfined broker): the launcher copies a staged version into a folder only it writes, accepts the copy, and renames it into place, or on macOS exchanges it in, as decision 78 already has it do on Windows; the staged folder is never moved. A staged version whose root is a link is refused, and only the files of The Update Framework's client, and the floor of decision 91, are kept under `staging/metadata`. One routine closes all three ways the reviews found: a root that is a link, a hard link, and a write descriptor kept open. macOS's code signing covers a bundle's code pages but not its other files. Not taken: refusing a link at the root alone, or copying on Linux alone. | Taken 24 October 2026 while the owner was away; built 24 October 2026, verified on macOS and Linux; owner to confirm |
| Decision 88 | 24 October 2026 | **The broker reads only teletext for the trusted core** (findings F-03, the broker reading any path a compromised core names and following links it made in its own folder, and F-61, the broker opening any web address the core names): (1) files are opened without following links and checked by their descriptor, not their path; (2) the browser's own folders, every entry beginning with a dot under the home folder, and the platform's application-data folders (`~/Library` on macOS, `%APPDATA%` and `%LOCALAPPDATA%` on Windows) are refused, which refuses every link the confined core can make, since it writes only in the first; (3) only bytes the engine's `detect` recognises in a format the decoder is built with are returned, or a bookmarks file by its fixed first line; (4) every refusal is one fixed sentence, "not a channel this browser can open", so none tells whether a file exists or its size; (5) the broker opens a web address at most once in the interval the core already keeps between hand-offs, one constant for both. The fetcher's start arguments are not checked, since any data fits in a valid address. Not taken now: a file chooser owned by the broker, recorded as the design that closes this, since it needs native dialogs in the broker on three desktops and a change to how file channels are opened; and the broker's own confirmation of each web address. Running `detect`, which is bounded, fuzzed and free of unsafe code, in the unconfined broker is a deliberate small addition to it (requirement 2.1). What remains: a compromised core can still read files elsewhere that look like teletext, including any file of exactly 960 or 1000 bytes. On Windows, by hand: a `.t42` file in Documents and a folder of TTI files open; a file under `%APPDATA%` is refused; a junction in the settings folder pointing at `%USERPROFILE%\.ssh` is refused. | Taken 24 October 2026 while the owner was away; built 24 October 2026, verified on macOS and Linux; the checks by hand on Windows owed; owner to confirm |
| Decision 89 | 24 October 2026 | **The local-network rule in the sandbox** (finding F-05, nothing but the fetcher's own engine check keeping a compromised fetcher or keyboard off this computer and the local network, requirement 5.10): on macOS the fetcher's profile refuses loopback when the channel may not reach the local network; the sandbox's profile language cannot name private ranges, so the local network stays a residual there. On Linux nothing changes now, and the residual is documented: the rule holds while the fetcher is not compromised. Connecting in the broker and passing the socket to the fetcher is recorded as the planned design; it changes the broker's protocol, the core's pipes to the fetcher and the fetcher's transport, so it needs its own design and security analysis first. Decision 86 already removes the Linux escape through local sockets. | Taken 24 October 2026 while the owner was away; built 24 October 2026 on macOS, verified there; owner to confirm |
| Decision 90 | 24 October 2026 | **The Windows interface in an AppContainer** (finding F-04, the Windows interface able to reach the network and read the user's files under its restricted token at low integrity, against `sandboxing.md`): the interface runs in an AppContainer of its own with no capabilities, as every other helper does, with Win32k left on because it has windows, and keeps its job object, mitigation policies and handle list. It closes the network and the file reads together. If the container cannot be made, the interface does not run (decision 58). This replaces decision 78's restricted token for the interface. Not taken: a restricted token whose restricting identities deny the network device (`\Device\Afd`), and documentation alone. On Windows, by hand: the window opens and draws a teletext page and the strip's text; typing, and an input method's composition (Japanese, for example), work in the address field; copy and paste work in fields; full screen, a change of display scaling and resizing work; the pointer takes its shapes; and the probe can neither connect to the internet nor read a file in Documents. If the window cannot be made in an AppContainer, the work stops and the owner chooses; there is no fallback. | Taken 24 October 2026 while the owner was away; built 24 October 2026, its test passing in the virtual machine and failing on the code before; the window's checks by hand in its container owed; owner to confirm |
| Decision 91 | 24 October 2026 | **A floor on the update root's version** (finding F-06, the updater starting from its built-in root every time, so a host that withholds newer root files rolls it back to keys already rotated away): the updater keeps the highest root version it has reached in `staging/metadata/root-version`, refuses a root below it, and raises it after each successful load. The built-in root stays the trust anchor, so the chain is always verified from it. Not taken: starting from a stored `root.json`, which would make a file the updater writes the anchor. | Taken 24 October 2026 while the owner was away; built 24 October 2026, verified on macOS and Linux; owner to confirm |
| Decision 92 | 24 October 2026 | **The Windows manifest is signed** (finding F-07, no signature covering the Windows manifest, so programs signed for different releases could be combined under a new version number): the Windows release's manifest is signed with the existing Ed25519 release key, as Linux's is (`manifest.sig`), and checked before Authenticode by the code Linux uses. It covers every file, not programs alone, reuses a key and code that exist, and is tested on macOS. The Windows release folder gains `manifest.sig`. Not taken: a version mark in every program, which leaves the other files to a hash the attacker writes. On Windows, by hand: build and sign a test-identity release, publish it to the test host, update in the virtual machine, and see a version with a tampered `manifest.sig` refused. | Taken 24 October 2026 while the owner was away; built 24 October 2026, tested on macOS and Linux with the Windows platform; the check by hand in the virtual machine owed; owner to confirm |
| Decision 93 | 24 October 2026 | **A start deadline, then a health deadline** (finding F-09, the launcher's deadline running from the broker's start on a clock apart from the core's 30-second window, so a slow computer rolls a good version back, marks it bad and never offers it again): the broker tells the launcher `started` before it starts the core; the launcher waits for it up to a start deadline of 5 minutes, for a first run's checks and antivirus scans, then up to a health deadline counted from `started`, made of the core's health window and settling time, now defined once for both, and a named margin. A trial is counted only after `started`; a clean quit before health is not counted; a trial the launcher was running when it died counts at its next start; three starts that never reach `started` roll back. This amends decision 80's 45 seconds from the start. Not taken: the review's full redesign, which would offer again, after some days, a version rolled back for its deadline alone; a version still marked bad is superseded by the next release. On Windows, by hand: an update trial commits in the virtual machine, and a test version that crashes rolls back after two starts. | Taken 24 October 2026 while the owner was away; the launcher side built 24 October 2026, verified on macOS and Linux; the broker's `started` and the core's shared constants built 24 October 2026 (`native/core`), an update trial with them not yet run end to end; owner to confirm |
| Decision 94 | 24 October 2026 | **Linux helpers signal only themselves** (finding F-60, a confined helper able to signal any of the user's processes, to stop the broker or kill an editor): seccomp lets `kill`, `tgkill` and the queued-signal calls name only the helper's own process, and refuses `tkill`, `pidfd_open` and `pidfd_send_signal`. It keeps today's kernel floor, Landlock 5 on Linux 6.10, and is verified on the Linux host. Not taken: requiring Landlock 6's signal scoping (Linux 6.12), which would cut users and is the owner's to choose; documentation alone. | Taken 24 October 2026 while the owner was away; built 24 October 2026, verified on the Linux host; owner to confirm |
| Decision 95 | 24 October 2026 | **A narrow AppArmor profile** (finding F-23, the profile an administrator is asked to install giving user namespaces to any program at a pattern under `~/.local/share`, which every user writes, and missing an installation under `XDG_DATA_HOME`): the profile names only the four programs that make namespaces, the broker and core, the decoder, the interface and the sound helper, at the real installation root, which the launcher prints; the text tells the administrator in one sentence what it allows. What remains is documented: any program the user writes under those four names gets user namespaces. An installation owned by root in `/opt`, with its profile there, is recorded as the design that closes it, not built, since it breaks decision 54's updates for each user. Not taken: doing without user namespaces, which decision 86's mount namespace needs. This narrows decision 77's profile. | Taken 24 October 2026 while the owner was away; built 24 October 2026, verified on the Linux host with the restriction on; owner to confirm |
| Decision 96 | 24 October 2026 | **The phone keyboard's relay serves the phone's code** (finding F-18; documentation only): `interaction-security.md` is corrected to say that a passive relay or network observer cannot read the keys, and that the relay's operator, or whoever compromises its server, can read the PIN and the keys, since the relay serves the phone's page; that document's section F bounds it, a hostile phone being a hostile local keyboard with no modifiers. Not taken now: serving the phone's page apart from the relay, since qrkeyboard.com is not this repository's server. | Taken 24 October 2026 while the owner was away; `interaction-security.md` corrected 24 October 2026; owner to confirm |
| Decision 97 | 24 October 2026 | **macOS sandbox profiles take paths as parameters** (finding F-51, paths written into the profiles' string literals unescaped, so a bundle in a folder with a crafted name would run under a profile that allows everything): every path is passed with `sandbox-exec -D` and named in the profile as a parameter. Not taken: refusing paths that hold a quotation mark, a backslash or a control character, which would refuse some users' folders. | Taken 24 October 2026 while the owner was away; built 24 October 2026, verified with `sandbox-exec`; owner to confirm |
| Decision 98 | 24 October 2026 | **The cookie key is not wiped** (finding F-32; documentation only): `cookies.md` and `cookies-security.md` are corrected to say that the key is held in the trusted core's memory for its life and not wiped at exit, as built. Not taken: a key wiped with volatile writes, which needs unsafe code in the core (requirement 10.12), or the `zeroize` crate, a new dependency; an attacker who can read the core's memory has its cookies already. | Taken 24 October 2026 while the owner was away; `cookies.md` and `cookies-security.md` corrected 24 October 2026; owner to confirm |
| Decision 99 | 24 October 2026 | **Phone keys outside a typing page only move between pages** (finding F-20, phone keys reaching the browser's own controls, so a compromised keyboard process could type an address, change the proxy, or confirm a hand-off or a site change): while no page is open for typing, the trusted core passes on only the page-navigation keys, the digits, the four arrows and the four Fastext colour keys where the layout has them, and the interface gives phone keys to the page view alone, never to a field, a prompt or a panel. The core's filter is the deciding control and does not depend on the interface's code (decision 20). Not taken: a rule refusing a confirmation within a delay after a phone key, which a keyboard that waits defeats. This amends what decision 46 confirmed of `interaction.md`. | Taken 24 October 2026 while the owner was away; built 24 October 2026 in the trusted core and the interface, not yet tried with a real phone; owner to confirm |
| Decision 100 | 24 October 2026 | **Three smaller choices of the review:** (1) on Windows, if a confined helper started from a console can open `CONIN$` and write the console's input (finding F-22, the counterpart of `TIOCSTI`, which seccomp refuses on Linux), helpers are started detached from the console and the broker relays their standard error (measured in the virtual machine and built on 24 October 2026, `sandboxing.md` section 9.10); (2) the web opener refuses an address holding a shell-significant character, `'`, `;`, `$`, `(`, `)`, `!` or `` ` ``, rather than encoding it (finding F-41); on Windows, by hand: a link from a page still opens; (3) the phone keyboard's relay override, `NEOTELETEXT_QRKEYBOARD_RELAY`, and its local-network access are for debug builds only (finding F-64); on Windows, by hand: the phone keyboard pairs with qrkeyboard.com in a release build. | Taken 24 October 2026 while the owner was away; item 1 measured in the virtual machine on 24 October 2026: started from a console, a helper can write its input, and the detached start is not built; items 2 and 3 built 24 October 2026, their checks by hand on Windows owed; owner to confirm |

---

## 3. Concepts and terms

| Term | Meaning in this project |
|---|---|
| **Teletext page** | One screen of teletext: 40 columns by 25 rows of character cells, row 0 being the header. Identified by magazine (1 to 8), page number (00 to FF) and subcode. |
| **Subpage** | One of several versions of a page carrying the same page number, distinguished by subcode. |
| **Channel** | One teletext service. On the web it is identified by its **origin** and its **folder path** (`neoteletext` decision E.70, the engine's `docs/spec/requirements/web.md`): a folder holding `channel.json`, `contents.txt`, optionally a bundle `index.t42`, and page files such as `101.tti`. A file channel is a recording or a folder of page files. The channel is the unit of trust and isolation: it plays the part a site plays in a traditional browser. |
| **Origin** | The combination of scheme, host and port of an address. |
| **Tab** | One view onto one channel at a time, with its own history. |
| **Bank** | A hundred pages, such as 100 to 199: teletext's convention gives each section a hundred, with the section's index on its first page. The window shows the eight banks 100 to 899. |
| **Trusted core** | The process that holds everything the user trusts: which channel each tab shows, history, bookmarks, settings, and the decisions about addresses. It contains no web engine and draws nothing. |
| **Interface process** | The process that draws the window: tabs, the trusted strip, the banks, the page list, the page area and the overview. It holds no secrets, opens no network connection and no file, and reads only what the trusted core sends. |
| **Phone keyboard** | The instance's token and the pairing keys of one phone session | The network and the name resolver, for qrkeyboard's relay | Keys typed as the viewer would type them; no pages, cookies or files (`interaction-security.md` section F) |
| **Updater** | Nothing secret; the release it is staging | The update host's one port and its staging folder (`installation-and-updates.md` section 8.1) | Files in its staging folder, which the launcher accepts only when both keys signed them |
| **Launcher** | The installed versions and the state file | Everything the user may; not confined | The user's rights; it parses only its own state, signed manifests, the version mark and the broker's health line (`installation-and-updates-security.md` section 4a) |
| **Interface core** | The library, without the standard library, that holds the window's state, layout, animation, hit-testing and input handling, and draws only through its drawing interface. |
| **Drawing back end** | An implementation of the drawing interface: `vello_cpu` and Parley on the processor for desktops and phones, `embedded-graphics` for microcontrollers. |
| **Trusted strip** | The toolbar along the top of the window: Back, Forward, the encryption state, the address with its host emphasised, the bookmark star and the menu. It shows only what the trusted core sends, and its input reaches the trusted core as strip input, message kinds that page intents never use. It is drawn by the interface process (section 6.1). |
| **Fetcher** | The process that holds one web channel's network connections and reads its network formats through the engine's web module. |
| **Decoder** | The process that recognises, decodes and renders one channel's teletext. It has no network and no secrets. |
| **Frame** | One rendered page: palette indices plus its palette, produced by the decoder. |
| **Resource role** | The purpose for which a resource was requested: teletext data, channel descriptor, contents file, or discovery (a font was one until decision 66). A response is read only by the reader for the role it was requested for. |
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

The engine is `neoteletext` (`~/devel/neoteletext-engine`), a Rust implementation of World System Teletext, EN 300 706, Presentation Levels 1.0, 1.5, 2.5 and 3.5, less what the owner removed on 20 October 2026: dynamically redefinable characters (DRCS) are not decoded, and the Level 3.5 font styles (bold, italic and proportional spacing) are not drawn (its decisions E.95 and E.96). Its core is `no_std` (it does not depend on the Rust standard library) and performs no input or output of its own: it is fed bytes and returns results.

**It is a separate project** (decision 36). NeoTeletext depends on its crates, in every workspace, at one pinned commit, `fb2dd8a` (23 October 2026: E.106 to E.111, every feature set built again, the qrkeyboard waker, a proxy's answer read in chunks, the receiver's row 24 and links in the formats crate, channel log lines, Device Bound Session Credentials behind a feature, and fonts named by channels withdrawn), through a `git` source in the engine's local repository, because that commit is not yet published; Cargo reads the commit, never the engine's working tree. When the commit is published, the source becomes the engine's own repository address at the same commit. Changes the engine needs are made in the engine's repository by its own process, never patched here.

| Crate | What it does | Used by NeoTeletext |
|---|---|---|
| `neoteletext-core` | Packet decoding, page store, Presentation Levels 1.0 to 3.5, rendering a page to palette indices, flash timing | In the decoder; its flash timing constants in the trusted core; its page addresses in the fetcher. Built with the engine's recommended `modern-classic` profile and its `level35-32colors` feature (decision 60) (its decision E.71) |
| `neoteletext-formats` | Readers for T42 recordings, Teletext Text Interchange page files, transport streams and sliced records; content detection | In the decoder, with the `tti`, `transport` and `sliced` features |
| `neoteletext-image` | Palette indices to colour formats; the Portable Network Graphics writer | The palette in the decoder; the writer no longer (decision 61) |
| `neoteletext-web` | The strict address parser, origins, the HyperText Transfer Protocol version 1.1, Server-Sent Events, WebSocket, the cookie store, channel descriptors, contents files, push subscriptions, and the user agent that opens and fetches a channel, with no network input or output of its own | In the fetcher, with the `push` and `cookies` features; in the trusted core, its cookie jar, proxy-address parser and address type; in the phone keyboard; and in the updater, pinned in `native/tuf/Cargo.toml` |
| `neoteletext-audio` | The audio profile's receiver (its area AUD) | In the decoder (decision 37) |
| `neoteletext-audio-mix` | The audio mixer | In the sound helper (decision 55) |
| `neoteletext-input` | Key, cursor and control records and input sessions (its area INT) | In the trusted core, the decoder, the interface core and the interface (decisions 44 and 48) |
| `neoteletext-qrkeyboard` | qrkeyboard's client: pairing, encryption and QR codes | In the phone keyboard (decision 44) |
| `neoteletext-slicer` | Recovering teletext from sampled video | Not yet |
| `neoteletext-browser`, `neoteletext-gui`, the application crate | The engine's own browser shell | **Not used** (decision 36) |

### 5.1 Changes required in neoteletext

NeoTeletext defines no limit and no fetching behaviour of its own (decision 26). Everything below is work in neoteletext, recorded with its questions in `neoteletext/docs/neoteletext-browser-open-questions.md`.

| Change | NeoTeletext requirements |
|---|---|
| A grace period during which a closed tab's channel keeps refreshing | Requirement 5.17 |
| A threshold of malformed packets after which a live feed is closed | Requirement 6.5 |
| A default refresh schedule for a channel that declares none, if none exists | Requirement 5.16 |
| The cookie store separable from the user agent, so the trusted core can own it | Requirement 7.5; done by the engine's E.104 (`64c1a97`) |
| A page's text, cleaned of control, bidirectional and zero-width characters and bounded, for display and accessibility | Requirements 11.5 and 12.9 |
| Transport Layer Security for the engine's network port, or a statement of which library NeoTeletext should supply it with | Requirement 5.8 |

**Limits NeoTeletext takes from neoteletext.** Among them: `contents::MAX_PAGES` (800 pages), `contents::MAX_BYTES` (64 kilobytes), `DEFAULT_PAGE_LIMIT` (256 kilobytes), `DEFAULT_PUSH_LIMIT`, `DEFAULT_DESCRIPTOR_LIMIT`, `DEFAULT_HEADER_LIMIT`, `DEFAULT_FETCH_INTERVAL_US` (one request every 0.2 seconds), `DEFAULT_CONNECT_TIMEOUT_US` (10 seconds), `DEFAULT_RESPONSE_TIMEOUT_US` (30 seconds), and `MIN_REFRESH_SECONDS` and `MAX_REFRESH_SECONDS`. The full table is in `neoteletext/docs/neoteletext-browser-open-questions.md` section 3.

---

## 6. Processes and isolation

### 6.1 The process model

Decision 18's shape, with the interface process in place of the web-engine window host (decision 33), sound in a helper of its own (decision 55), and a broker that starts every process and does for the confined trusted core what it may not do itself (decision 56; `sandboxing.md` section 7).

```
┌── broker (started by the launcher or the user; with the launcher, not confined) ───────┐
│ starts the trusted core confined, then each helper the core asks for, confined by its  │
│ program's role, passing the core its pipes; reads file channels; opens the web browser │
└───────────────────────────────────────────┬────────────────────────────────────────────┘
                                            │ bounded requests, pipes passed as descriptors or handles
┌───────────────────────────────────────────▼─── trusted core (Rust, no web engine, draws nothing) ┐
│ owns: tabs, history, bookmarks, settings, address rules, which pages are visible, frame checking, │
│ image encoding, the state of the trusted strip; passes file channels on unread                    │
└──────┬─────────────────────────────┬──────────────────────────────────┬───────────────────────────┘
       │ framed pipe                  │ framed pipe                       │ framed pipe
       │ contents, page bytes,        │ bytes, shown pages, views in;     │ tabs, strip, channel, frames out;
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
       also: the sound helper (checked voice operations in, which channels sound out), the updater
       (a check or a download in, what it found or staged out) and, while open, the phone keyboard;
       the launcher, which the user starts, chooses the version and starts the broker (section 12)
```

| Process | Holds | May access | If compromised, the attacker gains |
|---|---|---|---|
| **Broker** | Nothing of the user's | Everything the user may; it does only what section 7 of `sandboxing.md` lists | The user's rights; this is why it parses only the trusted core's few bounded requests and reads nothing else from outside |
| **Trusted core** | Everything the user trusts in the browser | Its settings folder; through the broker, starting the browser's own helpers, reading file channels and opening the web browser | The browser's state and the strip, but not the user's files or the network, which its sandbox denies (decision 56); this is why it reads no network or teletext format and contains no web engine |
| **Fetcher** | One channel's network connections; no cookies (decision 62: it asks the trusted core for each request's) | The network, which is its job | The ability to send and receive on the network, and the cookies the trusted core gives its channel's requests; no other channel's cookies, no history, no screen |
| **Decoder** | One channel's pages | Nothing it needs; see section 6.5 for what the sandbox still allows | Pictures of its choosing in that channel's page images, which its server could draw anyway |
| **Sound helper** | The sound now playing | The audio output alone | Sound of its choosing, and a false report of which tabs sound |
| **Interface process** | What is on screen | The window and input; see section 6.5 | Everything on screen, including a false trusted strip, and the ability to send intents and **strip input**: it can navigate a tab to any address the trusted core's rules accept, go back and forward, and set bookmarks. No secrets, because none are sent to it |

**The trusted strip and the interface process.** The strip is drawn by the interface process, in its own window, outside the page area (design section 5.2, option 1). That keeps the strip out of reach of every page image and every other process's window. It does not keep it out of reach of a fault in the interface process itself: a compromised interface process can draw a false strip and send strip input. Reaching it needs a fault in NeoTeletext's own interface code or its drawing libraries, fed only by our own images and cleaned text (`security-analysis.md` section 5.6). Requirement 3.9's hostile interface harness tests every other property with an attacker's interface in its place.

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
- **Each process flushes its pipe once per event, not per message** (decision 53), so what one event produced goes in one write. **The trusted core writes every pipe through a queue with a writer thread of its own** (`core/src/outbox.rs`): its one thread, which owns every tab, never waits on a process that is slow to read. A decoder that stops reading fills its queue, 16 MB, and is killed and replaced with what it was given (decision 47); an interface that stops reading ends the browser.

**From the trusted core to a decoder** (`protocol::decoder`):

| Message | Meaning |
|---|---|
| Data | Bytes of the channel, at most 64 kilobytes, passed on unread |
| End | The channel's data has ended; finish what is buffered |
| Showing | The pages the interface shows now, to render |
| Close | Stop |
| Opened | The page the user opened, or none: its sounds play while it is open |
| Whole | A whole page file, bundle or pushed message, TTI or 42-byte packets, whose pages are complete as it ends; a fetched copy of a page the channel pushed is dropped (`interaction.md` section 17 item 4) |
| View | How the open page is drawn: its reveal, hold, mix and sub-page (decision 67) |

**From a decoder to the trusted core:**

| Message | Meaning |
|---|---|
| Confined | Its first message, before it reads anything: it is confined and running; one that ends before saying it fails an update's trial (finding F-12) |
| Arrived | A page was received completely for the first time: its number and a title from its first text row |
| Updated | A page already held arrived again with different rows |
| Frame | A rendered page: 480 by 500 palette indices and a palette of at most 32 colours, every index checked against the palette on arrival |
| Audio | One voice operation of the channel's audio receiver: define a sound, its 16-bit samples (at most 32 768 a message), forget it, announce a live sound and give its next samples, or start, stop or ramp one of four channels; every field bounded (requirement 16.4) |
| Links | The links found in a page, before its image when they change: at most 32 runs (page numbers in its text and its `LT` lines, each a place and either a page number or a target as written, at most 256 bytes, never cut) and its `LK` lines; and whether an image follows (`hyperlinks.md` section 8.1) |
| StoreFull | The page store is full and the oldest pages are being forgotten |
| SubPages | The sub-pages of a page on screen and the one drawn, whenever they change (decision 67) |
| Cursor | A cursor record the channel sent for a page, untrusted, for the trusted core to check (INPUT.md section 7.1) |
| InputControl | An input control record (WELCOME, `PUSH_MODE`, `POP_MODE`, `PAGE_MODES`) or a page's `IM` line, untrusted (REQ-INT-111, REQ-INT-114) |
| ChannelLog | A log line the channel wrote (an `LG` line), untrusted, for the console's Log alone (decision 70) |
| Bookmark, BookmarksRead | A folder, its end or a bookmark found in another browser's bookmark file, and how many were left out, from a decoder started with `--bookmarks` (decision 71) |

**From a fetcher to the trusted core** (`protocol::fetcher`):

| Message | Meaning |
|---|---|
| Confined | Its first message, before it opens its channel: as a decoder's |
| Contents | The channel's contents file as the engine read it: section headings and page entries with titles, at most 1,000 entries |
| Page | A page's bytes, fetched or pushed, passed to the decoder unread |
| Notice | A short report, such as an unreachable address, cleaned and shown in the trusted core's log |
| Links | The descriptor's aliases (at most 16) and named services (at most 16), as written, for the trusted core to parse |
| Pushing | The push connection opened or was lost (decision 47) |
| Exchange | One exchange the agent made, for the developer console (requirement 19.9) |
| ProxyFailed | The proxy refused, failed or did not answer; it carries nothing (requirement 19.10) |
| Input | The push WebSocket of a channel that takes input opened or closed, and whether it is `wss` |
| CookieQuestion, ReachedQuestion | Asks for a request's `Cookie` header, or whether a host was reached securely with a live cookie (decision 62) |
| CookieSet | A `Set-Cookie` value a response carried, as received |
| ClearSiteData | A host's encrypted response said `Clear-Site-Data: "cookies"` |

**From the trusted core to a fetcher** (`protocol::fetcher`):

| Message | Meaning |
|---|---|
| Send | One input message for the channel's push WebSocket, at most 4,096 bytes (requirement 4.7) |
| CookieAnswer, ReachedAnswer | The answers to the fetcher's two questions |

**Between the trusted core and the sound helper** (`protocol::sound`), **the phone keyboard** (`protocol::keyboard`) **and the updater** (`update::wire`):

| Direction | Message | Meaning |
|---|---|---|
| Core to sound helper | Audio, Settings, Close | A checked voice operation of a channel; page sounds and their volume; stop |
| Sound helper to core | Sounding | The channels sounding now |
| Keyboard to core | Join, State, Key, Text | The QR and PIN; the session's state; a key down or up; typed text |
| Core to keyboard | Finish | End the session |
| Core to updater | Check, Download | Look for a newer version; download and stage the one the user accepted |
| Updater to core | Latest, Staged, Failed | The newest version and when; a version staged; why a check or download failed |

**Between the trusted core and the interface process** (`protocol`):

| Direction | Message | Meaning |
|---|---|---|
| Core to interface | Tabs | Each tab's cleaned title, colour, channel and whether it is muted, and which is active |
| Core to interface | Strip | What the trusted strip shows: the address as the address rules formatted it, the channel's identity, whether it is encrypted or a local file, whether Back and Forward can go anywhere, whether it is bookmarked |
| Core to interface | Channel | The active channel: each page's status and title, and the banks' names |
| Core to interface | Page | A page arrived, with its cleaned title |
| Core to interface | Updated | A page changed |
| Core to interface | Frame | One page image: channel and page, frame version, and the image in `protocol::frame`'s form: palette, screen colour, and dots raw, in runs or as changes against the frame of the page the interface holds (decision 61) |
| Core to interface | Sounding | The channels sounding now, muted or not, for the tabs' speakers; the sound helper says it and the trusted core passes it on (decision 55) |
| Core to interface | Link: page links | Where each link of one image of a page lies and what kind of place it leads to, never its target; and the page numbers it remaps |
| Core to interface | Link: pointing, prompt | What the strip shows of the link under the pointer or the focus; the site-change screen or the web-browser prompt, with its number |
| Core to interface | Link: show, aliases | Open a page a link led to; the page numbers the channel's descriptor links elsewhere |
| Core to interface | Typing, phone, phone key | Whether keys go to the open page's channel, and its cursor; the phone keyboard's panel; a key of the paired phone (decisions 45 and 48) |
| Core to interface | Settings, update | The settings panel's view; the version and an update's offer (decision 81) |
| Core to interface | Console, cookies | The developer console's new entries; the cookies tab's view, never a name or value |
| Core to interface | History, suggestions, tab list, bookmarks, star, bar, window | The panels' views, the address field's suggestions, a Back or Forward list, the bookmarks bar, the window's kept place (decisions 57 and 59) |
| Core to interface | Page controls, remote | The open page's controls; the remote control's place (decisions 67 and 68) |
| Core to interface | Target, open window, close window | Which window the messages that follow are for; open or close a window (decision 72) |
| Core to interface | Close | The application is ending |
| Interface to core | Visible | The pages the interface's layout puts on screen |
| Interface to core | Cell | A click on a page's cell: page, the version of its image on screen, column and row |
| Interface to core | Point, typed, focused | The link cell under the pointer or the focus, or none; a page number the page or channel links elsewhere; whether the window has the keyboard focus |
| Interface to core | Select tab, close tab, new tab | Tab changes |
| Interface to core | Opened | The page the user opened in the active tab, passed to that channel's decoder |
| Interface to core | Mute tab, mute all | Mute or unmute one tab; mute or unmute every tab |
| Interface to core | Strip input: navigate, back, forward, bookmark, confirm, cancel | What the user did in the trusted strip, including the answer to its question, which names the question. These are message kinds of their own; a page intent reaches another channel only through the site-change screen |
| Interface to core | Move tab | Strip input: move a tab along the strip (decision 69) |
| Interface to core | Editing, input | Editing the address began or stopped; a key, text, the mouse, a paste or a composition for the open page's channel (decision 48) |
| Interface to core | Phone panel, phone finish | Strip input: open or close the phone keyboard's panel; end its session |
| Interface to core | Home, reload, change setting, install update, console | Strip input: Home, Reload, a change the settings panel asks for, install the offered version; the console opened or closed |
| Interface to core | Drawn | The window drew its first frame, part of a version's health (decision 80) |
| Interface to core | History, bookmarks, star, bar, suggest, suggestion, tab list, go to, new private tab, window | The panels' requests, the address field's text and a chosen suggestion, a Back or Forward list and a step in it, a private tab, the window's place and size |
| Interface to core | Cookies, page control | Strip input from the cookies tab; a key of the open page's controls |
| Interface to core | From, new window, tab to window, close window | Which window the intents that follow are from; open a window, move a tab to another or a new one, close the window (decision 72) |

### 6.5 Isolation on each platform

What is built today is in **bold**; the rest is required.

| Platform | Trusted core | Fetcher | Decoder | Interface process | Sound helper |
|---|---|---|---|---|---|
| macOS | **`sandbox-exec` profile from "deny everything", started by the broker: its settings folder read and written; no network, services or programs** | **`sandbox-exec` profile from "deny everything": outbound TCP and the name resolver, no file reads otherwise, no programs** | **`sandbox-exec` profile from "deny everything": its pipes alone** | **`sandbox-exec` profile from "deny everything": the system's folders and metadata, the window, font, text-input, pasteboard, notification, preference and Dock services; no network, no programs** | **`sandbox-exec` profile from "deny everything": the audio services alone** |
| Windows | **Less privileged AppContainer with no capabilities, its settings folder granted, job object, mitigation policies, Win32k filter** | **AppContainer with the internet-client capability (and the private-network one when local addresses are allowed), job object, mitigation policies, Win32k filter** | **Less privileged AppContainer with no capabilities, job object, mitigation policies, Win32k filter** | **AppContainer with no capabilities, job object, mitigation policies** (decision 90; to be verified in the virtual machine) | **AppContainer with no capabilities, job object, mitigation policies** |
| Linux | **Confines itself before any thread: empty network namespace, Landlock (its settings folder), seccomp (no sockets)** | **Landlock (the resolver's directories), seccomp (sockets of the internet families only)** | **Empty network namespace, Landlock (nothing readable), seccomp (no sockets)** | **Empty network namespace, Landlock (fonts, preferences, the system's directories), seccomp (local-domain sockets only)** | **Empty network namespace, Landlock (the sound devices), seccomp (local-domain sockets only)** |
| Android | Main application process | Separate service process | Isolated service process with a seccomp filter | The application's activity | Inside the activity |
| iOS | One application process | Inside it | Inside it; the WebAssembly interpreter `wasmi` as its wall (open decision 2) | Inside it | Inside it |
| Microcontrollers | One program; a single-purpose device, with its own analysis (section 13) | | | |

**The macOS profiles start from "deny everything"** (21 October 2026), import Apple's baseline for any process (`system.sb`) and allow only what each process is for; `security-analysis.md` section 13 says how they were made. Every child, on every platform, is started with an environment of only what it needs. The broker, and in an installed browser the launcher that starts it (section 12), are the processes not confined, on every platform (`security-analysis.md` section 5.8, `installation-and-updates-security.md` section 4a). The phone keyboard is confined as a fetcher is, with the network and the name resolver alone; the updater with the update host's one port and its staging folder (`installation-and-updates.md` section 8.1).

**On Linux there is no confinement of the interface beyond the platform's,** because there is no web engine to contain: the interface process is NeoTeletext's own code, and it needs the display as any application does. Under X11 any application can send input to other applications' windows (`prototypes/docs/proofs.md` section 6.8); that is the exposure of every X11 application, not a containment problem.

---

## 7. Network access

### 7.1 What may be fetched: resource roles

The application loads only resources that are part of a teletext service (decisions 12 and 14). Each is fetched in exactly one role and read only by the reader for that role.

| Role | Read by | Read in | What the trusted core receives |
|---|---|---|---|
| **Teletext data**: page files, bundles, the other formats of `neoteletext-formats`, and pushed pages | Content detection and the format readers | Decoder | Arrivals, updates and frames |
| **Channel descriptor** (`channel.json`) | The engine's descriptor reader | Fetcher | Its aliases and named services, as written, for the trusted core to parse (`Links`, `hyperlinks.md` section 6.4); the fetcher uses the rest to find the contents file and the push feed |
| **Contents file** (`contents.txt`) | The engine's contents reader | Fetcher | A bounded, typed list of sections and pages |
| **Discovery** (HyperText Markup Language) | The engine's discovery reader | Fetcher, not built | — |

Discovery's security analysis was the engine's `docs/web-discovery-security.md`, in its git history since 18 October 2026; its requirements are in the engine's `docs/spec/requirements/web.md`. Its results are never loaded directly: each is a page-initiated navigation through the site-change rules.

### 7.2 Transports

- **The HyperText Transfer Protocol version 1.1**, plain or with Transport Layer Security (`rustls` with Mozilla's root certificates, decision 41; `fetcher/src/transport.rs`). The trusted strip shows "Not secure" for a channel reached without Transport Layer Security (decision 10).
- **Server-Sent Events and WebSocket** carry the channel's push feed: the fetcher subscribes to all of a channel's pages and passes each pushed page to its decoder; on a channel whose descriptor says it takes input, the WebSocket also carries the viewer's input (`interaction.md`, decision 45; `core/tests/latency.rs`).
- **Not supported:** the push mechanism of the HyperText Transfer Protocol version 2, which traditional browsers have removed.

### 7.3 Rules that apply to every request

These are the engine's user agent's rules, which the fetcher uses as they are:

- **Addresses on this computer and the local network are refused** unless the user names a specific address, as the engine's requirement REQ-SEC-006 requires. The user names it in the settings panel (requirement 19.12, decision 51) or with `--allow-local` on the command line.
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

**Where it lives** (decision 62, `cookies.md`). The trusted core holds every jar, one for ordinary tabs and one for private ones; a fetcher holds none and asks for each request's header (requirements 7.4, 7.5 and 21.1). Persistent cookies are kept between runs sealed under a key in the platform's secret store; session cookies never are.

**No `Domain`, so no Public Suffix List.** A cookie never reaches beyond the host that set it, so the list browsers use to stop `Domain=co.uk` has nothing to do; where Chrome groups by registrable domain, this browser groups by host or channel. Why, and what it costs, is `cookies.md` section 1.1.

### 9.4 Storage

- The decoder's page store belongs to one channel.
- The trusted core's record of frames sent is keyed by channel and page.
- The interface process stores nothing: no cookies, no files, no cache.
- **History, bookmarks, settings and the session are kept** by the trusted core in its settings folder, readable by the user alone (decisions 51, 57 and 59); persistent cookies are kept sealed (decision 62).

### 9.5 Navigation and the site-change screen

- **Navigations the user starts:** the address field, Back, Forward, a new tab, bookmarks, history, suggestions, Home and Reload. Built.
- **Navigations a page starts:** Fastext keys, page links, redirects, discovery results. Fastext keys and page links are built, decided by the trusted core (decisions 43 and 67); redirects that leave the channel and discovery results are not built.
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

Built (decision 41): the fetcher's transport runs `rustls` with Mozilla's root certificates (`webpki-roots`); a certificate that fails its check fails the connection, and nothing offers a way past it (decision 24). The strip shows a padlock for a channel reached over Transport Layer Security and "Not secure" for plain HyperText Transfer Protocol.

### 9.8 Local files

Each file is its own channel, with no cookies. It is read by the trusted core and decoded by its own decoder, never parsed by the trusted core. A file is limited to 64 megabytes, and a folder to 4,096 page files.

---

## 10. How a page reaches the screen

1. **The decoder renders a frame:** 480 by 500 palette indices (40 columns of 12 dots, 25 rows of 20 lines), a palette of at most 32 colours, the page's Colour Map with its redefinitions (decisions 6 and 60), and its screen colour, in the engine's resolved-colour mode with its bundled font, Bedstead, at the engine's default presentation level. It renders only the pages the trusted core says are shown.
2. **The trusted core checks the frame:** a known channel and page, the exact raster size, a palette of at most 32 entries, every index within the palette. Frames for pages the interface does not show are dropped.
3. **The trusted core encodes the image** in `protocol::frame`'s smallest form: raw, in runs, or as changes against the last frame it sent for that page, which the interface holds (decision 61); a frame identical to that one is not sent. A page's frames are sent at most once a field, 20 ms (`frames::MIN_FRAME_INTERVAL`, the engine's `FIELD_PERIOD_US`; finding F-11): nothing on a teletext display changes faster, the fastest flash phase lasting a twelfth of a second and a page answering keys being pushed twenty times a second (`interaction.md` section 2). A frame that comes sooner is held, the newest replacing any held before, and sent when the page's interval is up, encoded against the last sent; so a decoder alternating two images cannot fill the interface's queue (`a_page_s_frames_are_sent_at_most_once_a_field`).
4. **The trusted core sends the image to the interface process,** with the page's key and a frame version.
5. **The interface process reads it** with the same module's strict reader, against the frame of the page it holds: at most 32 colours, the screen colour and every dot within them, the dots filling 480 by 500 exactly. It keeps the page's image until a newer frame replaces it whole (decision 8), and draws the page scaled into its rectangle with nearest-neighbour sampling, inside a border of its screen colour one character cell wide (decision 60).
6. **Flash** is rendered by the decoder at the engine's timing, by its own clock: when a shown page has flashing cells it wakes at the next flash unit at which a rate a shown page uses changes phase (one second in twelve, the engine's decision E.48; a page with slow flash alone is drawn twice a second, not seven times) and draws the page again if its phase changed, and step 3 passes on only the frames that changed (decision 47). The decoder also wakes when its audio receiver next needs the time (the engine's decision E.98); otherwise it sleeps until the trusted core writes to it.

**Showing many pages at once (decision 7).** The interface knows exactly which pages its layout puts on screen and reports them; the trusted core asks the decoder only for those, of the active channel only. A channel of 800 pages costs what is visible.

**Why an encoded image, not raw indices, on the pipe.** A page's 240,000 dots would be 240 kilobytes raw; in `protocol::frame`'s runs, or as changes against the last frame, it is far smaller: a flashing page's next phase measured 2.1 kilobytes (decision 61). The interface's reader sees only what the trusted core's encoder produced. In one process (iOS, microcontrollers) frames go from decoder to interface as indices, with no encoding.

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

- **Redrawing is on demand:** only on input, a change of state, or while something animates, up to the frame that ends it: a frame drawn part of the way through a zoom or the remote's slide owes the next, however late the host asks (`a_zoom_drawn_short_of_its_end_asks_for_the_frame_that_ends_it`, `a_slide_drawn_short_of_its_end_asks_for_the_frame_that_ends_it`); and frames are asked for only for what is drawn (decision 53): an update to a page of another bank starts no frames beyond the overview's ring. The sound device runs only while a channel sounds, so an idle browser costs the audio thread nothing.
- **Widgets are our own** (design section 4.3, option B): the address field's editor is in the interface core, so it works without the standard library; text is laid out by Parley. Masonry, the alternative, is not proven (open decision 4).
- **Text** is Roboto, shipped with the application under the SIL Open Font License 1.1, in small capitals: every letter a capital, letters written in lower case drawn 80% as large. Open Sans is shipped too, for comparison.

### 11.2 What the window shows

| Part | What it does |
|---|---|
| Tab strip | Coloured tabs (the colour chosen by the trusted core from the site), close buttons, a new-tab button; a press of the middle button on a tab closes it (decision 63). On macOS the system's window buttons sit at its left end; elsewhere the window draws its own, the strip drags the window and its edges resize it; a tab dragged along the strip moves there, and dragged off it opens a window of its own or joins another window's strip; tabs that do not fit wrap into a panel joined under the strip (decisions 69 and 72) |
| Trusted strip | Back and Forward, dimmed when history has nothing, then Reload and Home (decision 63); a padlock, "Not secure" or "File"; the address with its host in full ink; a bookmark star; a menu button. A click on the address selects it for typing; Enter sends it as strip input |
| Page banks | 100 to 800, each with its colour, range, name (from the channel's contents file) and page count |
| Page list | The chosen bank's pages, number and title; Up and Down step through it; three digits typed choose a page |
| Page area | The bank's pages as a grid, sized by a logarithmic control that shrinks them over its whole travel to 80 pixels a page and shows a list only at its left edge. A click zooms a page to fill the area over 300 milliseconds, the wall dimming behind it; Escape or the close button returns. At the control's right end the zoomed page is shown, and the wheel or Up and Down slide to the next page of the bank, snapping into place: each notch of a mouse wheel is one page, at most one per slide, because macOS reports a slow notch as a tenth of a line. A page under the pointer pops out slightly |
| Overview | All 800 pages in eight rows: empty, available (listed in the contents file but not yet arrived), loaded, shown; the counts. A page that has arrived pops up at its native size under the pointer, which becomes a hand, never covering the trusted strip. A chevron at its top right folds it into the footer, where it draws nothing else and asks for no redraws |
| Developer console | Docked in the overview's place while open: the tabs Network, Packets, Pages, Log and Cookies, the last managing cookies by site and channel without ever showing a name or value (decisions 51, 62 and 63) |
| Tips | A label for each control drawn as an icon, a symbol or text that may be cut short, saying what it is and does and its shortcut, shown when the pointer rests on it for half a second, over everything, put away by a press or by leaving (decision 63) |
| Updates | A page whose content changes is marked in amber: a ring bursts from its cell in the overview and a dot stays for a minute; its row in the list flashes and shows how long ago it changed; its frame in the page area glows and fades over 3 seconds |

### 11.3 Input

- Pointer, wheel, keys and text from input methods arrive through winit.
- **Cell clicks** are computed from the pointer position and the rectangle the page was drawn in, which the interface core laid out itself, so they are exact.
- **Keys** that teletext needs: Fastext colours, Index, reveal, hold, mix and sub-page steps reach the trusted core as `PageControl` strip input (decision 67); page-number keys are handled by the interface in the list itself; on a channel that takes input every key goes to the channel through the trusted core's input session (decision 48).

### 11.4 The trusted strip's rules

- It shows only what the trusted core sends: the address as the address rules formatted it, the identity, the encryption state.
- It stays visible in every layout, including the zoomed page.
- Its input is strip input, message kinds of their own (section 6.4).
- The site-change screen is drawn in the strip only, never in the page area, so a page image cannot imitate it in place.

---

## 12. The application update mechanism

`installation-and-updates.md` is the design (decision 54), built on 23 October 2026 (decisions 73 to 82): a launcher that chooses, accepts, tries, commits and rolls back versions; a confined updater that verifies The Update Framework's repository and stages; per-user installs with no administrator on every desktop. In brief, as the earlier design (`prototypes/docs/architecture.md` section 12) put it:

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
| Windows | winit | as above | **Built, tested and run** on Windows 11 (build 26200) in a virtual machine, 19 October 2026: the whole workspace's tests pass, a page's sound plays through WASAPI (decision 40), and no console window opens. Every process but the broker and the launcher confined since 22 October 2026 (decisions 52, 56 and 58); the Dock menu has no Windows counterpart (a taskbar jump list) |
| Linux | winit (X11 and Wayland) | as above | **Built, tested and run** on Ubuntu 26.04, 19 October 2026, under X11: the whole workspace's tests pass, and a live sound plays through ALSA (decision 40). Every process but the broker and the launcher confined since 22 October 2026 (decisions 52, 56 and 58), on kernel 6.10 or later with unprivileged user namespaces (`sandboxing.md` section 8.2) |
| iOS, Android | winit | as above; the graphics-processor back end may matter most here | Later (decision 24); frame time at phone resolutions to be measured |
| Microcontrollers | the board's own display and input drivers | `embedded-graphics`, bitmap fonts | **The interface core drawn into a display in memory** by a test; no build for a microcontroller target yet |

**What the microcontroller build cuts** (to be decided with the owner): smooth text, screen readers, input methods, multiple tabs and windows, developer tools, and probably the network fetcher, depending on where its teletext comes from. It keeps pages, page-number and Fastext keys, flash and a simple page wall. Its security model is that of a single-purpose device and needs its own analysis.

**The display scale.** Every measurement so far was at a scale of 1; drawing on the processor costs about four times as much at a scale of 2.

---

## 14. Code layout

A module's tests are a `tests.rs` in the module's own folder (`session/tests.rs` for `session.rs`), or a `tests/` folder there where they need more than one file; tests that start processes are in the crate's own `tests/`.

```
neoteletext-browser/
  native/                   the application (a Cargo workspace)
    interface-core/         the window's state, layout, animation and input; no standard library
      src/                  the drawing interface (canvas.rs), geometry, the address field's editor, the
                            theme, tweens, page sizes, small text helpers, and the views the trusted core
                            sends: channel, console, controls, cookies, library, links, session, settings
      src/browser/          the window, a file for each part: the tab strip (+ tab_strip/, dragging and
                            drawing), the toolbar, the address field, its suggestions, Back and Forward's
                            lists, the menu, drop-downs, the banks, the page list, the overview, the page
                            controls, hyperlinks, typing (+ typing/draw.rs), the phone and its keys, the
                            certificate-error screen, tips, keys, focus, the pointer (+ pointer/press.rs),
                            frame timing, and regions, where each part was drawn, for hit-testing
      src/browser/<panel>/  the strip's panels, each a file with a folder of its drawing and its input:
                            bookmarks/, history/, settings_panel/, console_panel/ (its Cookies tab is
                            cookies_tab.rs), remote/; the star's popover and the bookmarks bar beside them
      src/wall/             the page area: the grid, the page under the pointer, the zoom, update marks,
                            the page-size control, and its drawing
    draw-vello/             the drawing back end for desktops and phones: vello_cpu and Parley, with the
                            fonts; a frame's recorded calls, its damage, rasterising, text and page images
    draw-embedded/          the drawing back end for microcontrollers: embedded-graphics
    protocol/               every message between processes: framing, fields, bounds, kinds, text cleaning,
                            page images (frame.rs), and a module for each pipe and feature: sound, audio,
                            the phone keyboard (keyboard.rs), settings, the console, controls, cookies
      src/interface/        the trusted core and the interface: kinds, tabs, pages, the strip, windows,
                            and each direction's reader and writer (to_core.rs, to_interface.rs)
      src/decoder/          the decoder's pipe: kinds, rendered frames, the open page's view, bookmark files
      src/fetcher/          the fetcher's pipe: kinds and the cookie messages
      src/library/          history, bookmarks and the address field's and Back and Forward's lists
      src/links/            what a decoder finds, Fastext, and what the interface is told of links
      src/interaction/      typing: the viewer's intents and the typing state
    confine/                the processes' sandboxes (sandboxing.md): spawn.rs, a parent's side and a
                            process's own (apply), the roles, each role's environment, a started
                            helper, the local time; macos.rs, the sandbox-exec profiles, each path a
                            parameter
      src/broker/           the broker's wire format, its serving side and the trusted core's side
      src/linux/            the namespaces, Landlock, seccomp, the paths each role may read, the private
                            /run and /tmp of the interface and the sound helper, and the tests' probes
      src/unix/             the broker's channel on macOS and Linux, descriptors passed, the local zone
      src/windows/          AppContainer profiles, the job, the attribute list, access lists, handles, a
                            confined process and its environment block, the broker's channel, the zone
    core/                   the broker and the trusted core (neoteletext, and neoteletext --core); build.rs
                            delay-loads shell32.dll on Windows
      src/                  the library (lib.rs), each part pure logic or a kept file's format: addresses,
                            bookmarks (+ bookmarks/: the file, changes, import), the helpers' budget, the
                            calendar, the console's entries, cookies (+ cookies/file.rs, the sealed file),
                            dropped lines, frames, hearing, history (+ history/search.rs), links (+ links/:
                            targets, destinations, the strip's questions, following), the outbox, private
                            files, the session file (restore.rs), settings (+ settings/proxy.rs), sleep,
                            the start page, typing (+ typing/records.rs); and the binary: main.rs, the
                            entry alone, broker.rs, the unconfined broker, source.rs, a file channel's bytes
      src/session/          the tabs and windows: a tab, its history and navigation, its channel, closed
                            tabs, windows set aside, the session saved, and what links ask of it
      src/trusted_core/     Core and its events, a file for each concern: the helper processes, channels,
                            timing, windows, intents, decoder and fetcher messages, the strip, the console,
                            settings, sound, updates, the phone, links, typing, start-up, the run, panel
                            numbers, page controls, library/ (history, bookmarks, the star, the bar),
                            cookies (+ cookies/panel.rs); hostile_tests.rs, the hostile interface harness
    decoder/                the decoder (neoteletext-decoder): its process (process.rs) and the engine as
                            it uses it (engine.rs, + engine/: feed, render), changes, flash, input records,
                            links, titles, the voice sink, and the Netscape bookmark reader (netscape.rs)
    fetcher/                the fetcher (neoteletext-fetcher): its arguments, its loop (process.rs), the
                            engine's agent's own parts, its cookies, and the transport over the network
    interface/              the interface process (neoteletext-interface): the system window, the reader
                            thread, keys, and app.rs with app/: frames, input, the input method, typing,
                            the phone, the clipboard and menus, windows, what the core said, outputs
    keyboard/               the phone keyboard process (neoteletext-keyboard): qrkeyboard's relay, pairing
                            and keys
    dock-menu/              the Dock menu and the menu bar on macOS (decisions 39 and 84); unsafe code in
                            src/mac.rs alone
    web-opener/             hands a checked web address to the system's web browser (hyperlinks.md section
                            12); unsafe code in src/windows.rs alone
    sound/                  the sound helper (neoteletext-sound): its loop, the mixers (audio.rs) and the
                            output device (audio/device.rs)
    site/                   a development tool, not shipped (neoteletext-site): builds and serves a test
                            channel; the podcast (+ podcast/wave.rs)
      src/terminal/         the reference channel that takes input: the server, pages, the session (+
                            session/audio.rs), the phone keyboard's page, the game, sounds, TTI
    update/                 what the launcher, the updater, the installer and the release tool share:
                            versions, the platforms' release files, the manifest, the launcher's state,
                            gzip, the release key's signatures, the version mark, the updater's messages,
                            the installed layout, safe writes, acceptance, profiles, fault points
    release-identity/       the release's identity, built in at build time (production/, or a test identity)
    launcher/               the launcher (neoteletext-launcher): its state, a start's trial (trial.rs),
                            starting the broker (start.rs), tidying, a launcher release, the command line,
                            macOS's bundle (+ macos/translocation.rs), Windows, and on the terminal
                            --install and --uninstall (unix.rs, + unix/linux.rs)
    os-install/             the unsafe calls installing needs: CoreFoundation's alert (src/macos.rs);
                            WinVerifyTrust, the registry, a message box and the DLL search (src/windows.rs)
    stub/                   the Windows stub, NeoTeletext.exe, which starts launcher-a.exe or launcher-b.exe
    installer/              the Windows installer, NeoTeletext-Setup.exe, carrying its release; the Start
                            menu shortcut (shortcut.rs)
    tuf/                    a workspace of its own: updater/ (neoteletext-updater, confined, with tough:
                            the client, the transport, staging, the helper) and release/ (the release tool
                            and a test HTTPS host), and their fuzz targets
    fuzz/                   fuzz targets for every reader of input from outside a process, and regressions/,
                            the inputs that once failed (nightly, outside the workspace)
  docs/                     this document, the requirements, the security analysis, the design record
  scripts/check.sh          the one check script: "commit" before each commit, "full" before each push
  scripts/try.sh            the edit loop: Clippy on every save, or one crate's Clippy and tests
  scripts/tools.sh          the pinned development tools, installed into the ignored tools/
  rust-toolchain.toml       the pinned compiler
```

Unsafe code is allowed only in the modules requirement 10.12 names, each saying so with its own `#![expect(unsafe_code, reason)]` or `#![allow(unsafe_code, reason)]`: in `confine`, `src/windows.rs` and every leaf of `src/windows/` but `channel.rs`, `src/linux/namespace.rs`, `runtime.rs` and `probe.rs`, and `src/unix/descriptors.rs` and `time.rs`; `os-install`'s `src/macos.rs` and `src/windows.rs`; `dock-menu`'s `src/mac.rs`; and `web-opener`'s `src/windows.rs`.

`scripts/updates-e2e.sh` and `scripts/updates-e2e.ps1` are the end-to-end tests of installation and updates, run by hand; `scripts/release/` holds the release procedure's steps (`installation-and-updates.md` section 10).

---

## 15. What is built and what is not

**Built** (23 October 2026), with 440 tests passing in `native/` (`cargo test --workspace --all-features` on macOS) and the updater's own in `native/tuf`:

- The broker, the trusted core, the decoder, the fetcher, the interface process, the sound helper, the phone keyboard and the updater, each its own program, communicating only over the bounded protocols of section 6.4; and the launcher, which starts the broker.
- The engine at a pinned commit, `fb2dd8a`, decoding T42 recordings, Teletext Text Interchange page files, transport streams and sliced records.
- Web channels over the HyperText Transfer Protocol, plain or with Transport Layer Security: the contents file, fetching in its order at the engine's rate, push over Server-Sent Events and WebSocket, update marks in the window.
- File channels, fed at the same pace.
- The window of section 11, drawn on the processor, redrawing only the boxes that changed since the last frame (`native-interface-design.md` section 12.7); and the same interface core drawn by the microcontroller back end in a test.
- Every process but the broker and the launcher confined on macOS, Windows and Linux from "deny everything" (decisions 52 and 58, 22 October 2026); sound in a helper of its own (decision 55); the trusted core confined behind the broker (decision 56; `sandboxing.md` section 7).
- Builds and tests on Windows, in a virtual machine, and on Linux (`DEVELOPMENT.md` section 12).
- Transport Layer Security and `https` channels (decision 41); fuzz targets for every reader of another process's messages and of every file the browser keeps (requirement 4.6).
- The address rules of section 9.6; tabs.
- Sound: every tab heard and each muted (decisions 37, 38 and 40); on macOS the Dock menu (decision 39).
- Hyperlinks, phases 1 and 2 of `hyperlinks.md` (20 October 2026), behind the `links` feature, on by default: page numbers found in the text and `LT`, `LK` and descriptor links, decided by the trusted core; the underline drawn by the interface; the destination, the site-change screen and the web-browser prompt in the strip; the hand-off to the system's web browser (`web-opener`); `#302` page addresses.
- Channel push over WebSocket, typing back to a channel, its cursor and the phone keyboard (`interaction.md`, decisions 45 and 46, 20 October 2026); input protocol version 2 (decision 48) and pasting into a page that takes input (decision 49), 21 October 2026.
- Settings kept between runs with the bookmarks, the settings panel of the strip, Home and the start page `neoteletext:home`, the developer console, and HTTP and SOCKS5 proxies (`settings-home-console-proxy.md`, decision 51, 21 October 2026).
- History and bookmarks (`history-and-bookmarks.md`, decision 57, 22 October 2026): history kept 90 days with its panel, search and removal, private tabs, recently closed tabs, Back and Forward lists, address-field suggestions; bookmarks in folders with the star's popover, a manager, a bookmarks bar, import and export in this browser's format; the menu button's menu.
- The session kept after every change and restored at start, with its setting (`history-and-bookmarks.md` section 15, decision 59, 22 October 2026).
- The Colour Map of Levels 2.5 and 3.5 and the screen colour (decision 60), and page images in the browser's own form (decision 61), 22 October 2026.
- Cookies held by the trusted core, one jar for ordinary tabs and one for private, fetchers asking per request; kept between runs sealed under a key in the platform's secret store; the cookies panel, its settings, clearing with history and `Clear-Site-Data` (`cookies.md`, decision 62, 22 October 2026).
- Reload, closing a tab with the middle button, cookie management in the developer console, and tips on every icon and symbol (decision 63, 23 October 2026).
- History following Chrome's rules (`history-and-bookmarks.md` section 16, decision 64, 23 October 2026): reloads counted as Chrome counts them, lists of 12 with chapter stops, removal reaching the tabs' lists, closed tabs kept between runs, the last 15 minutes, a row's menu.
- Fastext, reveal, hold, mix and sub-pages (`page-controls.md`, decision 67), with the receiver's row 24 of pages with `LN` lines; the remote control (`remote-control.md`, decision 68); and dragging tabs and their overflow (`tab-strip.md`, decision 69), 23 October 2026.
- The console's Log, each line with its level and process, and the channel's own `LG` lines (decision 70); bookmarks imported from other browsers' Netscape files, and "Move…" (decision 71); multiple windows, with tabs moved and torn off between them (decision 72), 23 October 2026.
- Requirement 3.9's hostile interface harness against the trusted core (`native/core/src/trusted_core/hostile_tests.rs`), run by the commit gate, with bookmarks, the start page and a second, private window; on macOS and Linux against a stand-in broker whose requests and helpers' input it checks (finding F-98).
- Installation and updates on every desktop (`installation-and-updates.md`, decisions 54 and 73 to 82, 23 October 2026): the launcher, the confined updater with `tough`, the Windows stub and installer, Linux's `--install`, removal, the release tool, and the core's offer, setting and health check, proved end to end with test keys on macOS, Windows and Linux.
- The certificate-error screen (`certificate-errors.md`, decision 83, 23 October 2026).
- The menu bar on macOS, and cut, copy and paste in every text field on every desktop (`menu-bar.md`, decision 84, 23 October 2026).
- The source reorganised by concept, 24 October 2026 (`~/devel/neoteletext-common/structure/`, documents 20 to 23): each crate's files split by what they hold, every module's tests in a `tests.rs` in its own folder, every function under 80 lines and every `impl` block under 300, with no `too_many_lines` allow left; no file of code over 500 lines but the fetcher's `transport.rs`, which the security fixes grew to 564 (`TECH-DEBT.md`); the bytes of 150 messages on every pipe the same before and after; the modules that may hold unsafe code split into leaves, each saying so (requirement 10.12). Section 14 is the layout.
- The fixes of the security review of 24 October 2026 (`~/devel/neoteletext-common/reviews/2026-10-24-security/FIX-PLAN.md`, decisions 86 to 100), each with a regression test that failed on the code before it: on Linux no helper reaches the session's sockets (decision 86), calls by the x32 numbers, types into its terminal or signals another process (decision 94), and each ends with its broker; on macOS every path passed to a profile as a parameter (decision 97), the fetcher kept off this computer unless allowed (decision 89), and the interface's and the sound helper's reads narrowed to this user's own folders; on Windows the interface in an AppContainer of its own (decision 90), its test passing in the virtual machine; the broker reading only teletext, never through a link, and opening the web at most once a hand-off interval (decision 88); the launcher accepting a copy of each version on every desktop (decision 87), a floor on the update root's version (decision 91), the Windows manifest signed (decision 92), a start deadline and then a health deadline with the broker's `started` line (decision 93), a narrow AppArmor profile (decision 95), and the updater's requests, its clock and the launcher's recovery bounded; phone keys outside a typing page only moving between pages (decision 99); each helper's messages held to a budget of its own, a page's frames to one a field, a fetcher's connection to one deadline and a name leading to this computer refused before connecting; slicing a string at a byte offset denied in every crate that reads outside input. What is left is in `TECH-DEBT.md` and, for the owner, `PENDINGTASKSFORANDREW.md`.

**Not built:**

- The site-change screen for redirects and discovery results.
- Any sandbox on Android or iOS; Windows helpers started apart from the console they were started from (decision 100 item 1, measured as needed in the virtual machine on 24 October 2026); the fetcher's connections made by the broker (decision 89); a file chooser owned by the broker (decision 88).
- Sharing one decoder between tabs on the same channel (decision 11).
- The grace period for closed tabs; the live-feed threshold; refresh schedules.
- Discovery.
- Full screen of the page alone (`full-screen.md`, decision 85, designed 23 October 2026).
- Screen readers, the input method's text shown while it is composed, reduced motion.
- The hostile interface harness with the real helpers running (it runs against the trusted core with a stand-in broker and stand-in helpers).
- A release signed by the owner's keys: the certificates, the hardware keys, the update host and the key ceremony are the owner's (`PENDINGTASKSFORANDREW.md`); the Linux release build on the C library 2.28; an icon of the browser's own.
- Builds on phones and microcontroller targets.

---

## 16. Decisions still open

None. Every decision below was taken on 20 October 2026 (decisions 41 and 44), each as recommended; the questions are kept for reference.

| Number | Question | Decided (decision 41) |
|---|---|---|
| Open decision 1 | Which library gives the fetcher Transport Layer Security? | Ask the engine first (section 5.1); otherwise `rustls`, which is pure Rust and permissively licensed |
| Open decision 2 | Does the decoder also run inside the WebAssembly interpreter `wasmi`, as the earlier design required? It is the decoder's only wall on iOS, where there is one process | Yes on iOS at least; measure its cost on desktops before deciding there |
| Open decision 3 | Is the trusted strip drawn by the interface process, as now, or by a smaller separate process, so that a fault in the interface's drawing cannot forge strip input (`security-analysis.md` section 5.6)? A separate process's window can be covered by other windows (`prototypes/docs/proofs.md` section 3), so it would have to draw in the same window | Keep it in the interface process; make the interface process as small and as tested as the trusted core's readers, and revisit if the harness or fuzzing finds faults |
| Open decision 4 | Widgets: our own (built), or Masonry, once it draws with `vello_cpu` in a release | Our own, as built; reconsider for screen readers and text fields of teletext forms |
| Open decision 5 | What does the microcontroller build cut, and where does its teletext come from? | Section 13 |
| Open decision 6 | The libraries of `native/Cargo.toml`, each by name (requirement 14.3) | Approve the list as built |
| Open decision 7 | The browser's own licence: it has no `LICENSE` file and no `license` field (`licensing.md` section 1) | MIT, as the engine |
| Open decision 8 | Hyperlinks: the proposal of `hyperlinks.md`, with its ten decisions (its section 16), the amendment of requirement 4.5 among them | Adopt it and build its phase 1 |
| Open decision 9 | Push, typing back and the cursor: the proposal of `interaction.md`, with its twelve decisions (its section 14), the amendment of requirement 4.7 among them | Adopt it and build its phase 1 |

---

## 17. Glossary

| Term | Meaning |
|---|---|
| AccessKit | A Rust library that publishes an application's accessibility tree to each platform's screen readers. |
| App Sandbox | The macOS sandbox for applications: a process runs with only the entitlements it is given. |
| AppContainer | The Windows sandbox for applications: a process runs with only the capabilities it is given. |
| Bedstead | The teletext font the engine bundles. |
| Chapter stop | In a Back or Forward list beyond its first 12 entries, the nearest entry where the host changes, as Chrome's Back menu offers them by registrable domain (decision 64). |
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
| Page image | A page's palette, screen colour and dots as the trusted core sends them to the interface: raw, in runs, or as changes against the frame held (`protocol::frame`, decision 61). Portable Network Graphics images were used until 22 October 2026. |
| Parley | A Rust library that lays out and shapes text, with full Unicode and bidirectional support. |
| Presentation Level | The teletext standard's feature levels: 1.0 and 1.5 (basic), 2.5 (enhanced colours and characters), 3.5 (further enhancements). |
| Public Suffix List | Mozilla's list of the parts of a name under which anyone may register a site (`com`, `co.uk`, `github.io`); browsers use it to stop a cookie set for a whole suffix. Not used here: no cookie reaches beyond its host (decision 62, `cookies.md` section 1.1). |
| Registrable domain | The part of a host one step below a public suffix (`example.co.uk` for `news.example.co.uk`): a browser's "site". This browser groups by host or channel instead. |
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
