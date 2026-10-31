# NeoTeletext: architecture

| | |
|---|---|
| **Status** | Adopted 16 October 2026 (decision 33). Built in part, in `native/`; section 15 says what is built and what is not. |
| **Date** | 16 October 2026 |
| **Companion documents** | The requirements and the owner's decisions are records in rdm (since 30 October 2026, rdm decision DEC-1389): each requirement numbered ("Requirement 6.3"), with its reason, how it is verified and whether it is met, and each decision ("Decision 50") with its reason and what holds now. Section numbers here are unchanged; sections 2 and 16, which held the decisions, are gone. [`security-analysis.md`](security-analysis.md) analyses this architecture's security. [`hyperlinks.md`](hyperlinks.md) specifies hyperlinks (decision 41) and [`hyperlinks-security.md`](hyperlinks-security.md) analyses their security. [`interaction.md`](interaction.md) specifies channel push, typing back and the cursor (decision 44) and [`interaction-security.md`](interaction-security.md) analyses their security. [`licensing.md`](licensing.md) says which licences may sit beside the browser's code. [`native-interface-design.md`](native-interface-design.md) records how this architecture was chosen, the prototypes it was chosen from, its review against the earlier design, and what was built when. |
| **The earlier design** | The Tauri design that came before, with its own architecture, requirements, security analysis and proofs, was kept in `prototypes/docs/`, beside the prototypes that tested it, until they were removed on 21 October 2026 (decision 50); they are in git at the tag `prototypes-final`, and a path under `prototypes/` in this document is a path there. This document replaces it; where the two differ, this one holds. |
| **Engine documents** | The teletext engine's own design lives in `~/devel/neoteletext-engine`: `ARCHITECTURE.md`, `docs/spec/requirements/web.md` (channels, REQ-WEB-023 to REQ-WEB-026, and discovery) and `docs/spec/decisions.md`; its `docs/channels.md` and `docs/web-discovery-security.md` left the engine with its browser on 18 October 2026 (engine commit `94346b0`) and are in its history. This document refers to them by path. |

**Abbreviations.** This project avoids abbreviations, because they hide meaning. Where a technical term is normally known by its initials, the full name is written out, followed by the initials in brackets only when the initials are a literal identifier in code, configuration or a standard's title. Section 17 is a glossary of every technical term used.

---

## Contents

1. What NeoTeletext is
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

## 3. Concepts and terms

| Term | Meaning in this project |
|---|---|
| **Teletext page** | One screen of teletext: 40 columns by 25 rows of character cells, row 0 being the header. Identified by magazine (1 to 8), page number (00 to FF) and subcode. |
| **Subpage** | One of several versions of a page carrying the same page number, distinguished by subcode. |
| **Channel** | One teletext service. On the web it is identified by its **origin** and its **folder path** (`neoteletext` decision E.70, the engine's `docs/spec/requirements/web.md`): a folder holding `channel.json`, `contents.txt`, optionally a bundle `index.t42`, and page files such as `101.tti`. A file channel is a recording or a folder of page files. The channel is the unit of trust and isolation: it plays the part a site plays in a traditional browser. |
| **Origin** | The combination of scheme, host and port of an address. |
| **Tab** | One view onto one channel at a time, with its own history. |
| **Bank** | A magazine, as the window calls it (decision 105): a hundred pages, such as 100 to 199: teletext's convention gives each section a hundred, with the section's index on its first page. The window shows the eight banks 100 to 899. |
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

**It is a separate project** (decision 36). NeoTeletext depends on its crates, in every workspace, at one pinned commit, `75db08c` (30 October 2026: SELECT names a connection's one selection, with no view number, neoteletext-common's DEC-1527). Before it, `f393623` (30 October 2026: SELECT, every pushed packet checked and the invalid ones counted, a live channel opened by its connection alone, and its index, LOG, SET_COOKIE, LINKS and SUBPAGES records, neoteletext-common's DEC-1459 and DEC-1500). Before it, `a58958f`; before that, `8c59a89` (28 October 2026: every move over the page paced at 60 a second by `pace::Pacer`, touch numbered by `contacts::Contacts` with MOUSE's device and TOUCH, MOUSE action 3 gone, resumption of a lost session with `ResumableSession` and RESUME, the round trip measured by ping and pong, the messages of one moment written together and moves held per contact while the socket is congested, and pointer areas withdrawn, the engine's decision E.133 (decision 125). Before it, `e09a589`, 28 October 2026: every address World System Teletext defines accepted and every sub-page to the standard's 8,190 held, the numbers in `neoteletext_core::limits`, the engine's decision E.130 and its area NUM; PCM16 and IMA ADPCM each behind a feature, which the desktop builds leave out, with the `.ttau` file and WebSocket readers and fades at a lost block, E.131; and each sub-page a file of its own with each page's catalogue, `NNN.sub`, E.132 (decision 121). Before it, `dcb2a3b`, 27 October 2026: no input modes, every event sent, moves only while a button is held or within a pointer area a page declares, and HOVER over controls, the engine's decisions E.125, E.128 and E.129, built on its E.122, the standard's maxima defined once. Before it, `c0cdc6a`, 27 October 2026: a page's controls, read from reserved column mode 00101, and the hover and pressed looks drawn on them, the engine's decision E.127, built on its E.126, the TTI Form Extension withdrawn, and its E.123 and E.124, the integer Opus decoder carried as codec 2. Before it, `11149ef`, 26 October 2026: a found page shows the latest clock, decision E.121. Before it, `eb6b2d2`, 25 October 2026: a contents title may be a whole headline, 120 characters, decision E.120; and the wheel reaches a channel over its page in any mode, decision E.119. Before it, `c997514`, 24 October 2026: the engine's tests for its core's line coverage of 90 % (REQ-TST-041), which found and fixed an object placed outside every display area kept instead of discarded (REQ-OBJ-014) and a DRCS page known only by its X/28/3 kept as a text page (decision E.117); and `p256` moved to 0.14 for advisory RUSTSEC-2026-0387 in the `crypto-bigint` it brought in (decision E.118). Before it, `5311f98`, the same day: the engine's source reorganised by concept with every public path kept; its fixes from NeoTeletext's security review of that day, decisions E.112 to E.116: a 307 or 308 form redirect kept to its target's origin, a wider list of local names through a proxy, the web user agent's narrower choices for cookies, redirects, folders, event ids, close codes and IPv6 hosts, more local address ranges and embedded IPv4 addresses judged as themselves, and a qrkeyboard phone trusted less, reported connected only once a frame authenticates; and the mixer's buffer-taking forms, so a port allocates a sound outside its lock. Before it, `fb2dd8a`, 23 October 2026: E.106 to E.111, every feature set built again, the qrkeyboard waker, a proxy's answer read in chunks, the receiver's row 24 and links in the formats crate, channel log lines, Device Bound Session Credentials behind a feature, and fonts named by channels withdrawn), through a `git` source on the engine's published repository, `https://github.com/neoteletext/neoteletext-engine`, since 28 October 2026 (before it, the engine's local repository, while its commits were unpublished); Cargo reads the commit, never a working tree, so a pin moves only to a commit already pushed there. Changes the engine needs are made in the engine's repository by its own process, never patched here.

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

**Limits NeoTeletext takes from neoteletext.** Among them: `contents::MAX_PAGES` (800 pages), `contents::MAX_BYTES` (64 kilobytes), `DEFAULT_PAGE_LIMIT` (256 kilobytes), `DEFAULT_PUSH_LIMIT`, `DEFAULT_DESCRIPTOR_LIMIT`, `DEFAULT_HEADER_LIMIT`, `DEFAULT_FETCH_INTERVAL_US` (one request every 0.2 seconds), `DEFAULT_CONNECT_TIMEOUT_US` (10 seconds), `DEFAULT_RESPONSE_TIMEOUT_US` (30 seconds), and `MIN_REFRESH_SECONDS` and `MAX_REFRESH_SECONDS`. The full table is in `neoteletext/docs/neoteletext-browser-open-questions.md` section 3. *To change (decisions 112 and 119, not built):* `contents::MAX_PAGES`' 800 becomes the 800 keyable page numbers of the engine's `limits`, beside the 2,022 that may carry a page, and every page and sub-page number comes from `limits` (the engine's area NUM). *Built 28 October 2026 for sub-pages and sub-codes (decision 121):* every sub-page count and sub-code is `limits`'; the pages are still the 800 keyable.

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
| InputControl | An input control record (WELCOME; the withdrawn mode records are ignored), untrusted (REQ-INT-111) |
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
| Core to interface | Link: pointing, prompt | What the strip shows of the link under the pointer or the focus; the web-browser prompt, with its number |
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
| Interface to core | Strip input: navigate, back, forward, bookmark, confirm, cancel | What the user did in the trusted strip, including the answer to its question, which names the question. These are message kinds of their own; a page intent reaches another channel only by a link the trusted core holds (REQ-685) |
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

Discovery's security analysis was the engine's `docs/web-discovery-security.md`, in its git history since 18 October 2026; its requirements are in the engine's `docs/spec/requirements/web.md`. Its results are never loaded directly: each is shown to the user, and a chosen one is a page-initiated navigation (REQ-694).

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

Decision 22, **rescinded on 25 October 2026 by decision 104**: pages are to be fetched while they are visible or have focus. What follows is what is built, until decision 104's fetching is designed. The pattern of network requests must reveal nothing about which pages are viewed.

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
- A page cannot cause a request to another channel. Anything that would is a navigation the user chose, by a link the trusted core holds.

### 9.3 Cookies

The cookie store is the engine's, which follows the cookie specification draft RFC 6265bis as browsers implement it, made stricter: every cookie scoped to its host only, `Secure` cookies never sent over plain connections, the `__Host-` and `__Secure-` prefixes and expiry enforced, and no third-party context at all.

**Where it lives** (decision 62, `cookies.md`). The trusted core holds every jar, one for ordinary tabs and one for private ones; a fetcher holds none and asks for each request's header (requirements 7.4, 7.5 and 21.1). Persistent cookies are kept between runs sealed under a key in the platform's secret store; session cookies never are.

**No `Domain`, so no Public Suffix List.** A cookie never reaches beyond the host that set it, so the list browsers use to stop `Domain=co.uk` has nothing to do; where Chrome groups by registrable domain, this browser groups by host or channel. Why, and what it costs, is `cookies.md` section 1.1.

### 9.4 Storage

- The decoder's page store belongs to one channel.
- The trusted core's record of frames sent is keyed by channel and page.
- The interface process stores nothing: no cookies, no files, no cache.
- **History, bookmarks, settings and the session are kept** by the trusted core in its settings folder, readable by the user alone (decisions 51, 57 and 59); persistent cookies are kept sealed (decision 62).

### 9.5 Navigation

- **Navigations the user starts:** the address field, Back, Forward, a new tab, bookmarks, history, suggestions, Home and Reload. Built.
- **Navigations a page starts:** Fastext keys, page links, redirects, discovery results. Fastext keys and page links are built, decided by the trusted core (decisions 43 and 67); redirects that leave the channel and discovery results are not built.
- **A link or Fastext key that leaves the channel** opens the destination at once, in the same tab, with Back to return, as in a web browser (REQ-1670, decision DEC-1664). Only a `web:` link, which hands an address to another program, asks first, in the trusted strip.
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

1. **The decoder renders a frame:** 480 by 500 palette indices (40 columns of 12 dots, 25 rows of 20 lines), a palette of at most 32 colours, the page's Colour Map with its redefinitions (decisions 6 and 60), and its screen colour, in the engine's resolved-colour mode at the engine's default presentation level, its text in the font of the user's "Font" setting: the engine's Noto Teletext, bundled in the decoder and read by the engine's reader of loaded fonts, with Bedstead for every character it lacks, or Bedstead alone. Row 0, mosaics and the G3 characters are the engine's own in either. It renders only the pages the trusted core says are shown.
2. **The trusted core checks the frame:** a known channel and page, the exact raster size, a palette of at most 32 entries, every index within the palette. Frames for pages the interface does not show are dropped.
3. **The trusted core encodes the image** in `protocol::frame`'s smallest form: raw, in runs, or as changes against the last frame it sent for that page, which the interface holds (decision 61); a frame identical to that one is not sent. A page's frames are sent at most once a field, 20 ms, on each of three clocks (`frames::MIN_FRAME_INTERVAL`, the engine's `FIELD_PERIOD_US`; finding F-11; decision 122): its content; the look under the pointer, a frame that changes only cells whose look was asked for; and the answer, the first frame of content after the viewer's input to the page, so that neither the look's frame nor the header's clock holds the channel's answer (requirement 18.12). Nothing on a teletext display changes faster, the fastest flash phase lasting a twelfth of a second and a page answering keys being pushed twenty times a second (`interaction.md` section 2). A frame that comes sooner than its clock allows is held, the newest replacing any held before, and sent when its clock is up, encoded against the last sent; so a decoder alternating two images, even with a look and an input every millisecond, gets at most three frames of a page a field to the interface (`a_page_s_frames_are_sent_at_most_once_a_field`, `inputs_and_looks_keep_a_page_to_its_clocks`).
4. **The trusted core sends the image to the interface process,** with the page's key and a frame version.
5. **The interface process reads it** with the same module's strict reader, against the frame of the page it holds: at most 32 colours, the screen colour and every dot within them, the dots filling 480 by 500 exactly. It keeps the page's image until a newer frame replaces it whole (decision 8), and draws the page scaled into its rectangle with nearest-neighbour sampling, inside a border of its screen colour one character cell wide (decision 60). A frame of the page keys go to, or of the page under the pointer, is drawn as it arrives; other changes at most every 16 ms while keys go to a channel and every 100 ms otherwise, counted from the last frame that changed the window (requirement 18.11, decision 122).
6. **Flash** is rendered by the decoder at the engine's timing, by its own clock: when a shown page has flashing cells it wakes at the next flash unit at which a rate a shown page uses changes phase (one second in twelve, the engine's decision E.48; a page with slow flash alone is drawn twice a second, not seven times) and draws the page again if its phase changed, and step 3 passes on only the frames that changed (decision 47). The decoder also wakes when its audio receiver next needs the time (the engine's decision E.98); otherwise it sleeps until the trusted core writes to it.

**Showing many pages at once (decision 7).** The interface knows exactly which pages its layout puts on screen and reports them; the trusted core asks the decoder only for those, of the active channel only. A channel of 800 pages costs what is visible.

**Why an encoded image, not raw indices, on the pipe.** A page's 240,000 dots would be 240 kilobytes raw; in `protocol::frame`'s runs, or as changes against the last frame, it is far smaller: a flashing page's next phase measured 2.1 kilobytes (decision 61). The interface's reader sees only what the trusted core's encoder produced. In one process (iOS, microcontrollers) frames go from decoder to interface as indices, with no encoding.

---

## 11. The browser window

The window is decision 35's, drawn by the interface process from the interface core.

### 11.1 Layers

```
 interface core (ours, no standard library)
   tabs and strip · channel magazines · page list · page area · overview
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
| Tab strip | Coloured tabs (the colour chosen by the trusted core from the site), close buttons, a new-tab button; a press of the middle button on a tab closes it (decision 63). On macOS the system's window buttons sit at its left end; elsewhere the window draws its own, the strip drags the window and its edges resize it; a tab dragged along the strip moves there, and dragged off it opens a window of its own or joins another window's strip; tabs that do not fit wrap into a panel joined under the strip (decisions 69 and 72); at its right end, before the window's own buttons, the browser's version and BETA, "0.1.0 BETA", as a quiet label (decision 123) |
| Trusted strip | Back and Forward, dimmed when history has nothing, then Reload and Home (decision 63); a padlock, "Not secure" or "File"; the address with its host in full ink; a bookmark star; a menu button. A click on the address selects it for typing; Enter sends it as strip input |
| Channel magazines | 100 to 800, each with its colour, range, name (from the channel's contents file) and page count (decision 105; "bank" in the code) |
| Page list | The chosen bank's pages, number and title; Up and Down step through it, and through the magazines when they were clicked (decision 105); three digits typed choose a page. The divider between it and the page area is pulled to resize it (decision 105) |
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
- The web-browser prompt is drawn in the strip only, never in the page area, so a page image cannot imitate it in place.

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
    icon/                   the browser's icon (decision 124): the owner's art, neoteletext-icon.png, the one
                            source; its PNG reader and writer, nearest-neighbour scaling, the Windows
                            resource file (windows.rs) and the macOS iconset (macos.rs), for build scripts
                            and the release tool alone
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
- The engine at a pinned commit, `8c59a89`, decoding T42 recordings, Teletext Text Interchange page files, transport streams and sliced records.
- Web channels over the HyperText Transfer Protocol, plain or with Transport Layer Security: the contents file, fetching in its order at the engine's rate, push over Server-Sent Events and WebSocket, update marks in the window.
- File channels, fed at the same pace.
- The window of section 11, drawn on the processor, redrawing only the boxes that changed since the last frame (`native-interface-design.md` section 12.7); and the same interface core drawn by the microcontroller back end in a test.
- Every process but the broker and the launcher confined on macOS, Windows and Linux from "deny everything" (decisions 52 and 58, 22 October 2026); sound in a helper of its own (decision 55); the trusted core confined behind the broker (decision 56; `sandboxing.md` section 7).
- Builds and tests on Windows, in a virtual machine, and on Linux (`DEVELOPMENT.md` section 12).
- Transport Layer Security and `https` channels (decision 41); fuzz targets for every reader of another process's messages and of every file the browser keeps (requirement 4.6).
- The address rules of section 9.6; tabs.
- Sound: every tab heard and each muted (decisions 37, 38 and 40); on macOS the Dock menu (decision 39).
- Hyperlinks, phases 1 and 2 of `hyperlinks.md` (20 October 2026), behind the `links` feature, on by default: page numbers found in the text and `LT`, `LK` and descriptor links, decided by the trusted core; the underline drawn by the interface; the destination and the web-browser prompt in the strip; the hand-off to the system's web browser (`web-opener`); `#302` page addresses.
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
- Every sub-page to the standard's 8,190, counted, stepped and placed in 16 bits from the engine's `neoteletext_core::limits`, the sub-page list a virtual list of rows asked for by range, and sub-pages as files of their own with each page's catalogue (decisions 112, 119 and 121, 28 October 2026); PCM16 and IMA ADPCM compiled out of the desktop builds, which `scripts/check.sh` proves (decision 121).
- The browser's version and BETA at the tab strip's right end in every window, the version the trusted core reports running (decision 123, 28 October 2026).
- Requirement 3.9's hostile interface harness against the trusted core (`native/core/src/trusted_core/hostile_tests.rs`), run by the commit gate, with bookmarks, the start page and a second, private window; on macOS and Linux against a stand-in broker whose requests and helpers' input it checks (finding F-98).
- Installation and updates on every desktop (`installation-and-updates.md`, decisions 54 and 73 to 82, 23 October 2026): the launcher, the confined updater with `tough`, the Windows stub and installer, Linux's `--install`, removal, the release tool, and the core's offer, setting and health check, proved end to end with test keys on macOS, Windows and Linux.
- The certificate-error screen (`certificate-errors.md`, decision 83, 23 October 2026).
- The browser's icon, the owner's art, on every desktop (decision 124, 28 October 2026): the macOS bundle's icon file, made by `iconutil` and checked pixel for pixel, and the Dock's icon outside a bundle; a resource in the Windows stub, installer, launcher, broker and interface, written by a build script; Linux's icon files in the user's icon theme and the menu entry naming them; every window's icon on Windows and X11, and its class on Linux. Every size by nearest neighbour from the one source, `native/icon/neoteletext-icon.png`.
- The menu bar on macOS, and cut, copy and paste in every text field on every desktop (`menu-bar.md`, decision 84, 23 October 2026).
- Full screen of the page alone (`full-screen.md`, decision 85), tabs of one width with the overflow panel raised as a dialog (decision 102), and a tab's address in its tip (decision 103), 25 October 2026.
- The source reorganised by concept, 24 October 2026 (`~/devel/neoteletext-common/structure/`, documents 20 to 23): each crate's files split by what they hold, every module's tests in a `tests.rs` in its own folder, every function under 80 lines and every `impl` block under 300, with no `too_many_lines` allow left; no file of code over 500 lines but the fetcher's `transport.rs`, which the security fixes grew to 564 (`TECH-DEBT.md`); the bytes of 150 messages on every pipe the same before and after; the modules that may hold unsafe code split into leaves, each saying so (requirement 10.12). Section 14 is the layout.
- The fixes of the security review of 24 October 2026 (`~/devel/neoteletext-common/reviews/2026-10-24-security/FIX-PLAN.md`, decisions 86 to 100), each with a regression test that failed on the code before it: on Linux no helper reaches the session's sockets (decision 86), calls by the x32 numbers, types into its terminal or signals another process (decision 94), and each ends with its broker; on macOS every path passed to a profile as a parameter (decision 97), the fetcher kept off this computer unless allowed (decision 89), and the interface's and the sound helper's reads narrowed to this user's own folders; on Windows the interface in an AppContainer of its own (decision 90), its test passing in the virtual machine; the broker reading only teletext, never through a link, and opening the web at most once a hand-off interval (decision 88); the launcher accepting a copy of each version on every desktop (decision 87), a floor on the update root's version (decision 91), the Windows manifest signed (decision 92), a start deadline and then a health deadline with the broker's `started` line (decision 93), a narrow AppArmor profile (decision 95), and the updater's requests, its clock and the launcher's recovery bounded; phone keys outside a typing page only moving between pages (decision 99); each helper's messages held to a budget of its own, a page's frames to one a field, a fetcher's connection to one deadline and a name leading to this computer refused before connecting; slicing a string at a byte offset denied in every crate that reads outside input. What is left is in `TECH-DEBT.md` and, for the owner, rdm to-dos for the owner.
- The return channel's answer drawn without waiting (`~/devel/neoteletext-common/RETURN-CHANNEL.md` sections 2.2 and 2.4, REQ-RCH-001 to REQ-RCH-005; decision 122, 28 October 2026): the page keys go to, or the page under the pointer, drawn as its frame arrives, a frame that changed nothing delaying none; three clocks a page in the trusted core, so the channel's answer waits neither for the look's frame nor for the header's, three frames a field at most; what an event sends written once it is handled, before the frame it asks for; one frame queued on every platform, by inspection; and the middle, right, back and forward buttons and the mouse's modifiers sent over a page that takes input, a link the page declares being the browser's there as on every page (requirements 18.11 to 18.15).

**Not built:**

- Redirects that leave a channel, and discovery results.
- Any sandbox on Android or iOS; Windows helpers started apart from the console they were started from (decision 100 item 1, measured as needed in the virtual machine on 24 October 2026); the fetcher's connections made by the broker (decision 89); a file chooser owned by the broker (decision 88).
- Sharing one decoder between tabs on the same channel (decision 11).
- The grace period for closed tabs; the live-feed threshold; refresh schedules.
- Discovery.
- Full screen of the page alone (`full-screen.md`, decision 85, designed 23 October 2026).
- Screen readers, the input method's text shown while it is composed, reduced motion.
- The hostile interface harness with the real helpers running (it runs against the trusted core with a stand-in broker and stand-in helpers).
- A release signed by the owner's keys: the certificates, the hardware keys, the update host and the key ceremony are the owner's (rdm to-dos for the owner); the Linux release build on the C library 2.28.
- Builds on phones and microcontroller targets.
- Page numbers with a hexadecimal digit, which decision 119 accepts (`STANDARD-LIMITS.md` question 3); a server's catalogue of sub-pages, its titles by range and tuning by sub-page (decision 112 item 3), which await the server; a channel's IMA ADPCM and PCM16 sound on the desktops, by the owner's instruction (decision 121 item 7).

---

## 17. Glossary

| Term | Meaning |
|---|---|
| AccessKit | A Rust library that publishes an application's accessibility tree to each platform's screen readers. |
| App Sandbox | The macOS sandbox for applications: a process runs with only the entitlements it is given. |
| AppContainer | The Windows sandbox for applications: a process runs with only the capabilities it is given. |
| Bedstead | The teletext font the engine bundles. |
| Noto Teletext | The engine's Noto-derived 12 by 20 teletext font (its REQ-FNT-012), which the decoder bundles for the "Font" setting. |
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
