# NeoTeletext: architecture

| | |
|---|---|
| **Status** | Adopted 16 October 2026 (decision 33). Built in part, in `native/`; section 15 says what is built and what is not. |
| **Date** | 16 October 2026 |
| **Companion documents** | The requirements and the owner's decisions are records in rdm (since 30 October 2026, rdm decision DEC-1389): each requirement numbered ("Requirement 6.3"), with its reason, how it is verified and whether it is met, and each decision ("Decision 50") with its reason and what holds now. Section numbers here are unchanged; sections 2 and 16, which held the decisions, are gone. [`security-analysis.md`](security-analysis.md) analyses this architecture's security. [`hyperlinks.md`](hyperlinks.md) specifies hyperlinks (decision 41) and [`hyperlinks-security.md`](hyperlinks-security.md) analyses their security. [`interaction.md`](interaction.md) specifies channel push, typing back and the cursor (decision 44) and [`interaction-security.md`](interaction-security.md) analyses their security. [`licensing.md`](licensing.md) says which licences may sit beside the browser's code. [`native-interface-design.md`](native-interface-design.md) records how this architecture was chosen, the prototypes it was chosen from, its review against the earlier design, and what was built when. |
| **The earlier design** | The Tauri design that came before, with its own architecture, requirements, security analysis and proofs, was kept in `prototypes/docs/`, beside the prototypes that tested it, until they were removed on 21 October 2026 (decision 50); they are in git at the tag `prototypes-final`, and a path under `prototypes/` in this document is a path there. This document replaces it; where the two differ, this one holds. |
| **Engine documents** | The teletext engine's own design lives in `~/devel/neoteletext-renderer-teletext`: `ARCHITECTURE.md`, `docs/spec/requirements/web.md` (channels, REQ-WEB-023 to REQ-WEB-026, and discovery) and `docs/spec/decisions.md`; its `docs/channels.md` and `docs/web-discovery-security.md` left the engine with its browser on 18 October 2026 (engine commit `94346b0`) and are in its history. This document refers to them by path. |

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
- **Every tab is one teletext channel.** A teletext channel is one teletext service: eight magazines, numbered 1 to 8, each with pages numbered 00 to FF in hexadecimal, and each page with subpages. A viewer keys pages 100 to 899, which the lists and the overview hold; a page with a hexadecimal digit, such as 1A0, is reached by a link and drawn like any other (REQ-300, REQ-313), and every number but those Annex A.10 of the standard reserves is a page (`interface_core::channel::is_channel_page`).
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
| **Channel** | One teletext service. On the web it is identified by its **origin** and its **folder path** (`neoteletext` decision E.70, the engine's `docs/spec/requirements/web.md`): a folder holding `channel.ttx`, `contents.txt`, optionally a bundle `index.t42`, and page files such as `101.tti`. A file channel is a recording or a folder of page files. The channel is the unit of trust and isolation: it plays the part a site plays in a traditional browser. |
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

The engine is `neoteletext` (`~/devel/neoteletext-renderer-teletext`), a Rust implementation of World System Teletext, EN 300 706, Presentation Levels 1.0, 1.5, 2.5 and 3.5, less what the owner removed on 20 October 2026: dynamically redefinable characters (DRCS) are not decoded, and the Level 3.5 font styles (bold, italic and proportional spacing) are not drawn (its decisions E.95 and E.96). Its core is `no_std` (it does not depend on the Rust standard library) and performs no input or output of its own: it is fed bytes and returns results.

**It is a separate project** (decision 36). NeoTeletext depends on its crates, in every workspace, at one pinned commit, `4229773` (19 November 2026, the head of the engine's branch `parity`, through the same local `git` source: neoteletext-common's renderer parity wire vectors, PAR-44, in the engine's copy of the table, `neoteletext_web::wire_vectors`, which the decoder's and the trusted core's tests read, with the engine's writer and reader of them; no library code the browser uses changed). Before it, `d024352` (the same day, the head of the engine's branch `parity` then, through the same local `git` source: its NAPLPS renderer pin moved to `9122afd`, which the browser matches, and the picture text writer's ReGIS cell of 16 by 20 and NAPLPS rows written from their foot). Before it, `8b973b7` (the same day: a static channel's sub-page file may carry its picture past the page's bound, the engine's DEC-6810). Before it, `4595844` (the same day: the picture converter's NAPLPS output loads its colours in SET COLOR's order, `43e0f06b`, ANSI X3.110-1983 section 5.3.2.5.1, and the engine's NAPLPS renderer pin is `19f5bba`, which the browser matches so that one copy resolves). Before it, `cf95e76` (19 November 2026, the head of the engine's branch `parity` then, not yet published, so through a `git` source on its local repository, `file:///Users/andrewstuart/devel/neoteletext-renderer-teletext`, to move to the published repository at the same commit once it is pushed: renderer parity's capability bits `REGIS_CAP` and `NAPLPS_CAP` (bits 23 and 24, neoteletext-common's `docs/renderer-parity.md` PAR-1), `DRAWING_LINKS` and `REGIS_CURSOR` naming NAPLPS objects as well as ReGIS drawings (PAR-2), the joining data of the plain Arabic-script sets, `neoteletext_core::glyphs::joining` (PAR-3), and the picture converter's ReGIS and NAPLPS outputs, `neoteletext_picture::to_regis` and `to_naplps` (PAR-9), which bring the ReGIS renderer's `regis-minify` into the workspace; the ReGIS renderer moves with it to `9c1a162`, the head of its branch `parity-2` (the 55 provided sets as alphabets 100 to 154, `regis::ProvidedSet`, and their joining, PAR-12 and PAR-13), and the NAPLPS renderer to `2700040`, the head of its branch `parity-2` (joining, `naplps::Validator`, and the stroke font's bound `naplps::limits::MAX_STROKE_FONT_BYTES`, PAR-14, PAR-15 and PAR-35), both through `git` sources on their local repositories; the renderers need Rust 1.97, so the workspaces' `rust-version` is 1.97). Before it, `3e5ddb0` (19 November 2026, the head of the engine's working branch `livedevelopment`, pushed, taken again from the engine's public repository: `rules-2`'s `24b6a96` merged, with one documentation link mended and neoteletext-web no longer building the MIDI layer, so the trusted core links no instrument crate, DEC-2355; the ReGIS renderer moves with it to `f6d14d8`, the head of its working branch, which merges its branches `animation`, `parity` (labels on numbered drawings and a picture's title) and `fills` (each changed place repainted apart, filled polygons written a row's run at a time), and the NAPLPS renderer stays at `b435b7d`, now the head of its working branch; both from their public repositories). Before it, the same `24b6a96` (19 November 2026, the head of the engine's branch `rules-2`, through a `git` source on its local repository: open atomic frames keyed on a page number and sub-code together, which the decoder follows, neoteletext-common's DEC-6599; an input session that discards its unacknowledged RULE_EVENTs and still resumes, `SessionOf::discard_rule_events`, which the trusted core calls when a connection ends; NAPLPS objects' labels, a picture's title and one object redefined in NAPLPS_OBJECTS, which the browser does not pass on yet; and a rule table's JSON read as the server reads it). Before it, `33d9089` (19 November 2026, the head of the engine's branch `rules-2`: the SPRITES and NAPLPS_OBJECTS bodies written alone, as a rule's group holds them, `neoteletext_web::records::sprites::write_body` and `naplps_objects::write_body`, which the browser does not call; the NAPLPS renderer unchanged; the ReGIS renderer at `9cbbe83`, the head of its branch `latency`, made from `30e12f4` of its branch `animation`, by a `git` source on its local repository: a picture's repaint is up to eight rectangles, one for each place apart that changed, so a key pressed at one side and a meter lit at the other repaint those two places alone, RULE-65). Before it, `b3edee3` (18 November 2026, the head of the engine's branch `rules-2`, through a `git` source on its local repository while unpublished: the second form of the page rule table, a rule running one group of protocol commands, every table, record and message keyed on a page number and sub-code together, push protocol 10, cue points, and each event handled at once with only time on the tick, neoteletext-common's DEC-6587, DEC-6599 and DEC-6635; the ReGIS renderer moves with it to `30e12f4` and the NAPLPS renderer to `dfd5e8c`, their branches `animation`, by `git` sources on their local repositories). Before it, `d61d292` (18 November 2026, the head of the engine's working branch, pushed: its wire vectors gain the worked examples' rule tables and the local sound's OBJECT as the server writes it, and no library code the browser uses changes; with it the engine and the ReGIS and NAPLPS renderers are taken from their public repositories again, the renderers at the same commits). Before it, `22e8f6e` (18 November 2026, the head of the engine's branch `rules-integrate`: the audio receiver reads the tracked values of a synthesizer sound played from blocks while its audio channel is watched, the feature `watch` of `neoteletext-audio`, `docs/page-rules.md` RULE-30; `neottx` sends its keys inside RULE_EVENT; the wire vectors of push protocol 9; and the tick, the sprite limits and the record types defined once across the engine's crates). Before it, `1e7ea05` (18 November 2026, the same branch: the page rule table of neoteletext-common's `docs/page-rules.md`, with the new crate `neoteletext-rules` that decodes, checks and evaluates a table; the push protocol at version 9, in which a viewer's input travels only inside a RULE_EVENT and the descriptor's `push.input` and `push.wheel` are withdrawn (RULE-46, RULE-49), with the events RULE_EVENT, PAGE_STATE and RULES_FAULT and the records 21 to 27; the local sound, OBJECT tag 9, with the mixer's pulled voice and per-voice peak (RULE-60, RULE-62); flash timed by the presentation clock's tick, 57 ticks shown and 19 hidden, in place of microseconds (`docs/animation.md` ANIM-1); and the engine's sprites, damage and animation state. The ReGIS renderer moves with it to `0920a09` and the NAPLPS renderer to `5a2464e`, their branches `animation`, each through a `git` source on its local repository while that commit is unpublished). Before it, `2c65d50` (18 November 2026: each playing of a sound streamed on its own audio channel, DEC-6477). Before it, `df51a96` (18 November 2026, the engine's branch `integrate-audio`: viewer state in place of cookies, neoteletext-common's DEC-6471, the push protocol at version 8 with the records STATE_CHALLENGE, STATE_END and STATE_KEEP_OFFER and the receiver's STATE_PROOF, STATE_NONE and STATE_OFFER, the engine's cookie jar, SET_COOKIE and its Device Bound Session Credentials removed; and the synthesizers' blocks carried as event lists under codecs 12 to 18 with their banks as codec 19, codes 3 to 11 retired, neoteletext-common's DEC-6465 and `docs/audio/midi-architecture.md`, MIDI-107; the receiver still renders a stored synthesizer sound whole and keys its lanes by sound, so `integration.md` INT-33 and INT-37 wait on the engine; and `neoteletext-audio` now links `neoteletext-audio-instruments` in every build, the trusted core's too, with no family compiled in there). Before it, `02adbf1`, on the branch `viewer-state` alone. Before it, `dd0421c` (18 November 2026: the synthesizers carry NeoTeletext's own names, synth6581 to synth80, in the engine's features, modules and types, neoteletext-common's DEC-6461, which instrument each models being said once in `neoteletext-common/docs/audio/synthesizers.md`; each analogue synthesizer takes its standard interface in place of the 20-byte patch, the engine's DEC-6453; and a channel that uses the mouse wheel over its pages says so in `push.wheel`, DEC-6447, which came with `5369fab` the same day). Before it, `7e09448` (17 November 2026: synth106 and synth60, synthD, synth53, synth8 and synth80 as instruments of the audio profile, codecs 6 to 10 with their patch bank as codec 11, the engine's DEC-6429, rendered in the decoder alone, after five rounds of review, its TODO-6438; with all seven instruments the engine's receiver is about 270 KB, so the decoder keeps it on the heap). Before it, `9d6faa4` (17 November 2026: the engine's main, taken from `https://github.com/neoteletext/neoteletext-renderer-teletext`, as the ReGIS and NAPLPS renderers are from their own public repositories; since `08276d2` only tests and gate steps changed). Before it, `08276d2` (16 November 2026: synth6581 and synth7 as instruments of the audio profile, the engine's DEC-6410, rendered in the decoder alone). Before it, `885817d` (4 November 2026: the engine's resolved colour mode draws the status area and a receiver-drawn row 24 in the receiver's own eight colours, indices 32 to 39 after the Colour Map's 32, the engine's DEC-2464, so a frame's palette holds up to 40 colours, `neoteletext_core::consts::RESOLVED_COLOURS`; the receiver's colours may be the user's, DEC-2482; a live sound's encoder keeps its state between blocks, `LiveEncoder`, DEC-2467, which the reference channel's podcast uses; a proxy's address and tunnel refused with typed errors, `ProxyParseError` and `TunnelError`; the glyph line cache behind a feature; and the engine's merged branches of that day). Before it, `e950dcb` (4 November 2026: the picture converter as a library, the engine's DEC-2325, which the browser's Tools menu uses; a refusal for its media type names that type, `Failure::MediaType`; Device Bound Session Credentials kept across runs; hexadecimal pages reachable under the diagnostic setting; the qrkeyboard client's readers fuzzed; and the engine's merged branches of that day). Before it, `4ddd34b` (3 November 2026: Thai and Lao dropped entirely, the engine's DEC-2239, so group 1100 is reserved again and no Thai or Lao set, glyph or page layout remains; Yoruba dropped, DEC-2240, and the set at 0101 110 named for Igbo, DEC-2244), through a `git` source on the engine's local repository, `file:///Users/andrewstuart/devel/neoteletext-renderer-teletext`, while that commit is unpublished, to move to the published repository at the same commit once it is pushed. Before it, `61ed70b` (3 November 2026: Japanese in rōmaji and in hiragana and katakana sets, group 1101, with their Noto Teletext glyphs; NeoTeletext's row set triplet, column mode 01010, which reads a row's own codes in another set from its column; one table from a language tag to its set; and the page writer's languages, levels and the standard's Arabic set of Table 44). Before it, `854015b` (2 November 2026: each cookie kept to the folder of the channel that set it, so none reaches a channel nested in that folder, and the jar's saved form at version 3; a channel's `initial_page` read as three decimal digits only). Before it, `5b39e3d` (2 November 2026: a page's sub-pages in a folder named for the page, `NNN/SSSS.t42` and `NNN/SSSS.tti`, with its list in parts of 64 rows, `NNN/P.sub`, neoteletext-common's DEC-2116; SELECT sent only when both sides agreed it; a coloured key live only while row 24 shows its prompt whole, `Grid::prompts_shown`, DEC-2115; the latest sub-page of the page shown never evicted; and the fixes of the engine's audit of that day). Before it, `ca0f1e8` (2 November 2026: Thai as the second set of `RE,C`). Before it, `3c3cf4e` (1 November 2026: the channel URL map, `neoteletext_web::map`, with its conformance table; a channel's live connection fixed at its folder's `push`; page parameters in SELECT at input protocol 5; neoteletext-common's REQ-1693). Before it, `75db08c` (30 October 2026: SELECT names a connection's one selection, with no view number, neoteletext-common's DEC-1527). Before it, `f393623` (30 October 2026: SELECT, every pushed packet checked and the invalid ones counted, a live channel opened by its connection alone, and its index, LOG, SET_COOKIE, LINKS and SUBPAGES records, neoteletext-common's DEC-1459 and DEC-1500). Before it, `a58958f`; before that, `8c59a89` (28 October 2026: every move over the page paced at 60 a second by `pace::Pacer`, touch numbered by `contacts::Contacts` with MOUSE's device and TOUCH, MOUSE action 3 gone, resumption of a lost session with `ResumableSession` and RESUME, the round trip measured by ping and pong, the messages of one moment written together and moves held per contact while the socket is congested, and pointer areas withdrawn, the engine's decision E.133 (decision 125). Before it, `e09a589`, 28 October 2026: every address World System Teletext defines accepted and every sub-page to the standard's 8,190 held, the numbers in `neoteletext_core::limits`, the engine's decision E.130 and its area NUM; PCM16 and IMA ADPCM each behind a feature, which the desktop builds leave out, with the `.ttau` file and WebSocket readers and fades at a lost block, E.131; and each sub-page a file of its own with each page's catalogue, `NNN.sub`, E.132 (decision 121). Before it, `dcb2a3b`, 27 October 2026: no input modes, every event sent, moves only while a button is held or within a pointer area a page declares, and HOVER over controls, the engine's decisions E.125, E.128 and E.129, built on its E.122, the standard's maxima defined once. Before it, `c0cdc6a`, 27 October 2026: a page's controls, read from reserved column mode 00101, and the hover and pressed looks drawn on them, the engine's decision E.127, built on its E.126, the TTI Form Extension withdrawn, and its E.123 and E.124, the integer Opus decoder carried as codec 2. Before it, `11149ef`, 26 October 2026: a found page shows the latest clock, decision E.121. Before it, `eb6b2d2`, 25 October 2026: a contents title may be a whole headline, 120 characters, decision E.120; and the wheel reaches a channel over its page in any mode, decision E.119. Before it, `c997514`, 24 October 2026: the engine's tests for its core's line coverage of 90 % (REQ-TST-041), which found and fixed an object placed outside every display area kept instead of discarded (REQ-OBJ-014) and a DRCS page known only by its X/28/3 kept as a text page (decision E.117); and `p256` moved to 0.14 for advisory RUSTSEC-2026-0387 in the `crypto-bigint` it brought in (decision E.118). Before it, `5311f98`, the same day: the engine's source reorganised by concept with every public path kept; its fixes from NeoTeletext's security review of that day, decisions E.112 to E.116: a 307 or 308 form redirect kept to its target's origin, a wider list of local names through a proxy, the web user agent's narrower choices for cookies, redirects, folders, event ids, close codes and IPv6 hosts, more local address ranges and embedded IPv4 addresses judged as themselves, and a qrkeyboard phone trusted less, reported connected only once a frame authenticates; and the mixer's buffer-taking forms, so a port allocates a sound outside its lock. Before it, `fb2dd8a`, 23 October 2026: E.106 to E.111, every feature set built again, the qrkeyboard waker, a proxy's answer read in chunks, the receiver's row 24 and links in the formats crate, channel log lines, Device Bound Session Credentials behind a feature, and fonts named by channels withdrawn), through a `git` source on the engine's published repository, `https://github.com/neoteletext/neoteletext-renderer-teletext`, since 28 October 2026 (before it, the engine's local repository, while its commits were unpublished); Cargo reads the commit, never a working tree, so a pin moves only to a commit already pushed there. Changes the engine needs are made in the engine's repository by its own process, never patched here.

**The NAPLPS renderer** (`~/devel/neoteletext-renderer-naplps`, its crates `naplps` and `naplps-fonts`) is pinned in the native and fuzz workspaces at `9122afd` (19 November 2026, the head of its branch `parity-2`, through a `git` source on its local repository while unpublished: `Terminal::compose` places objects by the design picture's pixels on every canvas size, with `Rect::on_canvas`, `naplps::design_pixel` and `Terminal::objects_on_canvas`, its TODO-6814; the decoder gives it objects unscaled and picks by the rectangles on the canvas). Before it, `19f5bba` (19 November 2026, the engine's own pin at `4595844` and `8b973b7`, so that one copy resolves). Before it, `2700040` (19 November 2026, the head of its branch `parity-2`, through a `git` source on its local repository while unpublished: renderer parity's joining of the plain Arabic-script sets, `naplps::Validator` and `naplps::limits::MAX_STROKE_FONT_BYTES`). Before it, `b435b7d` (19 November 2026, the head of its branch `charsets`, merged into its working branch and pushed, so taken from its public repository: the 55 teletext character sets taken as data, `Terminal::screen_colour`, objects' labels, a picture's title and one object redefined). Before it, `dfd5e8c`.

| Crate | What it does | Used by NeoTeletext |
|---|---|---|
| `neoteletext-core` | Packet decoding, page store, Presentation Levels 1.0 to 3.5, rendering a page to palette indices, flash timing | In the decoder; its flash timing constants in the trusted core; its page addresses in the fetcher. Built with the engine's recommended `modern-classic` profile and its `level35-32colors` feature (decision 60) (its decision E.71) |
| `neoteletext-formats` | Readers for T42 recordings and Teletext Text Interchange page files; content detection | In the decoder, with the `tti` feature: the browser opens these two and no other (DEC-1762), so the engine's other readers are not built |
| `neoteletext-image` | Palette indices to colour formats; the Portable Network Graphics writer | The palette in the decoder; the writer no longer (decision 61) |
| `neoteletext-web` | The strict address parser, origins, the HyperText Transfer Protocol version 1.1, WebSocket, the cookie store, channel descriptors, contents files, push subscriptions, and the user agent that opens and fetches a channel, with no network input or output of its own | In the fetcher, with the `push` and `cookies` features; in the trusted core, its cookie jar, proxy-address parser and address type; in the phone keyboard; and in the updater, pinned in `native/tuf/Cargo.toml` |
| `neoteletext-audio` | The audio profile's receiver (its area AUD) | In the decoder (decision 37) |
| `neoteletext-audio-mix` | The audio mixer | In the sound helper (decision 55) |
| `neoteletext-input` | Key, cursor and control records and input sessions (its area INT) | In the trusted core, the decoder, the interface core and the interface (decisions 44 and 48) |
| `neoteletext-qrkeyboard` | qrkeyboard's client: pairing, encryption and QR codes | In the phone keyboard (decision 44) |
| `neoteletext-transport` | A client's reader thread, TLS pump, handshake and TLS configuration, shared with the engine's own programs (its DEC-1905) | In the fetcher, and through `neoteletext-qrkeyboard` in the phone keyboard. Its opening under one deadline and its writer thread are the fetcher's own (the engine's DEC-1906) |
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
| **Decoder** | One channel's pages; and, for the one page of it that is the open page of the active tab, that page's rule table, its rule state (variables, timers, which rules are on) and the viewer's input events its table asked for, as what the evaluator matches and never as their bytes (neoteletext-common's `docs/page-rules.md` RULE-58) | Nothing it needs; see section 6.5 for what the sandbox still allows | Pictures of its choosing in that channel's page images, which its server could draw anyway; and the viewer's own input on that channel's open page sent to that channel, within the trusted core's budgets, which the channel's table could have asked for (RULE-52) |
| **Sound helper** | The sound now playing; and the instruments of the open page's local sounds, with their bank images and the MIDI messages waiting for them (RULE-60, RULE-62) | The audio output alone | Sound of its choosing, a false report of which tabs sound, and false levels for the views that watch them, at most 32 bytes a tick for each (RULE-33; `security-analysis.md` section 5.10, threat 8) |
| **Interface process** | What is on screen | The window and input; see section 6.5 | Everything on screen, including a false trusted strip, and the ability to send intents and **strip input**: it can navigate a tab to any address the trusted core's rules accept, go back and forward, and set bookmarks. No secrets, because none are sent to it |

**The trusted strip and the interface process.** The strip is drawn by the interface process, in its own window, outside the page area (design section 5.2, option 1). That keeps the strip out of reach of every page image and every other process's window. It does not keep it out of reach of a fault in the interface process itself: a compromised interface process can draw a false strip and send strip input. Reaching it needs a fault in NeoTeletext's own interface code or its drawing libraries, fed only by our own images and cleaned text (`security-analysis.md` section 5.6). Requirement 3.9's hostile interface harness tests every other property with an attacker's interface in its place.

### 6.2 Why bytes pass through the trusted core

The fetcher never talks to a decoder directly. The trusted core passes every page the fetcher delivers to the decoder of that fetcher's channel, because only the trusted core knows which tab shows which channel. It never parses those bytes: it cuts them into pieces within the message limit and passes them on. File channels are the same, but for their pace: a file the viewer opens is passed on at once (DEC-1762). The broker reads the file for the trusted core, which passes its bytes, unread, in pieces within the message limit, as many as may be queued for the decoder, half of `outbox::MAX_QUEUED_BYTES`; what does not fit follows at the next `LOAD_INTERVAL`, 0.2 seconds on. A new decoder is given the file from its start, so a file is not kept twice.

### 6.3 Decoders and fetchers per tab

- **Every channel has its own decoder,** and every web channel its own fetcher, so no two channels share a process that reads their data.
- **Every tab is a channel of its own** (decision 11 as amended, DEC-910 version 3; neoteletext-common's DEC-1527): `Session::channel_for` gives each tab, and each navigation to another channel, a channel number of its own, even when another tab shows the same channel. So each tab of a live channel has its own push connection, fetcher, decoder, page store and kept sleep data, and its own one selection, told on its own connection with SELECT. The server keeps one selection and one baseline of changed rows per connection, so they apply to the store that received the version they were taken against. Nothing of this is shared between tabs, and no sharing is planned.
- **The cookie jar alone is per channel:** every tab of a channel is answered from the channel's one jar (decision 62, REQ-880), so a cookie set through one tab's connection is sent by the others; a private tab has a jar of its own (REQ-881).
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
| RulesPage | The page and sub-code of the channel that is the open page of the active tab, told as its sub-page is drawn, or none, the pause of RULE-111 case 4: only its rule table is evaluated, and only it plays a local sound (`page-rules.md` RULE-58, RULE-112) |
| ConnectionEnded | The view's push connection ended: every local state of the page is dropped at once, with no `close` tick (RULE-111 case 2) |
| CueHeard | A cue the sound helper heard on one of the view's audio channels, for the open page's `cue` rules (RULE-109) |
| FramePhase | The interface's refresh period and the time from the message's writing to its next composition, which each receiver adds to its own clock as it reads it (DEC-6659 item 12): the decoder locks its presentation clock to it and paints just before each composition (RULE-98, RULE-65). The trusted core tells it every decoder, again as a decoder starts and as a page opens, never kept for a sleeping one |
| RuleInput | One input event of the viewer for the open page and sub-code, of a class its table asked for: what the evaluator matches (a key's identity and modifiers, the pointer's pixel, button and control, the wheel, a focus change, a ReGIS drawing entered or left) under the sequence number a report names it by. The event's bytes stay in the trusted core (RULE-52 items 1 and 2) |
| Metered | The sound helper's newest report for the open page's watch list, one byte for each slot, passed only at that list's length (RULE-33 item 4) |
| RulesQuery | A state query the channel sent for the open page (RULE-47) |

**From a decoder to the trusted core:**

| Message | Meaning |
|---|---|
| Confined | Its first message, before it reads anything: it is confined and running; one that ends before saying it fails an update's trial (finding F-12) |
| AudioReceiver | Its second message, once its audio receiver is built: what the receiver holds, three figures shown in the channel's Log tab (the engine's REQ-AUD-017). The browser builds every receiver with a 4,194,304-byte object store, 256 objects and 8 records reassembled at once (`AUDIO_ARENA`, `AUDIO_OBJECTS` and `AUDIO_RECORDS` in `native/decoder/src/engine.rs`), at least audio Level B's minimum (`neoteletext_audio::LEVEL_B_MINIMUM`), which a compile-time assertion checks |
| Arrived | A page was received completely for the first time: its number and a title from its first text row, or its NAPLPS picture's title when it has one |
| PictureTitle | A page already reported as arrived has a NAPLPS picture's title it did not arrive with, or a new one: its number and the title, untrusted, which the trusted core cleans and gives the page in place of its rows' title, never in place of the program's index's (DEC-6638 item 3, TODO-6748) |
| Updated | A page already held arrived again with different rows |
| Frame | A rendered page: 480 by 500 palette indices and a palette of at most 32 colours, every index checked against the palette on arrival |
| Audio | One voice operation of the channel's audio receiver: define a sound, its 16-bit samples (at most 32 768 a message), forget it, announce a live sound and give its next samples, or start, stop or ramp one of four channels; and for a local sound (RULE-63) one of its bank images (at most 16,384 bytes), its start (family, variant, option, split, gain, fade, duck and a setup block of at most 1,005 bytes) or one MIDI message of a rule's `midi` action; every field bounded (requirement 16.4), the bank image and the setup block passed on unread, as samples are |
| Links | The links found in a page, before its image when they change: at most 32 runs (page numbers in its text and its `LT` lines, each a place and either a page number or a target as written, at most 256 bytes, never cut) and its `LK` lines; and whether an image follows (`hyperlinks.md` section 8.1) |
| StoreFull | The page store is full and the oldest pages are being forgotten |
| SubPages | The sub-pages of a page on screen and the one drawn, whenever they change (decision 67) |
| Cursor | A cursor record the channel sent for a page, untrusted, for the trusted core to check (DOC-1954) |
| InputControl | An input control record (WELCOME; the withdrawn mode records are ignored), untrusted (REQ-INT-111) |
| ChannelLog | A log line the channel wrote (an `LG` line), untrusted, for the console's Log alone (decision 70) |
| Bookmark, BookmarksRead | A folder, its end or a bookmark found in another browser's bookmark file, and how many were left out, from a decoder started with `--bookmarks` (decision 71) |
| RulesOpen | What the open page's table asks for: its generation (0 for no table), its interest mask (keys, text, pointer buttons, pointer moves, the wheel, focus changes), its watch list of at most 32 metered values, and at most 16 rectangles of the picture inside which an enabled `wheel` rule applies (RULE-52 item 1, RULE-33 item 1, RULE-51 item 4). Untrusted: taken only for the open page |
| Report | A request to report a group's `REPORT`: the page and sub-code, the table's generation, the rule and its row (255 for none), the tick, the target, and either an input event's sequence number, a value and its reading, or nothing. It never carries an event's bytes: the trusted core builds the `RULE_EVENT` from its own copy (RULE-52 item 3) |
| PageState | The answer to a state query, at most 1,800 bytes, passed once, only in answer to a query the fetcher received (RULE-52 item 6) |
| RulesFault | A fault of the page's table, by its code and count (RULE-48), passed to the channel at most once a second for each code |
| RuleCounters | The open page's counts (RULE-72), once a second: ticks run and late with the longest, frames finished after their composition (RULE-98 item 5), rules fired, groups run, commands applied and skipped, events handled, waited and dropped, sequence starts late and skipped, cues heard, reports asked for, and the sixteen variables, for the developer console's Stats tab, its "Page cost" section (REQ-6626) |

**From a fetcher to the trusted core** (`protocol::fetcher`):

| Message | Meaning |
|---|---|
| Confined | Its first message, before it opens its channel: as a decoder's |
| Contents | The channel's contents file as the engine read it: section headings and page entries with titles, at most 1,000 entries |
| Page | A page's bytes, fetched or pushed, passed to the decoder unread |
| Denied | A selection the channel refused: its status and place (DOC-1961), checked by the trusted core, whose words the page area shows |
| Notice | A short report, such as an unreachable address, cleaned and shown in the trusted core's log |
| Links | The descriptor's aliases (at most 16) and named services (at most 16), as written, for the trusted core to parse |
| Pushing | The push connection opened or was lost (decision 47) |
| Exchange | One exchange the agent made, for the developer console (requirement 19.9) |
| ProxyFailed | The proxy refused, failed or did not answer; it carries nothing (requirement 19.10) |
| Input | The push WebSocket of a channel whose push address may take messages opened or closed, and whether it is `wss`. The descriptor's `push.input` and `push.wheel` are withdrawn: a page's rule table is the only thing that asks for input (RULE-49) |
| RulesQuery | A `RULES_QUERY` record the channel pushed: its page and identifier (RULE-47), so that the trusted core passes a page's state only in answer to one |
| CookieQuestion, ReachedQuestion | Asks for a request's `Cookie` header, or whether a host was reached securely with a live cookie (decision 62) |
| CookieSet | A `Set-Cookie` value a response carried, as typed fields the fetcher read (DEC-1765) |
| ClearSiteData | A host's encrypted response said `Clear-Site-Data: "cookies"` |

**From the trusted core to a fetcher** (`protocol::fetcher`):

| Message | Meaning |
|---|---|
| Send | One message of the input stream for the channel's push WebSocket (requirement 4.7): the session's own (HELLO, RESIZE, SELECT, RESUME, viewer state's) or a `RULE_EVENT`, `PAGE_STATE` or `RULES_FAULT`. Under push protocol 9 a viewer's input goes only inside a `RULE_EVENT`, and the protocol crate refuses to write one outside it (RULE-46) |
| CookieAnswer, ReachedAnswer | The answers to the fetcher's two questions |

**Between the trusted core and the sound helper** (`protocol::sound`), **the phone keyboard** (`protocol::keyboard`) **and the updater** (`update::wire`):

| Direction | Message | Meaning |
|---|---|---|
| Core to sound helper | Audio, Settings, Close | A checked voice operation of a channel, a local sound's among them, and, from the trusted core alone, a view's watch list (operation 15, `Watch`: at most 32 slots of an audio channel, a kind and a part); page sounds and their volume; stop |
| Sound helper to core | Sounding, Metered, LocalOverrun, CueHeard, Spent | A cue of a view's sound heard, after the device buffer that holds its sample (RULE-109); the time spent rendering a view's sounds, for its "Page cost" (REQ-6626); a local overrun's count, 0 when the small device buffer was given up for the usual one (RULE-97); the channels sounding now; the values of one view's watch list, one byte for each slot, once half a tick of frames was rendered, which is after every device buffer of 10 ms (RULE-33 item 3, RULE-65, DEC-6573); a view's local sounds stopped for overrunning their time (RULE-62) |
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
| Interface to core | FramePhase | The display's refresh period and the time to the next composition (RULE-98 item 1), sent where the platform gives the interface frame times and told again at least every half second. winit gives none on any platform, so the shipped interface sends none and decoders paint at once (RULE-65's case without one); the timed test's own build gives the interface a display composing at 60 hertz that sends it |
| Interface to core | History, bookmarks, star, bar, suggest, suggestion, tab list, go to, new private tab, window | The panels' requests, the address field's text and a chosen suggestion, a Back or Forward list and a step in it, a private tab, the window's place and size |
| Interface to core | Cookies, page control | Strip input from the cookies tab; a key of the open page's controls |
| Interface to core | From, new window, tab to window, close window | Which window the intents that follow are from; open a window, move a tab to another or a new one, close the window (decision 72) |

#### 6.4.1 A page's rule table between the processes

A page's rule table (neoteletext-common's `docs/page-rules.md`, second form: DEC-6522, DEC-6587, DEC-6599, DEC-6635) binds the viewer's input, timers, sequences, cues and conditions to groups of protocol commands. It is untrusted, so the processes divide it as follows.

- **The decoder evaluates** (RULE-58; `decoder/src/rules/host.rs`, `decoder/src/engine/rules.rs`). It holds one table, that of the view's selected page and sub-code (a page is a page number and a sub-code together, RULE-112): it assembles the RULES record's fragments, decodes the image with the engine's `neoteletext-rules` and `neoteletext_web`'s body check (a table is accepted whole or refused whole and reported as fault 1), and runs the evaluator while the trusted core names that page and sub-code with RulesPage. Every event is handed to `Evaluator::handle` the moment it is read, before a record, a repaint or a tick (RULE-96 as DEC-6635 revises it): an input from the trusted core, the server's VARIABLE, TIMER, REPORT, RUN and SEQUENCE records and RULES_QUERY, a cue the sound helper heard; and what the handling produced, a group's sound first, then its reports and a state answer, is written to the trusted core and flushed at once. Only time runs on the presentation clock's ticks (`Evaluator::tick`: `open`, timers, sequences, `becomes`, frame cycling). Each command of a group is applied by the code that applies the same record from the server (RULE-118): CONTROL by the engine's audio receiver, LOCAL_MIDI to the local sound and its tracker, ANIMATE, SPRITES and NAPLPS_OBJECTS by the animation, which changes the picture's state at once for the next draw. A page ends or rests in the four ways of RULE-111: another page or sub-code opened closes it (its `close` handling runs and reports, then its table, its local sounds and its picture's local state are dropped); the view's connection ending drops it all with no `close` (ConnectionEnded); an empty RULES record drops the table; and a tab not the active one is paused, its state kept, no tick run and no input taken, going on from the next tick without `open`. Ticks that take longer than their own time are counted and told as fault 9. A NAPLPS page's pointer is given to the evaluator in the picture's own 640 pixels.
- **The trusted core gates** (RULE-52; `core/src/typing/gate.rs`). It reads no table. It names the open page and sub-code to its decoder as the sub-page is drawn, and gives that decoder input only while it has said the page has a table and only of a class in its interest mask, keeping a copy as encoded, the last 64. A report names a sequence number, the rule and its row; the `RULE_EVENT` is built from the kept copy, only for an event given for that page and sub-code within 4 ticks and not yet reported for that rule and row, on the session of that page's own channel, within 120 reports a second of which 20 with no input event. It passes at most 512 sound commands a tick from a view's decoder to the sound helper (`hearing.rs`, fault 10). When a connection ends it forgets the gate, tells the decoder, and does not resume a session that gave a table input, so that no unacknowledged `RULE_EVENT` is sent again (RULE-46). A page with no table is given nothing and sent nothing (RULE-50, REQ-6523). For 4 ticks after a close its decoder's `close` handling may still report.
- **What stays the browser's own** (RULE-51): the wheel is the page's only while the pointer is inside a rectangle the decoder said a `wheel` rule applies in.
- **The sound helper plays local sounds** (RULE-60, RULE-62, RULE-97; `sound/src/audio/local.rs`, `sound/src/audio/buffer.rs`). A local sound's instrument is held in the sound helper and rendered inside the audio callback in blocks of 128; a MIDI message waits at most one block. While a local sound is open the output device runs with a buffer of 256 frames in place of 480, given up for the usual size at the first underrun or overrun (fault 7, count 0) and kept two seconds after the last ends. `Pause` silences a paused view's local sounds with their notes released and forgets its cue marks. The helper keeps each cue mark the decoder sends (`Cue`) in the engine mixer and tells `CueHeard` after the buffer that holds its sample (RULE-109).
- **Metering** (RULE-32, RULE-33): the decoder tells the watch list with RulesOpen, the trusted core passes it to the sound helper for the open page alone, the helper reports the watched values once half a tick's worth of frames was rendered, and the trusted core gives a report to that view's decoder only at the length of the list it passed; the decoder applies it between handlings.

Measured by `core/tests/rules_latency.rs` on 18 November 2026 between threads of one process, 200 keys, a device of 480 frames every 10 ms: with each event held for the tick (the test's own switch, `NEOTELETEXT_TEST_EVENTS_AT_THE_TICK`, as before DEC-6580) a median of 14.7 ms and at most 26.5 ms from the key to the first sample, and with each handled on arrival a median of 5.2 to 5.4 ms and at most 10.1 to 14.7 ms (RULE-64 asks, for 480 frames, 9 and 17); from the key to the level rule's report a median of 23.8 ms held for the tick and 10.3 to 11.5 ms on arrival. The figures between real processes that RULE-66 asks for are taken by `scripts/timed-test.sh` (the ignored test `core/tests/timed_between_processes.rs` runs it): it builds the browser with the feature `timed-test`, whose marks each process writes on the monotonic clock and whose setting `NEOTELETEXT_TIMED_TEST` switches one change off (events held for the tick, the device buffer left at 480 frames, the tick unlocked with paint points at the ticks), and which gives the interface a display composing at 60 hertz on a private Xvfb display, where no platform frame times exist; it runs the protocol 15 server, the music player's page 101 and a private PipeWire with a null sink, presses 200 keys through XTEST, and prints the six sets of figures. Measured on 19 November 2026 (chunko, the commit 6f80008's message and REQ-6535 hold every figure): with every change on, the key's LocalMidi left the decoder in a median of 0.3 to 0.4 ms and at most 3.1 to 4.3 (RULE-64 asks 2); the first sample was handed to a device of 256 frames in a median of 3.5 to 3.9 ms and at most 9.2 to 10.4 (12 and 6 asked), and of 480 frames in 6.5 and 12.9 (17 and 9), against 14.7 and 27.9 with each event held for the tick; the frame showing the key's changed rectangle came in a median of 14.5 to 17.7 ms and at most 53 to 56 (RULE-65 asks 12.8 and 22.7), against a median of 21.1 with the tick unlocked; FramePhase's error through the pipes was a median of 0.2 ms and at most 2.4 to 5.6. The picture's bounds and LocalMidi's greatest are not met yet; the meter's figures are not yet sound (the previous note's meter still falls when the next key comes). The path of a key to its sound is then: the interface's intent, the trusted core's gate, the decoder's handling of it the moment it is read, `LocalMidi` to the trusted core and on to the sound helper, the next block, the device's buffer. Measured by `core/tests/rules_latency.rs` on 18 November 2026, in threads of one process, over 200 keys: a median of 12 to 15 ms and at most 27 ms from the key to the first sample handed to a device of 10 ms buffers (RULE-64 asks for 20 and 34), and a median of 24 ms from the key to the tick in which a rule on the metered level fires (RULE-65 asks for 30), nineteen in twenty within 33 ms and the greatest 34 and 43 ms in two runs (45); before the helper reported after every buffer the greatest was 50 ms. Measured the same day with the real, confined processes on a private display (Xvfb, a private PipeWire with a null sink and a quantum of 480 frames, the protocol 14 server and the music player's page 101, 100 keys injected through XTEST at random phases, the machine under a load of 4 to 6): from the key to the note on handed to the sound helper a median of 11.1 ms and at most 17.3 ms, to which the device's buffer of 10 ms is added for the first sample; from the key to the pressed key's pixels in the X server's framebuffer a median of 14.4 ms and at most 20.8 ms (RULE-65: one tick and 4 ms, 20.7 ms, from the interface reading the key); and from the key to the meter's lowest segment on the screen a median of 31.0 ms and at most 39.5 ms (RULE-65: 30 and 45). The server counted no input from the page.

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
| **Channel descriptor** (`channel.ttx`) | The engine's descriptor reader | Fetcher | Its aliases and named services, as written, for the trusted core to parse (`Links`, `hyperlinks.md` section 6.4); the fetcher uses the rest to find the contents file and the push feed |
| **Contents file** (`contents.txt`) | The engine's contents reader | Fetcher | A bounded, typed list of sections and pages |
| **Discovery** (HyperText Markup Language) | The engine's discovery reader | Fetcher, not built | — |

Discovery's design is `discovery.md` and its security analysis `discovery-security.md` (3 November 2026), which take over the engine's `docs/web-discovery-security.md`, in its git history since 18 October 2026. Its results are never loaded directly: each is shown to the user in the strip, and a chosen one is a page-initiated navigation (REQ-694). It is not built: DEC-1722 item 1 leaves it no address to run on until the owner answers DEC-2374, and the engine has no link-pointer reader yet (the engine's TODO-2376).

### 7.2 Transports

- **The HyperText Transfer Protocol version 1.1**, plain or with Transport Layer Security (`rustls` with Mozilla's root certificates, decision 41; `fetcher/src/transport.rs`). The trusted strip shows "Not secure" for a channel reached without Transport Layer Security (decision 10).
- **WebSocket** carries the channel's push feed: the fetcher subscribes to all of a channel's pages and passes each pushed page to its decoder; on a channel whose descriptor says it takes input, the WebSocket also carries the viewer's input (`interaction.md`, decision 45; `core/tests/latency.rs`).
- **Not supported:** the push mechanism of the HyperText Transfer Protocol version 2, which traditional browsers have removed.

### 7.3 Rules that apply to every request

These are the engine's user agent's rules, which the fetcher uses as they are:

- **Addresses on this computer and the local network are refused** unless the user names a specific address, as the engine's requirement REQ-SEC-006 requires. The user names it in the settings panel (requirement 19.12, decision 51) or with `--allow-local` on the command line.
- **Compression is undone with a fixed limit** on the decompressed size.
- **The declared media type is ignored** for acceptance (decision 13).
- **Every response has limits** on size, time and redirects, all the engine's constants.
- **No `Referer` header is sent,** and only the engine's minimal user agent profile.

### 7.4 Fetching what is wanted

Decision 22, under which fetching never depended on viewing, was **rescinded on 25 October 2026 by decision 104** (DEC-997): pages are fetched while they are visible or have focus, so a channel's server learns which of its pages are on screen. Requirements 5.15 (REQ-703) and 5.16 (REQ-704) state the design; DEC-2141 and DEC-2356 record how the browser builds it.

| What the channel publishes | What is fetched, and when |
|---|---|
| `channel.ttx` | First, when the channel opens. Its `name` goes to the trusted core as `FromFetcher::ChannelName`, untrusted, and titles the tab (REQ-6641); none is sent for a channel without a `channel.ttx`. A live channel, whose `channel.ttx` names a push connection, then opens that connection and fetches nothing more: its pages, its listing (the index records) and its lists of sub-pages come on it (DEC-1467) |
| A bundle, `index.t42` | Next, when a static channel opens, by the engine's user agent. The fetcher says `FromFetcher::BundleSettled` once the bundle's pages have gone to the decoder, or it was refused, or it is missing |
| A page's own file, `NNN.tti` or `NNN.t42` | Once the bundle has settled, when the page is wanted and has not arrived: the page a tab shows (the page open, or its address's, or the channel's first), each page opened after, and the pages the interface shows of the active tab's channel, the wall's thumbnails among them. Each is asked once until it arrives or is refused (`ToFetcher::FetchPage`) |
| A contents file, `contents.txt` | Once, when the channel's listing is first wanted: when its tab is in front of a window, after its page, since the page list, the contents view and the overview show it (`ToFetcher::FetchContents`). A page it lists is never fetched for being listed |
| A page's folder of sub-pages and its list in parts | As `page-controls.md` and requirement 4.7's third exception say |
| A `refresh` in `channel.ttx`, and no push connection | The page on screen in every window again at that interval, conditionally (REQ-704): the trusted core's timer, per channel, for each tab in front of a window that shows a page |
| A file on this computer | All of it, in its own order, at once, whatever is on screen (DEC-1762) |

- **Requests follow the trusted core's own state.** The trusted core asks a fetcher for a page, a sub-page, a part of a list or the listing from what it holds: the page the active tab's address names, the pages the interface says it shows of the active tab's channel, each checked against the protocol's bounds and asked once. Nothing the interface reports reaches another tab's channel (requirement 4.7, DEC-2356).
- **The agent keeps the rate.** Every request goes through the engine's user agent, which spaces them by its request interval and holds a bounded queue.
- **Closing a tab** stops its fetcher at once.

---

## 8. Content detection

A response in the teletext data role is recognised by its content alone, by the engine's `AutoReader` and `neoteletext-formats` detection (the engine's requirement REQ-INP-005):

1. Signatures are tried first: Teletext Text Interchange files and the other signed formats.
2. Then the headerless binary layouts, such as T42 packets, are scored; a layout is accepted only when most of its packet addresses decode correctly under the standard's Hamming 8/4 protection.
3. Detection waits until it has enough bytes, so its decision does not depend on how the data was split.

**Rules of this architecture:**

- **Detection runs in the decoder,** the process with no rights. The broker runs it too on a file, to read only teletext for the trusted core (decision 88). Both know T42 and TTI alone: the engine's other readers are not built (DEC-1762).
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
- **Navigations a page starts:** Fastext keys, page links, redirects, discovery results. Fastext keys and page links are built, decided by the trusted core (decisions 43 and 67); a redirect that leaves the channel shows its notice for a second and is then followed (REQ-701, REQ-1686); discovery results are not built.
- **A link or Fastext key that leaves the channel** opens the destination at once, in the same tab, with Back to return, as in a web browser (REQ-1670, decision DEC-1664). Only a `web:` link, which hands an address to another program, asks first, in the trusted strip.
- **The address shown is always the committed address.**

### 9.6 Addresses

The trusted core's address rules (`core/src/address.rs`) decide what may be opened and how the strip shows it:

- `https` and `http` web addresses, with `https` assumed when no scheme is typed;
- plain hosts only, letters, digits and hyphens: an internationalised name is opened and shown only in its ASCII `xn--` form, which no other script can imitate, and a host typed in Unicode is refused with a sentence saying how to type it (requirement 11.3, DEC-2340);
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

1. **The decoder renders a frame:** 480 by 500 palette indices (40 columns of 12 dots, 25 rows of 20 lines), a palette of at most 32 colours, the page's Colour Map with its redefinitions (decisions 6 and 60), and its screen colour, in the engine's resolved-colour mode at the engine's default presentation level, its text in the engine's compiled Noto Teletext font. Row 0, mosaics and the G3 characters are the engine's own. It renders only the pages the trusted core says are shown.
2. **The trusted core checks the frame:** a known channel and page, the exact raster size, a palette of at most 32 entries, every index within the palette. Frames for pages the interface does not show are dropped.
3. **The trusted core encodes the image** in `protocol::frame`'s smallest form: raw, in runs, or as changes against the last frame it sent for that page, which the interface holds (decision 61); a frame identical to that one is not sent. A page's frames are sent at most once a tick of the presentation clock, 16,666 microseconds, on each of three clocks (`frames::MIN_FRAME_INTERVAL`, the engine's `presentation_tick_due_us`; `docs/animation.md` ANIM-1; finding F-11; decision 122; one field of 20 ms until 18 November 2026): its content; the look under the pointer, a frame that changes only cells whose look was asked for; and the answer, the first frame of content after the viewer's input to the page, so that neither the look's frame nor the header's clock holds the channel's answer (requirement 18.12). Nothing a decoder draws changes faster: it composes animation, paints a rule's change and answers input once a tick, and flash is timed in ticks. A frame that comes sooner than its clock allows is held, the newest replacing any held before, and sent when its clock is up, encoded against the last sent; so a decoder alternating two images, even with a look and an input every millisecond, gets at most three frames of a page a tick to the interface (`a_page_s_frames_are_sent_at_most_once_a_field`, `inputs_and_looks_keep_a_page_to_its_clocks`).
4. **The trusted core sends the image to the interface process,** with the page's key and a frame version.
5. **The interface process reads it** with the same module's strict reader, against the frame of the page it holds: at most 32 colours, the screen colour and every dot within them, the dots filling 480 by 500 exactly. It keeps the page's image until a newer frame replaces it whole (decision 8), and draws the page scaled into its rectangle with nearest-neighbour sampling, inside a border of its screen colour one character cell wide (decision 60). A frame of the page keys go to, or of the page under the pointer, is drawn as it arrives; other changes at most every 16 ms while keys go to a channel and every 100 ms otherwise, counted from the last frame that changed the window (requirement 18.11, decision 122).
6. **Flash** is rendered by the decoder at the engine's timing, by its own clock: when a shown page has flashing cells it wakes at the next tick of the presentation clock, 60 a second, at which a rate a shown page uses changes phase, as the engine's `next_flash_tick` finds it (the Level 1 flash 57 ticks shown and 19 hidden, the units of the engine's decision E.48 five ticks each, `docs/animation.md` ANIM-1; a page with the Level 1 flash alone is drawn twice in its cycle of 76 ticks, not at every unit) and draws the page again if its phase changed, and step 3 passes on only the frames that changed (decision 47). The decoder also wakes when its audio receiver next needs the time (the engine's decision E.98); otherwise it sleeps until the trusted core writes to it.

**Animation: the decoder is the compositor (REQ-6536; neoteletext-common's `docs/animation.md`, ANIM-22).** Pages animate in the receiver, on every kind of page, and all of it is done in the decoder; the trusted core and the interface gain no knowledge of objects and go on checking, passing and drawing rectangles of pictures. In `decoder/src/animation.rs` and its modules, `decoder/src/engine/animation.rs`, `decoder/src/regis.rs` and `decoder/src/naplps.rs`:
- **The clock (ANIM-1).** One presentation clock of 60 ticks a second, the engine's `presentation_tick_at_us` of the decoder's own clock. Flash phases are the engine's by tick (`decoder/src/flash.rs` uses its `FlashRates` and `next_flash_tick`). The loop wakes only for the next tick at which something is due: a flash phase, a cycle's next frame, a drawing still painting, a WAIT's end, a blink's change, an open frame's limit, or the open page's rule table; otherwise it sleeps until it is written to (decision 47). At each wake the open page's rules run first, then `Engine::animation_step` brings the pages on screen to the tick, then pages are drawn.
- **Records.** `ANIMATE` (23), `SPRITES` (24), `NAPLPS_OBJECTS` (25), `SHEET` (26) and `FRAME` (27) are read with `neoteletext-web`'s readers from a channel's pushed messages (`animation::frames::Taken`).
- **Atomic frames and the page swap (ANIM-2, ANIM-3).** Between a `FRAME` record's begin and its end for a page, the page's teletext packets and its `REGIS`, `NAPLPS`, `REGIS_CURSOR`, `ANIMATE`, `SPRITES`, `NAPLPS_OBJECTS` and `SHEET` records are held as bytes (`animation::frames::Frames`, bounded by the engine's `OpenFrames`: 262,144 bytes, 120 ticks, 4 pages), and at the end applied in order before the page is drawn again, so its rows and its picture appear together and a new stream generation is never shown cleared or half drawn. A frame also ends, and what it held is applied, at a second begin, at either limit, when its page stops being shown and when the connection ends. `LINKS`, `DRAWING_LINKS` and `RULES` records are applied as they arrive.
- **The animation state (ANIM-6).** For each sub-page, at most 64 ReGIS drawings or NAPLPS objects (the engine's `AnimationStore`), or 32 teletext sprites and controls (the engine core's `sprites::Animation`), hold a state: shown, position, layer and order, frame, cycle and forced look. A rule's action and an `ANIMATE` record write the same state; a ReGIS commit that writes the same thing is the last writer (the decoder reads `Terminal::animation` back after each commit); new sprites, objects or a new stream generation forget it.
- **ReGIS (ANIM-9, ANIM-14, ANIM-15, ANIM-17).** The state is given to `regis::Terminal::set_animation`; the decoder holds the cycles and sets the frame at each tick one is due; `advance_ticks` is told the ticks that pass, which a drawing with a paint rate earns work by; a page's sheet is fed to the renderer of each of its sub-pages before that sub-page's stream.
- **NAPLPS (ANIM-11, ANIM-20).** The renderer's `Player` plays WAIT and BLINK on the clock under its budget of one minute a generation. The stream's picture is the background; the sub-page's objects are composed over it into the picture presented, only damage repainted. The objects that are shown and detectable are sent as the picture's pick record, so the trusted core picks a NAPLPS region as it picks a ReGIS drawing.
- **NAPLPS on the teletext raster (the NAPLPS renderer's REQ-5819, DEC-6667).** The decoder's canvas is 480 by 462 pixels, one to a dot of the raster, and the picture it sends is the raster itself, 480 by 500: the canvas on lines 19 to 480 and the 19 lines above and below in the colour the page last cleared its display to (`Terminal::screen_colour`). A picture's size follows its language (`interface_core::canvas::PictureLanguage`), which every reader of a picture, part or pick record checks it against. The interface lays a NAPLPS picture over the whole page image as the teletext page it stands for, whatever the fit, so its page keeps the teletext page's shape and actual size, and a pointer's pixel of it is a dot of the raster, in the teletext page's cell. Objects and a rule table's pointer and rectangles stay in the 640 by 480 picture of ANIM-11 and RULE-13: the decoder gives the renderer each object scaled to the canvas, reads rectangles back in that picture's pixels, and maps a pointer's dot to it (`naplps::design_point`). Every terminal is given the engine's 55 teletext character sets, from the Noto Teletext glyphs the decoder compiles in, so a page's private designation of any of them draws (REQ-6668, DEC-6669). A `NAPLPS_OBJECTS` record's labels reach the renderer, kept for the screen-reader tree (TODO-6658), its title names the page in the page list when the program's index gives none, and a record of one object redefines that object alone (DEC-6638 item 3).
- **Teletext (ANIM-7, ANIM-8).** A sub-page's sprites and the state of its sprites and controls are composed over the page with the engine core's `compose_sprites` and `apply_animation` when the page is drawn; the rows that then differ are found by the page's change tracker.
- **The rule table's picture.** The rule table's host is given the `decoder::rules::Picture` of the sub-page on screen at every wake that evaluates ticks (`engine/animation.rs`, `PictureFacts::picture`; `Engine::animation_picture` is the same for a caller that holds nothing else of the engine): each target's state and bounding rectangle (ANIM-13), the objects at a pixel, `animate` and `advance_cycles`. What a rule changes is painted by the draw that follows the tick in the same wake (RULE-65).
- **The budget (ANIM-16).** A ReGIS picture's repaint is held to its repaint budget and a NAPLPS repaint for animation to 153,600 pixels a tick; a repaint held back is made later of the latest state, and is counted, and the host reports the count as `RULES_FAULT` 8 at most once a second (`Engine::take_animation_counters`, `rules::host::Rules::animation_slowed`). The state itself changes at once, so rules and input are never delayed.
- **The viewer's setting "Paint drawings gradually" (ANIM-15).** `Engine::set_paint_gradually(false)` makes every ReGIS drawing whole at once and every NAPLPS WAIT pass without delay. The setting is on by default: the settings file's line `drawings gradual` or `drawings whole`, the settings panel's "Paint drawings: Gradually / At once", and the message `ToDecoder::PaintGradually` (kind 188), told to every decoder when the setting changes and, when it is off, to each decoder as it starts. No channel is told.
- **The cursor's blink (ANIM-1).** The ReGIS cursor drawing in the decoder and the text cursor in the interface blink at `interface_core::regis::BLINK_HALF_TICKS`, 32 ticks; the decoder begins a blink on the tick due, so the cursor shows and hides on ticks.
- **What is sent** (ANIM-5). A ReGIS and a NAPLPS picture are sent as the rectangles that changed (REQ-6399), which is what an animation's damage is; a part may carry new colours, so a NAPLPS blink that changes only the colours sends no pixels, and the interface's painter, which holds each picture's indices, colours it again and draws again only what changed colour. A teletext page whose colours are unchanged is sent as the bounding rectangle of its changed dots (`FromDecoder::FrameArea`), which the trusted core lays over the frame the decoder last sent of the page (kept for each page a decoder shows, forgotten as it stops showing it or a new decoder starts) before its frame check and pacing; the core then sends the interface only the dots that changed, as before.
- **Renderer parity (neoteletext-common's `docs/renderer-parity.md`, REQ-6770, REQ-6771, TODO-6778).** HELLO offers `REGIS_CAP` and `NAPLPS_CAP` (`core/src/typing.rs`, PAR-1). Every ReGIS terminal is given the engine's 55 provided sets as alphabets 100 to 154, and every ReGIS and NAPLPS terminal the joining data of the plain Arabic-script sets 44 to 51, built once from the compiled Noto Teletext glyphs (`decoder/src/provided.rs`, PAR-12 to PAR-14). A `REGIS_CURSOR` record goes to both stores, and the NAPLPS store blinks the named object through its animation state at the ReGIS cursor's half period (`decoder/src/naplps.rs`, PAR-27); `DRAWING_LINKS` for a NAPLPS sub-page is matched against its pick record as for ReGIS (PAR-28). The interface sends with each pointer the picture pixels its screen pixel covers (`RegisPointer::span`), and the trusted core's pointer module looks at that block, up to `MAX_PICK_SPAN` (8), for the target nearest its centre (`core/src/regis_pointer.rs`, PAR-29). The picture layout carries row 0 of the teletext page above the picture (`interface_core::canvas::RegisLayout::header`, PAR-31), drawn by the desktop painter and by `draw-embedded`, which takes pictures of both sizes (`Screen::set_picture`, PAR-37); the cost line is in the developer console's Stats tab. A held page's pictures are not taken (`Engine::take_pictures`, PAR-32). The interface tells the trusted core the platform's reduced-motion preference (`ToCore::ReducedMotion`), which tells every decoder (`ToDecoder::ReducedMotion`; `Engine::set_reduced_motion`, PAR-33). The page cache keeps each picture sub-page's records within `MAX_KEPT_PICTURE_SOURCE_BYTES` and draws one no live terminal holds again from them (`decoder/src/page_cache.rs`, `PictureSources`, PAR-34). A fetched file's picture records are read by a reader of their own (`decoder/src/engine/feed.rs`, `read_fetched_records`, PAR-36 item 1). Not yet built: keyboard reach and link marks on pictures (PAR-30), user-loadable picture fonts (PAR-35) and the Tools menu's ReGIS and NAPLPS outputs (PAR-36 item 2).

**A page's rules act before its picture.** Each wake of a decoder first evaluates the open page's rule table for every tick due and writes what its rules produced, a local sound's MIDI messages first, before anything is drawn (`page-rules.md` RULE-64); an input event given to the decoder marks the page's next frame as the answer to input, which the frame interval does not hold (RULE-65). Flash is timed by the same presentation clock's ticks, 57 shown and 19 hidden for the Level 1 flash (`docs/animation.md` ANIM-1).

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
| Tab strip | Coloured tabs (the colour chosen by the trusted core from the site), close buttons, a new-tab button; a press of the middle button on a tab closes it (decision 63). On macOS the system's window buttons sit at its left end; elsewhere the window draws its own, the strip drags the window and its edges resize it; a tab dragged along the strip moves there, and dragged off it opens a window of its own or joins another window's strip; the tabs are one row that slides with easing to show the active tab whole, and a count of those not shown; a rest on the tabs, or a press on the count, slides out the tab filmstrip, a row of still pictures of every tab (decisions 69 and 72, DEC-6637, REQ-6640); a tab is titled by its channel's name from `channel.ttx`, else its host (REQ-6641); at its right end, before the window's own buttons, the browser's version and BETA, "0.1.0 BETA", as a quiet label (decision 123) |
| Trusted strip | Back and Forward, dimmed when history has nothing, then Reload and Home (decision 63); a padlock, "Not secure" or "File"; the address with its host in full ink; a bookmark star; a menu button. A click on the address selects it for typing; Enter sends it as strip input |
| Channel magazines | 100 to 800, each with its colour, range, name (from the channel's contents file) and page count (decision 105; "bank" in the code) |
| Page list | The chosen bank's pages, number and title; Up and Down step through it, and through the magazines when they were clicked (decision 105), Page Up and Page Down a list-height at a time; a click on it or on the sub-page list gives that list these keys, on a channel that takes the keys too, until a click elsewhere or Escape (REQ-1758); three digits typed choose a page. The divider between it and the page area is pulled to resize it (decision 105) |
| Page area | The bank's pages as a grid, sized by a logarithmic control that shrinks them over its whole travel to 80 pixels a page and shows a list only at its left edge. A click zooms a page to fill the area over 300 milliseconds, the wall dimming behind it; Escape or the close button returns. At the control's right end the zoomed page is shown, and the wheel or Up and Down slide to the next page of the bank, snapping into place: each notch of a mouse wheel is one page, at most one per slide, because macOS reports a slow notch as a tenth of a line. A page under the pointer pops out slightly |
| Overview | All 800 pages in eight rows: empty, available (listed in the contents file but not yet arrived), loaded, shown; the counts. A page that has arrived pops up at its native size under the pointer, which becomes a hand, never covering the trusted strip. A window starts with it off, and the View menu shows it. At its top right a chevron folds it into the footer, where it draws nothing else and asks for no redraws, and to the chevron's right a close button takes it off the window again |
| The page's colour | While a tab shows one page alone, that page's screen colour is the background of all the window below the trusted strip: the margins, the page area, the Channel magazines, the page and sub-page lists and the overview, which draw on it in black or white, whichever stands out more, with secondary text and rules mixed from that ink towards the colour. A magazine's own colour that would vanish on the page's gives way to the ink; rows and magazines with a fill of their own keep their white text, and a fill that cannot be told from the colour is edged in the ink. The strip, the console and the browser's panels and popups keep their own colours. The wall of pages, a page not yet arrived, a redirect's notice, a refused certificate and a denied selection stand on the browser's own background (REQ-740, `surface.rs`) |
| Developer console | Docked in the overview's place while open: the tabs Network, Packets, Pages, Log and Cookies, the last managing cookies by site and channel without ever showing a name or value (decisions 51, 62 and 63) |
| Tips | A label for each control drawn as an icon, a symbol or text that may be cut short, saying what it is and does and its shortcut, shown when the pointer rests on it for half a second, over everything, put away by a press or by leaving (decision 63) |
| Updates | A page whose content changes is marked in amber: a ring bursts from its cell in the overview and a dot stays for a minute; its row in the list flashes and shows how long ago it changed; its frame in the page area glows and fades over 3 seconds |

### 11.3 Input

- Pointer, wheel, keys and text from input methods arrive through winit.
- **The wheel and the pointer's wheel indicator are one decision** (REQ-6632, `native/interface-core/src/browser/pointer/wheel_owner.rs`). Whose the wheel is where the pointer is is a pure function, `wheel_owner`, of an explicit state, `WheelState`: the place (`Elsewhere`: outside the page area, or on what lies over it or takes the wheel itself, such as a panel, a popover, the console, a scrolling tip, the page-size bar and the full-screen strip; `Wall`: the page area with no page zoomed; `Margin`: beside the zoomed open page; `Picture`: over it; `Filmstrip`: over the open tab filmstrip, which lies over all of these, REQ-6640), whether the browser's own modifier is held, whether the open page's rule table claims the wheel at the pointer (the page takes the mouse and the trusted core said the pointer is inside a `wheel` rule's rectangle, RULE-51 item 4), and the ways there are items to step to. Its answer is `Scroll` (the browser scrolls what is there), `Page` (sent to the channel), `Filmstrip` (the browser slides the filmstrip's row, whatever the modifier or a rule) or `Steps` (the browser steps the open page's sub-pages, or the pages when it has none: in the margin, and over the picture wherever no rule claims the wheel or the modifier is held). The module's table has a row for every combination, each tested. `terminal_wheel`, `wheel_lines` and `scroll_by` route the turn by it, and `wheel_cursor` shows the indicator exactly where it is `Steps` with a way to go, after every shape of higher precedence `Browser::cursor` chooses first (resize shapes, the sub-page rows' hand, links and controls, ReGIS drawings, the overview's and console's buttons, rows, magazines and wall pages). It is not a machine with states of its own beside the four of DEC-6358, because no input needs a history the interface keeps; the interface reads it again when the trusted core's typing state arrives and after every frame drawn, so the pointer follows a new rule table, page or count of sub-pages without moving.
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
                            drawing), the tab filmstrip (+ filmstrip/, its one state, its drawing and the
                            stills it keeps through the canvas, REQ-6640), the toolbar, the address field, its suggestions, Back and Forward's
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
                            files, the session file (session_file.rs), settings (+ settings/proxy.rs), sleep,
                            the start page, typing (+ typing/records.rs); and the binary: main.rs, the
                            entry alone, broker.rs, the unconfined broker, source.rs, a file channel's bytes
      src/session/          the tabs and windows: a tab, its history and navigation, its channel, closed
                            tabs, windows set aside, the session saved, and what links ask of it
      src/trusted_core/     Core and its events, a file for each concern: the helper processes, channels,
                            timing, windows, intents, decoder and fetcher messages, the strip, the console,
                            settings, sound, updates, the phone, links, typing, start-up, the run, panel
                            numbers, page controls, library/ (history, bookmarks, the star, the bar),
                            cookies (+ cookies/panel.rs); hostile_tests.rs, the hostile interface harness (+ hostile_tests/: the stand-in broker's watch, the real helpers' run)
    decoder/                the decoder (neoteletext-decoder): its process (process.rs) and the engine as
                            it uses it (engine.rs, + engine/: feed, render), changes, flash, input records,
                            links, titles, the voice sink, and the Netscape bookmark reader (netscape.rs)
    fetcher/                the fetcher (neoteletext-fetcher): its arguments, its loop (process.rs), the
                            engine's agent's own parts, its cookies, and the engine's network port:
                            opening a connection and writing to it, over neoteletext-transport
    interface/              the interface process (neoteletext-interface): the system window, the reader
                            thread, keys, and app.rs with app/: frames, input, the input method, typing,
                            the phone, the clipboard and menus, windows, what the core said, outputs
    chooser/                the file dialog (neoteletext-chooser): the broker starts it for one choice;
                            on Linux also the desktop portal's reduced-motion preference (DEC-2424)
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
- The engine at a pinned commit, `8c59a89`, decoding T42 recordings and Teletext Text Interchange page files (DEC-1762).
- Web channels over the HyperText Transfer Protocol, plain or with Transport Layer Security: the contents file, fetching in its order at the engine's rate, push over WebSocket, update marks in the window.
- File channels, fed at the same pace.
- The window of section 11, drawn on the processor, redrawing only the boxes that changed since the last frame (`native-interface-design.md` section 12.7); and the same interface core drawn by the microcontroller back end in a test.
- Every process but the broker and the launcher confined on macOS, Windows and Linux from "deny everything" (decisions 52 and 58, 22 October 2026); sound in a helper of its own (decision 55); the trusted core confined behind the broker (decision 56; `sandboxing.md` section 7).
- Builds and tests on Windows, in a virtual machine, and on Linux (`DEVELOPMENT.md` section 12).
- Transport Layer Security and `https` channels (decision 41); fuzz targets for every reader of another process's messages and of every file the browser keeps (requirement 4.6).
- The address rules of section 9.6; tabs.
- The tab filmstrip (18 November 2026; DEC-6637, REQ-6640, REQ-6641), on the branch `filmstrip`: a 300 millisecond rest on the tabs, or a press on the count, slides out from under the strip a row of still pictures of every tab, each about a fifth of the window wide, captioned with the tab's title and page number; the wheel slides the row with easing, as far as bringing its end picture across the window; a press on a picture chooses the tab, and the strip, now one sliding row, slides to show it. The stills are made by the canvas (`Canvas::keep_still`, at most `MAX_STILLS` of `STILL_WIDTH` by `STILL_HEIGHT`) from the images it holds when the viewer leaves a tab, and for the active tab when the filmstrip opens. The overflow panel of decision 102 is withdrawn with its code. A tab is titled by the name in its channel's `channel.ttx`, else by its host. Seen on a private display with the deployed channels on 18 November 2026.
- A page's rule table and animation (18 November 2026; neoteletext-common's `docs/page-rules.md` and `docs/animation.md`, DEC-6522, DEC-6524; REQ-6534, REQ-6535, REQ-6536), on the branch `rules-animation`, which merges the branches `rules` and `animation`: push protocol 9, in which a viewer's input goes only inside a `RULE_EVENT`; the decoder as the evaluator's host and as the compositor of animation, the rule table acting on the picture of the sub-page on screen (section 10); the trusted core as the gate, with the wheel rule; local sounds in the sound helper and metering after every device buffer; the tracked values of a synthesizer sound played from blocks, read from the engine's receiver; the setting "Paint drawings gradually"; a page's frames once a tick and the cursor's blink in ticks; the fuzz targets `core_report_gate` and `protocol_audio_local`; and the timed test of a key to its sound and its level (section 6.4.1). Seen working end to end on 18 November 2026 with the real, confined processes on a private display, against the protocol 14 server and the music player channel: pages 101 and 102 draw, the keys A, D and G show their pressed drawings while held and play the page's local sound, the meter's segments light while a note sounds and fall after it, the keys 1 to 4 change the sound and the name shown, and the server counts no input from the page; on the demo site a page number and a calculator's keys reach the program through the default table, a click on the ReGIS test channel's page 101 reaches it as a pick, and the NAPLPS and audio demos draw and play. Not built: the developer console's panel of a page's counts (they are written to the channel's log, RULE-72); a teletext page's frames as damaged rectangles and a NAPLPS blink without pixels (ANIM-5); LINKS, DRAWING_LINKS and RULES records held in an open frame (ANIM-2); the pixel maximums a tick for ReGIS and teletext (ANIM-16); a new sheet generation restarting the shown sub-page's renderer (ANIM-17); `playing` falling when a stored sound of samples ends by itself; the fetcher's unused move slots; and RULE-66's timed test as a test kept in the repository (the measurement of section 6.4.1 was made with scratch scripts; kept since 19 November 2026 as `scripts/timed-test.sh`). rdm's TODO-6563, TODO-6574 and TODO-6576 hold each.
- The second form of a page's rule table (18 and 19 November 2026; DEC-6587, DEC-6599, DEC-6635, DEC-6580; REQ-6534, REQ-6535, REQ-6536, REQ-6626), on the branch `rules-2`: one table for a page and sub-code, every event handled at once, the four ends of RULE-111, the gate by page, sub-code and row, the sound command budget, the small device buffer, pause and heard cues; the presentation clock locked to the interface's frames and the paint points (RULE-98, RULE-65), the FramePhase sent relative to its writing (DEC-6659 item 12) and, as winit gives no frame times, sent only by the timed test's display; a teletext page's changed rectangle and a picture's new colours without pixels (ANIM-5); the decoder's processor time read by the broker and the tab's mark (REQ-6626); the timed test between the real processes (RULE-66, `scripts/timed-test.sh`) and its six sets of figures; and the tests of the sound command flood, unbroken samples across a buffer change and teletext sprites drawn by both routes. Not met yet: RULE-65's picture bounds and RULE-64's 2 ms for LocalMidi in that test; not built: a platform's frame times, the processor time on macOS and Windows, and the engine's call to drop unacknowledged reports from a kept session. rdm's TODO-6660 holds each.
- Sound: every tab heard and each muted (decisions 37, 38 and 40); on macOS the Dock menu (decision 39).
- The wall on a live channel (31 October 2026; REQ-1558, REQ-1559): each tab's decoder keeps, over the engine's store, which pushed pages and sub-pages it holds, at most 128, how old each is and when it was last drawn (`native/decoder/src/page_cache.rs`, the `page-cache` feature). The source is the store's packets; nothing is an image. An entry older than 24 hours is dropped as it is about to be drawn. The decoder tells the trusted core which pages on screen it cannot draw, and the core selects them with the ordinary SELECT, one at a time in screen order, each after its frame or the channel's 5-second limit for answering, then SELECT page 0 (`native/core/src/trusted_core/wall_fill.rs`). The wall lays out at most 128 tiles. An entry dropped leaves its store slot until the engine reuses it: the engine's store has no removal, so the store's own size bounds that memory, not the cache's.
- Hyperlinks, phases 1 and 2 of `hyperlinks.md` (20 October 2026), behind the `links` feature, on by default: page numbers found in the text and `LT`, `LK` and descriptor links, decided by the trusted core; the underline drawn by the interface; the destination and the web-browser prompt in the strip; the hand-off to the system's web browser (`web-opener`); `#302` page addresses.
- Channel push over WebSocket, typing back to a channel, its cursor and the phone keyboard (`interaction.md`, decisions 45 and 46, 20 October 2026); input protocol version 2 (decision 48) and pasting into a page that takes input (decision 49), 21 October 2026.
- Settings kept between runs with the bookmarks, the settings panel of the strip, Home and the start page `neoteletext:home`, the developer console, and HTTP and SOCKS5 proxies (`settings-home-console-proxy.md`, decision 51, 21 October 2026). Since DEC-1652 the default home, which a new browser and a new tab open, is the home channel at `https://app.neottx.com/home/` (`HOME_CHANNEL` in `native/core/src/address.rs`); the built-in start page needs no network and opens in its place when nothing answers for it (DEC-2364).
- History and bookmarks (`history-and-bookmarks.md`, decision 57, 22 October 2026): history kept 90 days with its panel, search and removal, private tabs, recently closed tabs, Back and Forward lists, address-field suggestions; bookmarks in folders with the star's popover, a manager, a bookmarks bar, import and export in this browser's format; the menu button's menu.
- The session kept after every change and restored at start, with its setting (`history-and-bookmarks.md` section 15, decision 59, 22 October 2026).
- The Colour Map of Levels 2.5 and 3.5 and the screen colour (decision 60), and page images in the browser's own form (decision 61), 22 October 2026.
- Cookies held by the trusted core, one jar for ordinary tabs and one for private, fetchers asking per request; kept between runs sealed under a key in the platform's secret store; the cookies panel, its settings, clearing with history and `Clear-Site-Data` (`cookies.md`, decision 62, 22 October 2026).
- Reload, closing a tab with the middle button, cookie management in the developer console, and tips on every icon and symbol (decision 63, 23 October 2026).
- History following Chrome's rules (`history-and-bookmarks.md` section 16, decision 64, 23 October 2026): reloads counted as Chrome counts them, lists of 12 with chapter stops, removal reaching the tabs' lists, closed tabs kept between runs, the last 15 minutes, a row's menu.
- Fastext, reveal, hold, mix and sub-pages (`page-controls.md`, decision 67), with the receiver's row 24 of pages with `LN` lines; the remote control (`remote-control.md`, decision 68); and dragging tabs and their overflow (`tab-strip.md`, decision 69), 23 October 2026.
- The console's Log, each line with its level and process, and the channel's own `LG` lines (decision 70); bookmarks imported from other browsers' Netscape files, and "Move…" (decision 71); multiple windows, with tabs moved and torn off between them (decision 72), 23 October 2026.
- Every sub-page to the standard's 8,190, counted, stepped and placed in 16 bits from the engine's `neoteletext_core::limits`, the sub-page list a virtual list of rows asked for by range, and sub-pages as files of their own with each page's catalogue (decisions 112, 119 and 121, 28 October 2026); PCM16 and IMA ADPCM compiled out of the desktop builds, which `scripts/check.sh` proves (decision 121).
- One owner for a channel's magazines, pages and sub-pages (DEC-6358, REQ-6361, 16 November 2026). The trusted core's directory holds, for each tab, what the channel says of each page's sub-pages: the count its index declares and its list (`native/core/src/subpages.rs`), which what a decoder happens to hold never overrides; lists are kept by a budget of rows, `ROWS_KEPT`. The interface is a view: a page's count travels in its entry of the channel's list (`PageInfo.subpages`), and the interface keeps none of its own. A request for a list's rows, its answer, a choice of sub-page and the open page's controls each carry their channel, and a request its number, so nothing is answered from or into another tab. Four state machines, written as the engine's are with a test for every row of each table, hold the sequencing: the rows asked for and the move waiting to be shown, in the interface (`native/interface-core/src/browser/subpage_machines.rs`); each page's list of sub-pages (`native/core/src/subpages/list_state.rs`); and the choice of sub-page on its way to being shown (`native/core/src/trusted_core/selection.rs`). The open page's controls are each window's own.
- The browser's version and BETA at the tab strip's right end in every window, the version the trusted core reports running (decision 123, 28 October 2026).
- Requirement 3.9's hostile interface harness against the trusted core (`native/core/src/trusted_core/hostile_tests.rs`), run by the commit gate, with bookmarks, the start page and a second, private window; on macOS and Linux against a stand-in broker whose requests and helpers' input it checks (finding F-98); and, on macOS and Linux, with the real decoder and fetcher confined against two reference channels on this computer (`hostile_tests/real_helpers.rs`, TODO-1254): the channel behind hears exactly a quiet run's requests, the channel in front only those and bare SELECTs of its own pages, and nothing else is reached.
- Installation and updates on every desktop (`installation-and-updates.md`, decisions 54 and 73 to 82, 23 October 2026): the launcher, the confined updater with `tough`, the Windows stub and installer, Linux's `--install`, removal, the release tool, and the core's offer, setting and health check, proved end to end with test keys on macOS, Windows and Linux.
- The certificate-error screen (`certificate-errors.md`, decision 83, 23 October 2026).
- The browser's icon, the owner's art, on every desktop (decision 124, 28 October 2026): the macOS bundle's icon file, made by `iconutil` and checked pixel for pixel, and the Dock's icon outside a bundle; a resource in the Windows stub, installer, launcher, broker and interface, written by a build script; Linux's icon files in the user's icon theme and the menu entry naming them; every window's icon on Windows and X11, and its class on Linux. Every size by nearest neighbour from the one source, `native/icon/neoteletext-icon.png`.
- The menu bar on macOS, and cut, copy and paste in every text field on every desktop (`menu-bar.md`, decision 84, 23 October 2026).
- Full screen of the page alone (`full-screen.md`, decision 85), tabs of one width with the overflow panel raised as a dialog (decision 102), and a tab's address in its tip (decision 103), 25 October 2026.
- The source reorganised by concept, 24 October 2026 (`~/devel/neoteletext-common/structure/`, documents 20 to 23): each crate's files split by what they hold, every module's tests in a `tests.rs` in its own folder, every function under 80 lines and every `impl` block under 300, with no `too_many_lines` allow left; no file of code over 500 lines but the fetcher's `transport.rs`, which the security fixes grew to 564 (`TECH-DEBT.md`); the bytes of 150 messages on every pipe the same before and after; the modules that may hold unsafe code split into leaves, each saying so (requirement 10.12). Section 14 is the layout.
- The fixes of the security review of 24 October 2026 (`~/devel/neoteletext-common/reviews/2026-10-24-security/FIX-PLAN.md`, decisions 86 to 100), each with a regression test that failed on the code before it: on Linux no helper reaches the session's sockets (decision 86), calls by the x32 numbers, types into its terminal or signals another process (decision 94), and each ends with its broker; on macOS every path passed to a profile as a parameter (decision 97), the fetcher kept off this computer unless allowed (decision 89), and the interface's and the sound helper's reads narrowed to this user's own folders; on Windows the interface in an AppContainer of its own (decision 90), its test passing in the virtual machine; the broker reading only teletext, never through a link, and opening the web at most once a hand-off interval (decision 88); the launcher accepting a copy of each version on every desktop (decision 87), a floor on the update root's version (decision 91), the Windows manifest signed (decision 92), a start deadline and then a health deadline with the broker's `started` line (decision 93), a narrow AppArmor profile (decision 95), and the updater's requests, its clock and the launcher's recovery bounded; phone keys outside a typing page only moving between pages (decision 99); each helper's messages held to a budget of its own, a page's frames to one a field, a fetcher's connection to one deadline and a name leading to this computer refused before connecting; slicing a string at a byte offset denied in every crate that reads outside input. What is left is in `TECH-DEBT.md` and, for the owner, rdm to-dos for the owner.
- The return channel's answer drawn without waiting (`~/devel/neoteletext-common/RETURN-CHANNEL.md` sections 2.2 and 2.4, REQ-RCH-001 to REQ-RCH-005; decision 122, 28 October 2026): the page keys go to, or the page under the pointer, drawn as its frame arrives, a frame that changed nothing delaying none; three clocks a page in the trusted core, so the channel's answer waits neither for the look's frame nor for the header's, three frames a field at most; what an event sends written once it is handled, before the frame it asks for; one frame queued on every platform, by inspection; and the middle, right, back and forward buttons and the mouse's modifiers sent over a page that takes input, a link the page declares being the browser's there as on every page (requirements 18.11 to 18.15).

**Not built:**

- Any sandbox on Android or iOS; Windows helpers started apart from the console they were started from (decision 100 item 1, measured as needed in the virtual machine on 24 October 2026); the fetcher's connections made by the broker (decision 89);.
- The grace period for closed tabs; the live-feed threshold; refresh schedules.
- Discovery.
- Screen readers, reduced motion.
- The hostile interface harness with the real helpers on Windows, where `confine` has no stand-in broker (TODO-2200's desktop session).
- A release signed by the owner's keys: the certificates, the hardware keys, the update host and the key ceremony are the owner's (rdm to-dos for the owner); the Linux release build on the C library 2.28.
- Builds on phones and microcontroller targets.
- A server's catalogue of sub-pages, its titles by range and tuning by sub-page (decision 112 item 3), which await the server; a channel's IMA ADPCM and PCM16 sound on the desktops, by the owner's instruction (decision 121 item 7).

---

## 17. Glossary

| Term | Meaning |
|---|---|
| AccessKit | A Rust library that publishes an application's accessibility tree to each platform's screen readers. |
| App Sandbox | The macOS sandbox for applications: a process runs with only the entitlements it is given. |
| AppContainer | The Windows sandbox for applications: a process runs with only the capabilities it is given. |
| Noto Teletext | The engine's one font, converted from Noto to the 12 by 20 cell and compiled in (its REQ-FNT-001). |
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
