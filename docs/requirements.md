# NeoTeletext: requirements

| | |
|---|---|
| **Status** | Adopted 16 October 2026 with the native architecture (decision 33). Every limit these requirements refer to is a constant defined in neoteletext (decision 26; requirement 2.4). |
| **Companion documents** | [`architecture.md`](architecture.md) explains the design these requirements describe. [`security-analysis.md`](security-analysis.md) analyses its security. The earlier design's requirements were in `prototypes/docs/requirements.md`, kept in git at the tag `prototypes-final` since the prototypes were removed (decision 50); a path under `prototypes/` here is a path there. |

## How to read this document

- **No abbreviations.** Every technical term is written out in full. Initials appear only where they are a literal identifier in code, configuration, a file name or a standard's title. `architecture.md` section 17 is the glossary.
- **Numbering.** Each requirement is numbered by section: "Requirement 6.3" is the third requirement of section 6. **Numbers are carried over from the earlier design's requirements and never reused.** A requirement that existed only to contain a web engine keeps its number and is marked **withdrawn**, with the reason. New requirements take the next free number in their section.
- **"Must"** means the requirement is mandatory. **"Should"** means it is expected unless a recorded reason says otherwise.
- **Each requirement states:**
  - **Statement:** what must be true.
  - **Reason:** why it matters.
  - **Source:** the owner's decision it comes from (numbered in `architecture.md` section 2), or "Architecture" when it follows from the design, or "Prototype finding" when it was learned on this codebase.
  - **Verification:** how it is shown to hold. "Test" means an automated test. "Check" means an automated check in the project's check script. "Review" means a recorded human review.
  - **State:** whether it is met today, **met**, **partly met** or **not met**, as of the date it gives, with what is missing.
- **Engine requirements.** Where a requirement is already met by the teletext engine `neoteletext`, its own requirement identifier is cited, for example "`neoteletext` requirement REQ-SEC-006".

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
10. The interface process
11. The trusted strip
12. Window features
13. The application update mechanism
14. Build, dependencies and verification
15. Platforms
16. Sound
17. Hyperlinks
18. Push, typing and the cursor
19. Settings, the home page, the developer console and proxies
20. History and bookmarks
21. Cookies
22. Fastext, reveal, hold, mix and sub-pages
23. The remote control

---

## 1. Purpose and scope

### Requirement 1.1: What the application is
- **Statement:** NeoTeletext must be an application for browsing teletext delivered over the web and from local files. Each tab must show one teletext channel, and every page shown must be a teletext page.
- **Reason:** This defines the product and bounds what it may fetch and show.
- **Source:** Decisions 1, 12 and 33.
- **Verification:** Review.
- **State:** Met.

### Requirement 1.2: Security comes first
- **Statement:** Where a requirement trades capability, convenience or performance against security, the choice that is more secure must be made, and the trade-off must be recorded in `security-analysis.md`. Security must never be weakened or downgraded: no protection the design names may be dropped because a machine cannot give it, no weaker mode may be fallen back to, and no setting, switch or variable may select one; a process that cannot have every protection is not run (decision 58).
- **Reason:** The owner has stated that being hacked is not acceptable under any circumstances, and that security is paramount, with no weakening or downgrading.
- **Source:** Decisions 4 and 58.
- **Verification:** Review of every change that touches a requirement in sections 3 to 8, 10, 11 or 13.
- **State:** Met as a practice.

### Requirement 1.3: No executable content
- **Statement:** The application must never execute code supplied by a page, a channel, a server or a file. It must contain no interpreter reachable from outside data, except the decoder's WebAssembly interpreter, which decision 41 adopted for iOS (open decision 2), running only the application's own decoder module running only the application's own decoder module.
- **Reason:** Downloaded code is the most direct route to compromise.
- **Source:** Decision 4; `neoteletext` decision E.32.
- **Verification:** Review; check that no crate in the application's dependency tree is a script engine, a web engine or a general interpreter.
- **State:** Met; the check is not written.

---

## 2. The teletext engine

### Requirement 2.1: The engine is `neoteletext`
- **Statement:** All teletext recognition, decoding and rendering must be done by the `neoteletext` crates. All web protocol reading must be done by `neoteletext-web`. NeoTeletext must not contain a second implementation of any of these.
- **Reason:** One implementation, already tested and fuzzed, is safer than two.
- **Source:** Decisions 1 and 36.
- **Verification:** Review.
- **State:** Met: the decoder uses the engine's readers, decoder and renderer, and the fetcher the engine's user agent.

### Requirement 2.2: The engine does not depend on a user interface toolkit
- **Statement:** No `neoteletext` crate that NeoTeletext uses may depend on a user interface or graphics toolkit.
- **Reason:** The trusted processes must not carry toolkit code they do not need.
- **Source:** Decision 1.
- **Verification:** Check: a script runs `cargo tree` on each `neoteletext` crate the application uses and fails if any toolkit appears.
- **State:** Met; the check is not written.

### Requirement 2.3: The engine's own guarantees are kept
- **Statement:** The `neoteletext` crates must be built with their own checks, fuzzing and feature rules, and NeoTeletext must not enable any feature of them that the engine's rules forbid in a browser build.
- **Reason:** The engine's safety rests on those rules.
- **Source:** Architecture.
- **Verification:** Check: the NeoTeletext check script runs the `neoteletext` check script for the commit it builds against.
- **State:** Not met: the check is not written.

### Requirement 2.4: Limits and fetching behaviour come from neoteletext
- **Statement:** Every limit on fetching, refreshing and input must be a constant defined once in neoteletext and used by NeoTeletext by name. The fetching policy of requirements 4.7 and 5.15 to 5.17 must be implemented in neoteletext. Where neoteletext does not yet define a limit or behaviour, it must be added there (`architecture.md` section 5.1), not in NeoTeletext.
- **Reason:** One definition of every value and one implementation of channel fetching.
- **Source:** Decision 26.
- **Verification:** Check: NeoTeletext's source defines no constant for any of these limits; review.
- **State:** Partly met. Two of the engine's values are written again instead of named: the fetcher's connection time-out, 10 seconds (`CONNECT_TIMEOUT`, the engine's `DEFAULT_CONNECT_TIMEOUT_US`), and the trusted core's pace for file channels, 0.2 seconds (`LOAD_INTERVAL`, the engine's `DEFAULT_FETCH_INTERVAL_US`). Since 22 October 2026 the fetcher keeps no cookie store (decision 62); since 23 October 2026 the sub-page bound is the engine's `viewer::MAX_LISTED_SUBPAGES`, asserted equal to the interface core's.

### Requirement 2.5: The engine is pinned and never patched
- **Statement:** NeoTeletext must depend on the engine by one pinned commit, read from the engine's repository, never from a copy of its source or its working tree, and must never patch it locally.
- **Reason:** The engine is maintained in its own repository, by its own process (decision 36).
- **Source:** Decision 36.
- **Verification:** Check: every `neoteletext` dependency in `native/Cargo.toml` names the same `rev`.
- **State:** Not met, 23 October 2026: `native/Cargo.toml` names the engine's `00c90ea`, but the updater's workspace (`native/tuf/Cargo.toml`, `neoteletext-web`) and the fuzz workspace (`native/fuzz/Cargo.toml`, `neoteletext-qrkeyboard` and `neoteletext-input`) still name `d9ca77f`, so the fuzz targets build two engines. The check is not written; `scripts/check.sh` checks that `native/Cargo.toml` names one engine repository, not one commit. The pinned commit is in the engine's local repository and is not yet published.

---

## 3. Processes and isolation

### Requirement 3.1: Separate processes
- **Statement:** The application must run as separate processes: a broker, the one process not confined, which starts the others and does for the trusted core only what section 7 of `sandboxing.md` lists; a trusted core; one fetcher per web channel; one decoder per channel; an interface process; a sound helper; the phone keyboard while it is open; and, when built, a font helper, an updater and a launcher.
- **Reason:** Separation lets each part hold only what it needs, so that a compromise of one part gains little.
- **Source:** Decisions 18, 33, 55 and 56.
- **Verification:** Test: the running application's process list matches this set.
- **State:** Met for the processes built (broker, trusted core, fetcher, decoder, interface, sound helper, phone keyboard, updater, launcher): on 22 October 2026, on macOS, every helper and the core ran as the broker's children (`sandboxing.md` section 7.5); on 23 October 2026 on macOS, Linux and Windows the launcher started the broker, which started the confined updater on the core's request (`installation-and-updates.md` section 15, decision 73). There is no font helper (decision 66). The test of the process list is not written; `the_trusted_core_starts_confined_and_reaches_its_own_code` starts the core through the broker.
- *Amended 23 October 2026 (decision 66):* there is no font helper: fonts from channels are not supported.

### Requirement 3.2: The trusted core contains no web engine and no format reader
- **Statement:** The trusted core must not link or load a web engine, a script engine or any reader of network or teletext formats. It must receive only typed, bounded results from the other processes.
- **Reason:** The trusted core holds everything the user trusts; it must be the smallest and least exposed part.
- **Source:** Decision 4.
- **Verification:** Check: the trusted core's dependency tree contains no web engine, script engine, `neoteletext-formats` or `neoteletext-web`.
- **State:** Not met as stated, 23 October 2026: the trusted core links no web engine, script engine or `neoteletext-formats`, but it reads with the engine: `neoteletext-core` for each packet's address in the console (`console.rs`, requirement 19.9, decision 51) and the local-address rule; `neoteletext-web` for the address parser, the proxy's address (decision 51) and the cookie jar, which reads the `Set-Cookie` lines the fetcher passes on (decision 62, requirement 21.1); and `neoteletext-input` for the input records it encodes and the cursor records it checks (`typing.rs`, decision 48). `neoteletext-image` went with Portable Network Graphics (decision 61). Another browser's bookmark file is read by a decoder (decision 71). The check is not written; as the Verification words it, it would fail on `neoteletext-web`, so the Statement or the check needs the owner's word.

### Requirement 3.3: The fetcher has only the network
- **Statement:** The fetcher must be able to reach the network and nothing else: no files, and no other processes except through its pipe to the trusted core.
- **Reason:** The fetcher reads untrusted protocol data and must be worth little to an attacker who compromises it.
- **Source:** Architecture; `neoteletext` requirement REQ-SEC-018.
- **Verification:** Test on each platform: the fetcher's attempts to read or write a file, or start a process, fail.
- **State:** Met on macOS and Linux, 22 October 2026 (`sandboxing.md`, decision 52): on macOS its profile starts from "deny everything" and allows outbound TCP, the name resolver's socket and files, and nothing else; on Linux it confines itself with Landlock (the resolver's directories readable, nothing else) and a seccomp filter (no programs, sockets of the internet families only), tested by the crate's probe. Partly met on Windows: an AppContainer with the internet client capability, in a job object with the mitigation policies, which denies file reads only where packaging granted the container read of the program folder.

### Requirement 3.4: The decoder has no rights
- **Statement:** Each decoder must have no network access, no file access and no secrets.
- **Reason:** The decoder reads the most untrusted data; a fault in it must stay inside its own process.
- **Source:** Architecture.
- **Verification:** Test on each platform: the decoder's attempts to open a file or a network connection fail.
- **State:** Met on macOS and Linux, 22 October 2026 (`sandboxing.md`, decision 52): on macOS its profile starts from "deny everything" and allows its pipes alone; on Linux it confines itself with an empty network namespace (required since decision 58), Landlock with nothing readable and a seccomp filter with no sockets, no programs and no files, tested by the crate's probe (it can open no file, start no program, connect nowhere, write nothing). On Windows a less privileged AppContainer with no capabilities, in a job object with the mitigation policies and the Win32k filter. On iOS it must also run inside `wasmi` (decision 41, open decision 2), not built; on desktops that waits on measuring its cost.

### Requirement 3.5: Withdrawn: The font helper has no rights
Withdrawn on 23 October 2026 (decision 66): the browser does not support fonts from channels (the owner, 23 October 2026), so no font is fetched and there is no font helper.
- **Was:** Fonts fetched from the network must be read by a confined font helper with no network access, no file access and no secrets. Its output must be checked by the trusted core (hash and table shape) and checked again by the decoder before use.

### Requirement 3.6: The interface process holds no secrets and decides nothing
- **Statement:** The interface process must never receive cookies, credentials, history or bookmarks beyond what is on screen, or any setting value that affects security other than as the text the trusted strip shows, on which it acts in no way (amended 21 October 2026, decision 51). It must have **no network on every platform**, and no file access beyond its own resources and fonts. It must make no decision: it shows the state the trusted core sends and passes on what the user did.
- **Reason:** An attacker who took over the interface must find nothing of value.
- **Source:** Decisions 4, 18 and 33.
- **Verification:** Test on each platform: network and file access fail from the interface process; review of every message type sent to it.
- **State:** Partly met. No secret is ever sent to it (review of `protocol`). On macOS its profile starts from "deny everything" and allows the system's folders and the window, font, text-input, pasteboard, notification, preference, sound and Dock services, with no network and no programs; on Linux it confines itself with an empty network namespace where allowed, Landlock (fonts, preferences and the system's directories readable) and a seccomp filter (local-domain sockets only, no programs); on Windows a restricted token at low integrity in a job object with the mitigation policies, which does not stop reads of the user's files. Its environment is cut to the window system's on every platform (22 October 2026, `sandboxing.md`).
- *Amended 22 October 2026 (decision 57):* "beyond what is on screen" is exact for history and bookmarks: the interface receives them only as the views of `history-and-bookmarks.md` section 4, at most 100 rows at a time, and names a row only by its view's number (requirement 20.5).

### Requirement 3.7: Operating-system sandboxes
- **Statement:** On each platform the processes must be sandboxed as in `architecture.md` section 6.5, starting from "deny everything" and allowing only what each process needs. Every layer named there for a process is required: a process that cannot have all of them must not run, and must not run with fewer (decision 58).
- **Reason:** Each sandbox is a second wall behind the first.
- **Source:** Architecture.
- **Verification:** Test on each platform, as in requirements 3.3 to 3.6.
- **State:** Partly met, 22 October 2026 (`sandboxing.md`, decisions 52, 55, 56 and 58): every helper is confined from "deny everything" on macOS (`sandbox-exec` profiles), Linux (Landlock enforced whole, a network namespace for those without the network, and seccomp, applied by the helper itself before any thread, every layer required since decision 58: `a_helper_without_its_namespace_does_not_run` and `a_file_to_read_keeps_landlock_whole` failed on the code that reported a missing layer and ran) and Windows (job object, mitigation policies, AppContainer for the decoder, fetcher and sound helper, required since decision 58, with no fallback: `a_helper_whose_container_cannot_be_made_is_not_started`; a restricted token for the interface by design), with an environment of only what it needs; the sound helper holds the sound output and the interface no longer does; the trusted core is confined on all three behind the broker, with no network, no programs and only its settings folder (`sandboxing.md` section 7), checked by `the_trusted_core_starts_confined_and_reaches_its_own_code` on each and by running the browser on each. Not met: the broker itself, which is unconfined by design and parses only the core's bounded requests; the interface's file reads on Windows; Android and iOS.
- *Amended 23 October 2026 (decision 70):* each helper's confinement, as the trusted core started it, is a line of the console's Log tab, so it can be read from the window on every desktop, a Windows helper started from Explorer included.

### Requirement 3.8: The macOS sandboxing gap is closed
- **Statement:** The fetcher and decoder must be sandboxed on macOS before the first release, and the trusted core too (decision 56).
- **Reason:** Without it the macOS build lacks the second wall.
- **Source:** Architecture.
- **Verification:** Test, as in requirements 3.3 and 3.4, on macOS.
- **State:** Met for the build, 22 October 2026: every process but the broker runs under a `sandbox-exec` profile, the trusted core's started by the broker (`sandboxing.md` section 7). `sandbox-exec` is deprecated by Apple; it still works on macOS 15, and the release build's packaging (`installation-and-updates.md` section 4.1) must keep each process under a profile of its own.
- *Amended 23 October 2026:* the bundle, started by its launcher, ran healthy with every process under its profile and the updater under its own (`installation-and-updates.md` section 15, decision 73).

### Requirement 3.9: Security does not depend on the interface's code
- **Statement:** Every security property in this document must hold even if the interface process runs code written by an attacker, **except** the integrity of the trusted strip and of strip input, which the interface process draws and sends (`architecture.md` section 6.1; decision 41 kept the strip in the interface process, open decision 3).
- **Reason:** The owner's principle that the interface's code must make no difference to security (decision 20). In this architecture the strip is drawn by the interface process, so its integrity depends on that process; every other property must not.
- **Source:** Decisions 20 and 33.
- **Verification:** Test: a hostile interface harness replaces the interface process with an adversarial client that sends every intent kind with arbitrary values, malformed messages, floods and replayed messages; the test shows that no secret is disclosed, no file or network is reached, no channel's data reaches another channel's requests, and no intent other than strip input changes channel.
- **State:** Met in part (23 October 2026): the harness (`native/core/src/hostile_tests.rs`, run by the commit gate) sends 30,000 messages made at the byte level, every kind with arbitrary values, mutated real messages, lengths that lie and kinds the interface may not send, through the protocol's own reader into the trusted core's own handling. It shows that the core never panics, that no cookie's value reaches the interface, and that no intent but strip input changes a tab's channel or a setting; a fault planted to move a tab on a page intent is caught within 20 messages. It drives the core in its own test process, not through a substitute interface process, since a way for the broker to start another interface would be a way into a release build (`IMPLEMENTATION-LOG.md`, "The round of 23 October 2026", row 16). Not shown by it: that no file or network is reached (requirements 3.3 to 3.7, by the sandboxes) and that no channel's data reaches another's requests (requirement 7.3, by one fetcher per channel), which need running helpers.

### Requirement 3.10: Withdrawn
- **Was:** the Linux window host confined from outside.
- **Why withdrawn:** it existed to contain a web engine (decision 33). The interface process's Linux sandbox is requirement 3.7.

### Requirement 3.11: Withdrawn
- **Was:** the Linux window host drawing on a nested display of its own.
- **Why withdrawn:** it existed to contain a web engine (decision 33).

### Requirement 3.12: One fetcher per web channel
- **Statement:** Each web channel must have its own fetcher process, which fetches only that channel's resources.
- **Reason:** A fetcher compromised by one channel's server then holds only that channel's connections and cookies.
- **Source:** Architecture.
- **Verification:** Test: two web channels open two fetchers, and each fetcher's requests reach only its own channel's server.
- **State:** Met; the test is not written.

### Requirement 3.13: Background tabs sleep
- **Statement:** A channel's decoder must run only while its tab is awake: while it is the active tab, while its channel holds a push connection, while its sound plays, and for 15 minutes after it was last shown. A tab restored with a session, not yet shown and with nothing live, must sleep. A channel's fetcher must run for as long as its tab is open, asleep or not, and fetch the same whatever is shown (decision 22). What the trusted core gave a channel's decoder must be kept, up to 8 MB, and given to a new decoder when the tab wakes, with the rest of a file channel read as before; a channel whose data grows beyond that stays awake. A tab whose channel's push connection opens wakes at once; the trusted core looks for tabs that may sleep at least once a minute.
- **Reason:** Teletext is mostly pushed, and a pushing channel is live in the background, so only a tab with nothing live sleeps; its decoder at rest took 16 MB before requirement 3.14 and 6.2 MB after, for pages nobody is looking at. Closing or pausing a fetcher would tell the channel's server when its tab is looked at.
- **Source:** Decision 47.
- **Verification:** Tests of the policy: `a_tab_sleeps_once_it_has_not_been_shown_for_a_while_unless_it_sounds`, `a_channels_data_is_kept_for_its_next_decoder_and_too_much_keeps_it_awake`, `a_tab_whose_channel_pushes_never_sleeps`.
- **State:** Partly met, 21 October 2026: built and its policy tested; a decoder that slept and woke has not been watched in the window by hand. The sound helper tells the trusted core which channels sound (`FromSound::Sounding`, since decision 55; the interface did before), the fetcher whether it holds a push connection (`FromFetcher::Pushing`); a decoder or fetcher put to sleep that reports after the next one started is not listened to.

### Requirement 3.14: A decoder's store grows as it fills
- **Statement:** A decoder must start with room for 256 pages and sub-pages, and report once when it has received as many as it has room for; the trusted core must then start it again with twice the room, up to 4,096, and give it what the channel's decoder was given and the pages shown. A channel whose data was too much to keep (requirement 3.13) keeps the decoder it has.
- **Reason:** A store of 4,096 pages takes 10.5 MB from the start, and most channels have a few hundred pages.
- **Source:** Decision 47.
- **Verification:** Tests: `a_store_that_fills_is_reported_once`; the protocol's round trip of `StoreFull`.
- **State:** Met, 21 October 2026: a decoder at rest took 16 MB of physical memory, and 6.2 MB with the first store (release, `vmmap`). The restart with more room has not been watched by hand with a channel of more than 256 pages.

---

## 4. Messages between processes

### Requirement 4.1: One framed protocol
- **Statement:** All processes must communicate with the engine's framing (one kind byte, a 32-bit big-endian length and a payload), defined once in the `protocol` crate.
- **Reason:** One small, strict protocol for every boundary.
- **Source:** Architecture.
- **Verification:** Review.
- **State:** Met. Since 22 October 2026 (decision 53) a process flushes its pipe once per event rather than per message; under eight flashing pages the interface's system calls fell from 2,133 to 540 in 10 seconds.

### Requirement 4.2: Every message is bounded
- **Statement:** Every message, and every field in it, must have a fixed maximum size, stated in code as a named constant. A message larger than its maximum must be rejected before its payload is read.
- **Reason:** Unbounded messages are a route to exhausting memory.
- **Source:** Architecture.
- **Verification:** Test for every message kind: a message one byte over the maximum is rejected.
- **State:** Met: every length is checked before allocation. Tests cover oversized and malformed payloads for the interface's messages, the decoder's and the fetcher's; not yet one test per kind.

### Requirement 4.3: A malformed message closes its sender
- **Statement:** A process that sends a malformed message, or a kind it is not allowed to send, must be closed, and every tab it served must show that it failed.
- **Reason:** A process that misbehaves may be compromised; continuing to trust it is not safe.
- **Source:** Architecture.
- **Verification:** Test for each process: a malformed message closes it and its failure is shown.
- **State:** Partly met. Every reader stops at the first malformed message and its sender is closed; a broken interface ends the application. A failed decoder or fetcher is reported only in the trusted core's log, not in its tab.
- *Amended 23 October 2026 (decision 70):* a decoder or fetcher that stopped is an error line of its channel's Log tab.

### Requirement 4.4: Every message is checked by the receiver
- **Statement:** The trusted core must check every message from every other process against the state it holds: known channel, known tab, sizes within limits, palette within 32 entries, every palette index within the palette, page numbers within 100 to 8FF.
- **Reason:** A compromised process can send well-formed but false messages.
- **Source:** Architecture.
- **Verification:** Test: messages naming an unknown tab, a wrong channel or out-of-range values are refused.
- **State:** Met for the messages that exist: frames are checked on arrival, a visibility report naming another channel's pages is filtered out, a tab index out of range is ignored.

### Requirement 4.5: Intents cannot change channel
- **Statement:** No page intent may cause a tab to navigate to another channel, change a security setting or read a cookie. Navigating must be possible only through strip input, message kinds of their own (requirement 11.1).
  *Amended 20 October 2026 (decision 41):* a cell intent may start a page-started navigation (requirement 8.2) to a link the trusted core holds in its own link table (`hyperlinks.md` section 8); it reaches another channel only through the site-change screen (requirement 8.4) and the web only through the web-browser prompt (requirement 17.8). Intents still carry no address.
- **Reason:** Page intents carry what the user did in the page area, where a page image can draw anything.
- **Source:** Decisions 4, 18 and 41.
- **Verification:** Test: every intent kind is sent with values that would leave the channel or change a setting, and none succeeds.
- **State:** Met by construction: no intent kind carries an address. A cell or a typed number names only a cell or a number; the trusted core takes any target from its own link table, and leaving goes through the site-change screen or the web-browser prompt (test: `nothing_but_a_confirmation_hands_a_web_page_over`). The hostile interface harness (requirement 3.9) drives every intent kind with arbitrary values and shows no page intent changes a tab's channel or a setting.

### Requirement 4.6: The readers are fuzzed
- **Statement:** The trusted core's readers of messages from the fetchers, the decoders, the interface, the sound helper and the broker, the interface's reader of messages and images from the trusted core, the sound helper's reader of the core's messages, and the broker's reader of the core's requests, must be fuzzed continuously.
- **Reason:** These readers are each process's only exposure to the others.
- **Source:** Architecture.
- **Verification:** Check: fuzzing targets exist for each reader and run in the full check.
- **State:** Met, 23 October 2026: `native/fuzz` has a target for each reader (`fetcher_messages`, `fetcher_commands`, `decoder_messages`, `decoder_requests`, `core_messages`, `interface_messages`, `page_image`, `sound_messages`, `sound_requests`, `broker_requests`, `broker_replies`, `updater_messages`, the readers of addresses, settings, link targets, input records, phone keys and relay messages, and of the files `history_file`, `bookmarks_file`, `session_file`, `cookie_file`, `netscape_file`, `launcher_state`, `release_manifest`, `version_mark` and `gzip_member`), and `native/tuf/fuzz` has `tuf_metadata` and `http_response`; the full check replays every input that ever found a fault and then runs each target of both for 20 seconds (`check_fuzz`). Between pushes nothing fuzzes, so "continuously" means at every push.

### Requirement 4.7: No interface request or key affects fetching
- **Statement:** No intent from the interface and no key press, however received, may cause, delay, reorder or prioritise a network request. Selecting a page may only show a page the channel's decoder already holds. **Exception (decision 23):** on a channel that publishes only separate page files, selecting a page not yet held fetches that page's file, within the tab's own channel and the limits of requirement 5.12.
- **Reason:** A hostile interface sees pages of every open channel; if it could influence fetching it could encode one channel's pages in the pattern of requests to another.
- **Source:** Decisions 20 and 22.
- **Verification:** Test with the hostile interface harness: across every intent kind and key, with arbitrary values and timing, the requests reaching a test server are identical to a run with no interface activity.
- **State:** Met by construction: the fetcher takes from the trusted core only input messages (REQ-INT-133; requirement 18.4 before 21 October 2026), which name no address, page or connection and go only to the channel's own push WebSocket; the interface's visibility reports reach only the decoder's rendering. The test is not written. Channels with only separate page files are not supported yet.
- **Amended 20 October 2026 (decision 44):** the one further exception is requirement 18.4's key records, bounded, on the channel's own WebSocket, while a page of a channel that declares input is open.

### Requirement 4.8: Untrusted text is cleaned on both sides
- **Statement:** Every piece of text in a message that came from outside the application must be cleaned (requirement 11.5) by its sender and checked again by its receiver, which refuses text that still holds a hidden character.
- **Reason:** A compromised sender must not be able to put deceptive text on screen.
- **Source:** Architecture.
- **Verification:** Test: a message carrying a hidden character is refused.
- **State:** Met for the interface's messages.

---

## 5. Network access and resource roles

### Requirement 5.1: Only teletext resources
- **Statement:** The application must fetch from the network only resources in one of these roles: teletext data (pages, bundles and pushed pages), channel descriptor, contents file, font, discovery. The updater's fetches of updates are the only exception.
- **Reason:** Limiting what is fetched limits what can attack the application.
- **Source:** Decisions 12 and 14.
- **Verification:** Test: every request the fetcher makes carries one of these roles.
- **State:** Met: the engine's user agent requests only these. The test is not written.
- *Amended 23 October 2026 (decision 66):* "font" is no longer a role: a font a channel names is never fetched, and pages are drawn in the engine's own fonts.

### Requirement 5.2: A response is read only in its role
- **Statement:** A response must be read only by the reader for the role it was requested in.
- **Reason:** A server must not be able to move its content into a different reader.
- **Source:** Decision 14.
- **Verification:** Test: a contents request answered with a descriptor, and similar crossings, are refused.
- **State:** Met by the engine's user agent; the test is not written here.

### Requirement 5.3: Teletext data is read in the decoder
- **Statement:** Responses in the teletext data role must be passed, unparsed, by the trusted core to the decoder of the channel that requested them, which must detect and read them.
- **Reason:** Detection and decoding read untrusted bytes and belong in the process with no rights.
- **Source:** Architecture.
- **Verification:** Review; test that the trusted core forwards these bytes without reading them.
- **State:** Met: the trusted core cuts page bytes into pieces and passes them on. The test is not written.

### Requirement 5.4: Descriptors, contents files and discovery are read in the fetcher
- **Statement:** Channel descriptors, contents files and discovery responses must be read by the engine's readers inside the fetcher. The trusted core must receive only typed, bounded results and must check them again.
- **Reason:** Each is untrusted input; reading it in the fetcher keeps it out of the trusted core.
- **Source:** Decision 14.
- **Verification:** Test: oversized and hostile results are refused by the trusted core.
- **State:** Met for contents files (at most 1,000 entries, every title and section name cleaned and bounded). Descriptors are read only inside the fetcher. Discovery is not built.

### Requirement 5.5: Discovery reads only link pointers
- **Statement:** Discovery must read only `Link` response headers and `<link>` elements within the first 8,192 bytes of a HyperText Markup Language response, and nothing else, and only for an address the user typed that did not answer with teletext.
- **Reason:** Discovery is the application's only contact with HyperText Markup Language.
- **Source:** Decision 14; `neoteletext/docs/web-discovery-security.md`.
- **Verification:** Test with hostile markup.
- **State:** Not met: discovery is not built.

### Requirement 5.6: Discovery results are navigations
- **Statement:** Each address found by discovery must be a page-initiated navigation, checked by the address rules, and shown on the site-change screen if it leads to a different channel.
- **Reason:** A server must not be able to send the user elsewhere without the user seeing where.
- **Source:** Decision 14.
- **Verification:** Test.
- **State:** Not met: discovery is not built.

### Requirement 5.7: Withdrawn: Fonts are fetched only when named
Withdrawn on 23 October 2026 (decision 66): no font is fetched; every page is drawn in the engine's own fonts.
- **Was:** A font must be fetched only when a page or channel names it, and used only after the checks of requirement 3.5. Teletext fonts from the network must never reach the interface process.

### Requirement 5.8: Transports
- **Statement:** The fetcher must support the HyperText Transfer Protocol version 1.1 with and without Transport Layer Security, WebSocket, Server-Sent Events and the zxnet live feed. It must not support the push mechanism of the HyperText Transfer Protocol version 2.
- **Reason:** These are the standard ways teletext services publish pages and push updates.
- **Source:** Decision 12.
- **Verification:** Test against a test server for each transport.
- **State:** Partly met: plain HyperText Transfer Protocol and Server-Sent Events, exercised against the development site; WebSocket push, plain, by `keys_round_trip_and_pages_reach_the_screen_within_budget` against `neoteletext-site terminal` (requirement 18.1); Transport Layer Security with `rustls` and Mozilla's root certificates (decision 41), checked by hand on 20 October 2026 against badssl.com, whose expired, self-signed and wrong-host certificates are refused and whose valid one is accepted; secure WebSocket not yet tried against a `wss` channel. The zxnet live feed is not wired. The only automated test of Transport Layer Security, `a_tls_connection_opens_on_its_thread_and_answers`, needs the internet and runs only with `--ignored`.

### Requirement 5.9: Plain connections are allowed and shown
- **Statement:** Channels reached without Transport Layer Security must be allowed, and the trusted strip must show that such a channel is not encrypted for as long as it is shown.
- **Reason:** Many teletext services are plain; the user must still know.
- **Source:** Decision 10.
- **Verification:** Test.
- **State:** Met: the strip shows "Not secure".

### Requirement 5.10: The local network is refused by default
- **Statement:** Addresses on this computer and the local network must be refused unless the user allows a specific address in the trusted strip's settings.
- **Reason:** A remote page must not be able to reach devices on the user's own network.
- **Source:** Architecture; `neoteletext` requirement REQ-SEC-006.
- **Verification:** Test for each range.
- **State:** Met: refused by the engine's user agent unless allowed, with `--allow-local` or in the settings panel (requirement 19.3); through a proxy, by requirement 19.11.

### Requirement 5.11: Compression is bounded
- **Statement:** The fetcher must undo compression with a fixed maximum decompressed size, the engine's constant.
- **Reason:** A small compressed response must not be able to expand without bound.
- **Source:** Architecture.
- **Verification:** Test with a highly compressible response.
- **State:** Met by the engine's user agent; the test is not written here.

### Requirement 5.12: Every response is bounded
- **Statement:** Every response must have a maximum size, a maximum time to its first byte, a maximum total time and a maximum number of redirects, each the engine's constant.
- **Reason:** No server may hold the application's resources without bound.
- **Source:** Architecture.
- **Verification:** Test for each limit.
- **State:** Met by the engine's user agent; the tests are not written here.

### Requirement 5.13: Redirects to another channel are navigations
- **Statement:** A redirect that leads to another channel must be a page-initiated navigation, shown on the site-change screen before it is followed.
- **Reason:** A redirect must not move a tab to another channel without the user.
- **Source:** Architecture.
- **Verification:** Test.
- **State:** Not met: the site-change screen is built (requirement 8.4), but a redirect that leaves the channel is only refused, by the engine's agent with `redirects_within_channel` set (the engine's REQ-WEB-021); its `RedirectToOtherChannel` event is not passed to the trusted core, so the screen is never offered.

### Requirement 5.14: No referrer and a minimal user agent
- **Statement:** The fetcher must send no `Referer` header and only the engine's minimal user agent profile. Its `User-Agent` must be `NeoTeletext/<significant version> (<operating system family>)`, as the engine's REQ-WEB-031 defines, for example `NeoTeletext/0.1 (macOS)`: honest, the version that changes compatibility only, the operating system's family and nothing more (no version of it, no architecture, nothing about the person or the computer), the same for every user of a build; on every request and on the push WebSocket's upgrade.
- **Reason:** Requests must reveal as little as possible about the user; a channel needs to know which receiver it serves, as a web site does.
- **Source:** Architecture; the owner's instruction of 21 October 2026; the engine's decision E.99.
- **Verification:** Test: inspect requests and the upgrade received by a test server.
- **State:** Met: the fetcher names the browser to the engine's agent, which sends `NeoTeletext/0.1 (macOS)` on every request and on the push upgrade (the engine's REQ-WEB-031, built 21 October 2026; test `the_user_agent_names_the_browser` here, and the engine's `the_user_agent_goes_on_requests_and_the_upgrade`).
- *Amended 21 October 2026:* the user agent names the browser, its version and its platform.

### Requirement 5.15: Whole-channel fetching in a fixed order
- **Statement:** When a channel opens, its pages must be fetched in an order fixed by the channel's own data: the whole bundle if there is one; every page file listed in the contents file, in its order, if there is one; otherwise the initial page, with further pages fetched when selected (decision 23). Fetching must stop at the engine's maximum channel size and proceed at its request rate.
- **Reason:** Fetching that does not depend on viewing reveals nothing about viewing.
- **Source:** Decision 22.
- **Verification:** Test: the request sequence is identical whatever pages are viewed.
- **State:** Met for channels with a contents file, by the engine's user agent (measured: 218 of 800 pages in 45 seconds, one request each 0.2 seconds). File channels are fed at the same pace whatever is on screen. The test is not written.

### Requirement 5.16: Refreshing on a fixed schedule
- **Statement:** A channel's pages must be refreshed only by its declared schedule, a fixed default, or its push feed, never according to which page is shown.
- **Reason:** View-driven refreshing reveals and can signal what is viewed.
- **Source:** Decision 22.
- **Verification:** Test.
- **State:** Partly met: push is the only refresh, and it does not follow viewing. Schedules are not built.

### Requirement 5.17: A closed tab's channel keeps refreshing for a grace period
- **Statement:** When the last tab on a channel closes, that channel must keep refreshing for a fixed grace period, the engine's constant, before its fetcher and decoder stop.
- **Reason:** Otherwise the moment a tab closes is visible to the channel's server.
- **Source:** Decisions 20 and 22.
- **Verification:** Test.
- **State:** Not met: closing a tab stops its fetcher at once. The constant is to be added in neoteletext.

---

## 6. Content detection

### Requirement 6.1: The declared media type is never used for acceptance
- **Statement:** The `Content-Type` a server declares must never be used to accept or reject a resource.
- **Reason:** Teletext files are served with any media type.
- **Source:** Decision 13.
- **Verification:** Test.
- **State:** Met: page bytes go to the decoder whatever their declared type.

### Requirement 6.2: Content decides
- **Statement:** Responses in the teletext data role must be recognised by the engine's content detection (`neoteletext` requirement REQ-INP-005), in the decoder.
- **Reason:** Only content can be relied on.
- **Source:** Decision 13.
- **Verification:** Test with each supported format.
- **State:** Met: the decoder reads through the engine's `AutoReader`; tests cover page files and recordings.

### Requirement 6.3: Unrecognised content is refused, never shown
- **Statement:** A response that detection does not recognise must produce no page and must be shown only as a refusal stating the HyperText Transfer Protocol status.
- **Reason:** Web error pages, images and documents are not teletext.
- **Source:** Decisions 12 and 13.
- **Verification:** Test with markup, images and random bytes.
- **State:** Partly met: unrecognised bytes produce no page. No refusal is shown yet.

### Requirement 6.4: Detection is deterministic
- **Statement:** Detection's decision must not depend on how the network split the data.
- **Reason:** The same bytes must always be read the same way.
- **Source:** Architecture; `neoteletext` requirement REQ-INP-001.
- **Verification:** Test with the same input split at every possible point.
- **State:** Met by the engine; the test is not written here.

### Requirement 6.5: A misbehaving live feed is closed
- **Statement:** A live feed that produces malformed packets beyond a fixed threshold, the engine's constant, must be closed and reported to the user.
- **Reason:** A hostile feed must not keep the decoder busy indefinitely.
- **Source:** Architecture.
- **Verification:** Test.
- **State:** Not met: live feeds are not built.

---

## 7. The security model of a tab

### Requirement 7.1: Origins
- **Statement:** An origin must be scheme, host and port, as the engine defines it.
- **Source:** Decision 9.
- **Reason:** The same definition traditional browsers use.
- **Verification:** Test.
- **State:** Met by the engine.

### Requirement 7.2: The channel is the unit of isolation
- **Statement:** A channel must be identified by its origin and folder path (`neoteletext` decision E.70), and must be the unit for cookies, caches, decoders, fetchers and the site-change screen.
- **Reason:** The channel plays the part a site plays in a traditional browser.
- **Source:** Decision 9.
- **Verification:** Test.
- **State:** Met: every channel has its own decoder and fetcher, and its own cookie jar in the trusted core (decision 62, requirement 21.1).

### Requirement 7.3: Channels never share data
- **Statement:** A channel's pages, cookies, caches and history must never be visible to, mixed with or sent to another channel's decoder or requests.
- **Reason:** The equivalent of the same-origin policy.
- **Source:** Decision 9.
- **Verification:** Test with two channels whose pages and cookies use identical names.
- **State:** Met by construction (separate processes per channel); the test is not written.

### Requirement 7.4: Tabs on the same channel share
- **Statement:** Tabs showing the same channel must share that channel's cookies and decoder.
- **Reason:** The same behaviour as same-site tabs in a traditional browser.
- **Source:** Decision 11.
- **Verification:** Test.
- **State:** Partly met, 22 October 2026: tabs on a channel share its cookies, one jar in the trusted core (requirements 21.1 and 21.2, decision 62; `each_fetcher_is_answered_with_its_channels_cookies`); every tab still opens its own decoder and fetcher (`Session::open_tab`).

### Requirement 7.5: The trusted core owns the cookie store
- **Statement:** The cookie store must live in the trusted core. For each request the trusted core must give the fetcher only the cookies that request carries, and the fetcher must return any `Set-Cookie` headers to the trusted core.
- **Reason:** A compromised fetcher must see only the cookies of requests in flight.
- **Source:** Decision 4.
- **Verification:** Test: the fetcher process holds no cookie store.
- **State:** Met, 22 October 2026 (decision 62, requirement 21.1): the trusted core holds every cookie, one jar for the ordinary tabs and one for the private, each channel's under its own tag; a fetcher holds none (`the_fetcher_keeps_no_cookie`, its agent's jar `CookieJar<0, 0>`), asks the trusted core for each request's `Cookie` header and returns each `Set-Cookie` line (`each_fetcher_is_answered_with_its_channels_cookies`, `the_agent_asks_for_each_requests_cookies`), through the engine's E.104.

### Requirement 7.6: Cookie rules
- **Statement:** The cookie store must follow the engine's: the draft RFC 6265bis, every cookie scoped to its host only, `Secure` cookies never sent over plain connections, the `__Host-` and `__Secure-` prefixes and expiry enforced, and `SameSite` applied to form submissions that go to another channel.
- **Reason:** The same cookie behaviour as a traditional browser, made stricter.
- **Source:** Decision 9.
- **Verification:** Test for each rule.
- **State:** Met by the engine's store.

### Requirement 7.7: No third-party context
- **Statement:** No page may cause a request that carries credentials to an origin other than its channel's.
- **Reason:** Third-party cookies are a tracking and attack route.
- **Source:** Decision 9.
- **Verification:** Test.
- **State:** Met: a fetcher fetches only its own channel.

### Requirement 7.8: Caches are keyed by channel
- **Statement:** The decoder's page store, the trusted core's record of frames, and any later cache must be keyed by channel.
- **Reason:** Caches must not leak data between channels.
- **Source:** Decision 9.
- **Verification:** Test.
- **State:** Met for what exists.

### Requirement 7.9: The interface process holds no channel data
- **Statement:** The interface process must hold no cookies and no storage of any kind, and must keep a channel's pages only while that channel is shown.
- **Reason:** The interface process is untrusted and must not accumulate anything of value.
- **Source:** Decisions 4 and 33.
- **Verification:** Test: the interface process writes no file.
- **State:** Partly met: it writes no file (denied by its sandbox; on Windows its restricted token at low integrity denies writes to the user's files) and holds no cookies; it drops the image of every page its layout no longer shows each time it reports what is visible (`retain_pages` in `native/interface/src/main.rs`), so it holds only the shown channel's pages. Neither is tested.

### Requirement 7.10: Forms
- **Statement:** A page's form must be sent only to its declared destination, through the fetcher. A form whose destination is another channel must go through the site-change screen, with that channel's `SameSite` cookies withheld. No form value may be filled in automatically.
- **Reason:** The protection traditional browsers give against cross-site form submission.
- **Source:** Decision 9.
- **Verification:** Test.
- **State:** Not met: forms are not built. They need text fields in the page area.

### Requirement 7.11: Certificate failures
- **Statement:** A Transport Layer Security certificate that fails its checks must stop the navigation, with no way for the user to proceed.
- **Reason:** A failed certificate means the connection cannot be trusted.
- **Source:** Decisions 4 and 24.
- **Verification:** Test with expired, self-signed and wrong-host certificates.
- **State:** Met, 20 October 2026 (decision 41): a certificate that fails `rustls`'s checks against Mozilla's roots ends the handshake and fails the connection, with nothing offered to proceed (`open` in `native/fetcher/src/transport.rs`); by hand against badssl.com, whose expired, self-signed and wrong-host certificates were refused (requirement 5.8). The test with such certificates is not written; `a_tls_connection_opens_on_its_thread_and_answers` (`--ignored`, needs the internet) shows only a valid one accepted.

### Requirement 7.12: Local files
- **Statement:** Each local file must be its own channel with no cookies, read by the trusted core and decoded by its own decoder, and links from it to web addresses must go through the site-change screen.
- **Reason:** A file must not borrow the identity or cookies of a channel.
- **Source:** Architecture.
- **Verification:** Test.
- **State:** Met, 20 October 2026: a file is its own channel with its own decoder and no fetcher or cookies, at most 64 megabytes (`MAX_SOURCE_BYTES`) or 4,096 page files (`MAX_FILES`, `native/core/src/source.rs`); its links to its own pages open at once, and those to a web channel or the web leave it through the site-change screen or the web-browser prompt (`hyperlinks.md` section 11 item 4, decision 41; `a_file_channels_links_to_the_web_leave_it`).

### Requirement 7.13: A refused certificate is explained
- **Statement:** When a connection of a tab's channel is refused because of its certificate, the tab's page area must show the certificate-error screen in place of the channel's pages: the host as the trusted strip shows it, never a name from the certificate; what failed, in the trusted core's words, chosen from a fixed list by the kind of failure; that there is no way past it; and Try again, Go back (when the tab has history) and Close tab, with nothing that opens the channel anyway. The strip must mark the channel refused while the screen shows. The screen must go when the channel's contents next arrive or the tab goes elsewhere.
- **Reason:** A refused channel looked like a slow one, and the reason reached only the log; a person must be told why a channel will not open, without being offered a way past the refusal (decision 24).
- **Source:** Decision 83 (`certificate-errors.md`).
- **Verification:** Tests: the kinds told apart from the TLS library's errors; a failed handshake reported once; the strip and the Log told once; the screen's words and buttons.
- **State:** Met, 23 October 2026: `certificate_failures_are_told_apart`, `a_failed_handshake_is_reported_once`, `a_refused_certificate_shows_until_the_contents_arrive`, `a_refused_certificate_shows_why_and_no_way_past`, `the_screens_buttons_are_reload_back_and_close`. Not yet seen against a channel whose certificate has expired.

---

## 8. Navigation and the site-change screen

### Requirement 8.1: One navigation history per tab
- **Statement:** The trusted core must keep one flat history list per tab.
- **Reason:** Navigation state is security state and belongs in the trusted core.
- **Source:** Architecture.
- **Verification:** Test.
- **State:** Met, with a test. (The earlier requirement named the engine's `neoteletext-browser`, which decision 36 no longer uses.)

### Requirement 8.2: User-started and page-started navigations are distinguished
- **Statement:** Navigations the user starts in the trusted strip (typed address, bookmark, Back, Forward, Home, new tab) and navigations a page starts (Fastext, page links, redirects, discovery results) must be recorded as such and handled under their own rules.
- **Reason:** Navigations a page starts are less trusted.
- **Source:** Architecture.
- **Verification:** Test.
- **State:** Partly met: page links are recorded as page-started in the tab's history (test: `a_link_within_the_channel_keeps_it_open_and_is_recorded_as_the_pages`), and leave only through the site-change screen; a Fastext key to a service the descriptor lists leaves the same way (`fastext_keys_follow_as_links_do`, decision 67). No rule reads the record yet. Redirects (requirement 5.13) and discovery results are not built.

### Requirement 8.3: The committed address is shown
- **Statement:** The trusted strip must show the committed address until a new page commits.
- **Reason:** A pending navigation must not be presented as the current one.
- **Source:** Architecture.
- **Verification:** Test.
- **State:** Met: the strip shows the tab's current history entry, and a refused address leaves it unchanged.

### Requirement 8.4: Leaving a channel needs the site-change screen
- **Statement:** Every navigation that leaves the tab's channel, other than one the user started in the trusted strip, must show the site-change screen in the strip with the destination channel and how the navigation was started, and must proceed only on the user's confirmation there.
- **Reason:** The user must always see and agree to a change of channel.
- **Source:** Decision 9.
- **Verification:** Test for each kind of page-started navigation.
- **State:** Partly met: for links, built and seen on macOS on 20 October 2026; for Fastext, built 23 October 2026 (decision 67; `fastext_keys_follow_as_links_do`); for redirects (requirement 5.13) and discovery results, not built.

### Requirement 8.5: The site-change screen cannot be faked by a page
- **Statement:** The site-change screen must be drawn only in the trusted strip, never in the page area, and a confirmation must not be accepted from a click inside the page area or from pasted input.
- **Reason:** A page image can draw anything.
- **Source:** Decisions 4 and 24.
- **Verification:** Test; review.
- **State:** Met for links: the screen is drawn only in the strip, and confirmed only by strip input of its own kind, never a click in the page area, not within half a second of appearing, and by Enter only once the user moved the focus to it (tests in `hyperlinks-security.md` section 15).

### Requirement 8.6: Cancelling returns to the committed page
- **Statement:** Cancelling the site-change screen must return the tab to its committed page unchanged.
- **Reason:** Declining must have no effect.
- **Source:** Architecture.
- **Verification:** Test.
- **State:** Met for links: cancelling closes the question and changes nothing else (tests: `leaving_and_the_web_are_asked_about_and_answered_only_after_half_a_second`, `nothing_but_a_confirmation_hands_a_web_page_over`).

---

## 9. Rendering and showing pages

### Requirement 9.1: Pages are rendered in the decoder
- **Statement:** Pages must be rendered by the decoder into 8-bit palette indices at the native raster, 480 by 500, with a palette of at most 32 colours.
- **Reason:** Rendering untrusted pages belongs in the process with no rights.
- **Source:** Decision 6.
- **Verification:** Test.
- **State:** Met, with tests. Side panels, which widen the raster, are not supported yet.

### Requirement 9.2: Frames are checked before encoding
- **Statement:** The trusted core must check every frame: known channel and page, the exact raster size, a palette of at most 32 entries, every index within the palette.
- **Reason:** A compromised decoder can send false frames.
- **Source:** Architecture.
- **Verification:** Test with frames breaking each rule.
- **State:** Met, with tests in `protocol::decoder`.

### Requirement 9.3: Frames are lossless indexed images
- **Statement:** The trusted core must encode each frame as an indexed Portable Network Graphics image with the engine's writer, and the interface process must read only such images: indexed, 8 bits a dot, exactly 480 by 500, at most 32 colours, with a bound on the memory its decoding may take.
- **Reason:** Colours must be exact, and the interface's image reader must see only files our encoder produced.
- **Source:** Decisions 5 and 6.
- **Verification:** Test: decoding the image gives back the exact indices and palette; any other image is refused.
- **State:** Met, with tests.
- *Amended 22 October 2026 (decision 61):* the trusted core must encode each frame in `protocol::frame`'s form (its palette of at most 32 colours, its screen colour, and its dots raw, in runs, or as changes against the frame of the same page the interface holds, whichever is smallest), and the interface must read only that form: the screen colour and every dot within the palette, the dots filling 480 by 500 exactly, changes only against the version it holds. Met: `every_form_reads_back_exactly`, `a_broken_image_is_refused`, `a_frame_is_sent_as_its_changes_against_the_last`; the fuzz target `page_image` (every bound, round trip). The `png` crate is gone from the interface.

### Requirement 9.4: Unchanged frames are not sent again
- **Statement:** The trusted core must not send a frame identical to the last one sent for that page.
- **Reason:** Many pages on screen must cost only what changes.
- **Source:** Decision 7.
- **Verification:** Test.
- **State:** Met, with a test.

### Requirement 9.5: Only visible pages are rendered and sent
- **Statement:** The interface must report which pages its layout puts on screen, and the trusted core must have only those rendered and sent, of the active channel only.
- **Reason:** A wall of hundreds of pages must cost what is visible.
- **Source:** Decision 7.
- **Verification:** Test: a wall of 800 pages with 20 visible causes frames for those 20 only.
- **State:** Met; tests cover the report and the trusted core's filtering separately.

### Requirement 9.6: Whole-frame replacement
- **Statement:** Each new frame must replace the page's whole image, and the interface must keep showing the previous image until the new one is read completely.
- **Reason:** Simple, self-contained frames, with no blank frame and no memory growth.
- **Source:** Decision 8.
- **Verification:** Test: no blank frame during rapid updates; memory stays bounded over a long run.
- **State:** Met; the long-run memory test is not written.
- *Amended 22 October 2026 (decision 61):* a frame may be sent as its changes against the last frame of the page, which the interface holds; the interface still replaces the page's image whole, once the frame is read and applied, and a frame of changes to a page it no longer holds is dropped, since the trusted core sends a page whole when it is shown again.

### Requirement 9.7: Flash follows the engine's timing
- **Statement:** Flash must be rendered by the decoder at the engine's flash timing (one second in twelve units, its decision E.48), for shown pages that have flashing cells only, and only when their flash phase changes, by the decoder's own clock; and step 3 of `architecture.md` section 10 must pass on only the frames that changed. A shown page is otherwise drawn again only when the engine reports that its reception moved, or that data every page may draw from changed.
- **Reason:** The engine defines flash; rendering only what changes keeps it cheap.
- **Source:** Architecture; decision 47. (The earlier requirement had the trusted core alternate two stored phases; rendering in the decoder was measured at about 7 seconds of processor time a minute for the shown channel.)
- **Verification:** Test.
- **State:** Met, 21 October 2026: tests `only_a_shown_page_that_flashes_wakes_the_decoder_and_only_when_its_phase_changes`, `a_message_reports_only_the_pages_it_touched`, `arrivals_compare_only_the_pages_that_completed` (one comparison for one page, where scanning the store made 140 of 200). Measured by driving the release decoder for 10 seconds with one shown page: a steady page took 0.15 s of processor time when the trusted core ticked it twelve times a second, and 0.01 s on the decoder's own clock; a flashing page 0.21 s and 0.24 s, drawing 20 and 21 frames, two a second.
- *Amended 21 October 2026 (decision 47):* drawn by the decoder's own clock, for flashing pages only and at their phase changes; the trusted core no longer ticks decoders.
- *Amended 22 October 2026 (decision 53):* at the phase changes of the rates the page's cells use, not of every rate: a page with slow flash alone is drawn twice a second, not seven times (the test above, which failed on the earlier code; under eight flashing pages the decoder took 0.7 to 1.1 % of a processor against 6.3 %).

### Requirement 9.8: Scaling and pixel shape in the drawing transform
- **Statement:** Pages must be scaled, and their pixel shape corrected, by the interface when it draws them, never by rendering again at other sizes.
- **Reason:** One raster per page keeps rendering simple and exact.
- **Source:** Architecture.
- **Verification:** Review.
- **State:** Partly met: pages are scaled with nearest-neighbour sampling; the 4:3 pixel-shape correction is not applied yet.

### Requirement 9.9: Frame size is measured
- **Statement:** The size and encoding time of page images must be measured on real channels before the first release and recorded.
- **Reason:** The design assumes a few kilobytes per page.
- **Source:** Architecture.
- **Verification:** Recorded measurement.
- **State:** Partly met: about 6 kilobytes a page on the stand-in engine (`prototypes/docs/proofs.md` section 4). On 22 October 2026, on 800 pages of a recorded channel (decision 61): 16.7 kilobytes a page on average in runs, 29 at the 95th percentile, encoded in 156 microseconds and read in 55; a flashing page's next phase 2.1 kilobytes as changes, encoded in 118 microseconds and read in 8. Not yet measured on a live channel.

### Requirement 9.10: The Colour Map of Levels 2.5 and 3.5
- **Statement:** Pages must be shown in the Colour Map of EN 300 706 clause 12.4 as the page's packets define it: CLUTs 2 and 3 from X/28/0, else M/29/0; at Level 3.5, CLUTs 0 and 1 from X/28/4, else M/29/4, never CLUT 1 entry 0, which stays transparent; with the default row colour, black background colour substitution and colour table re-mapping of Table 4, and the colours that X/26 triplets select.
- **Reason:** Decision 6 is the Colour Map of Level 2.5, and the owner asked for both levels exactly as specified.
- **Source:** Decisions 6 and 60.
- **Verification:** Test: a page's X/28/0 and X/28/4 reach the frame's palette and its dots; the engine's own tests of each rule.
- **State:** Met, 22 October 2026: `a_page_redefines_its_colour_map` (decoder: X/28/0 redefining CLUT 2 with re-mapping 5 drawing red from CLUT 2, X/28/4 redefining CLUT 0, CLUT 1 entry 0 left transparent), which fails without the engine's `level35-32colors` (its `colours32` until E.105); the engine's `colour_map_from_page_packets`, `screen_and_row_colours`, `remapping_and_black_background_substitution`, `colour_triplets_set_colour_map_indices` and `colour_map_sources_and_precedence` (run in the engine's gate with every feature). Table 4's re-mapping read against the engine's table, row for row. Where the standard does not settle which of X/28/4 and M/29/0 wins a field, the page's own packet does (the engine's decision E.44).

### Requirement 9.11: The screen colour is shown
- **Statement:** Each page must be drawn inside a border of its screen colour (Table 4's Default Screen Colour, or clause 12.3's Full Screen Colour from X/26), one character cell wide at the sides, and links and cells must be found in the page's image, not its border.
- **Reason:** The standard colours the screen area outside the page's rows; without a border nothing shows it.
- **Source:** Decision 60.
- **Verification:** Test.
- **State:** Met: the decoder sends the engine's resolved screen colour with each frame (`write_frame_to_core`, refused when outside the palette), the interface keeps it with the page (`a_page_with_an_index_outside_its_palette_is_refused`), and every place a page is drawn draws it (`image_within`, `draw_page_on_screen`); the wall's tests find cells and the zoomed page within the image.

---

## 10. The interface process

**These requirements keep the interface process as small and as safe as the trusted core's readers,** because it draws the trusted strip (requirement 3.9).

### Requirement 10.1: No network byte reaches the interface process
- **Statement:** The interface process must receive pages only as images the trusted core encoded, and text only as cleaned, bounded text the trusted core sends. No byte received from the network or read from a file may reach it.
- **Reason:** This keeps the interface's readers away from attacker-controlled input.
- **Source:** Decisions 4 and 33.
- **Verification:** Review of every message type sent to the interface process.
- **State:** Met.

### Requirement 10.2: Page text is drawn only as text
- **Statement:** Text that came from a page, a channel or a server must be drawn only as text runs, never interpreted.
- **Reason:** There is no markup to inject into; this keeps it so.
- **Source:** Decision 4.
- **Verification:** Review.
- **State:** Met.

### Requirements 10.3 to 10.7: Withdrawn
- **Were:** a fixed list of run-time JavaScript packages; forbidden script interfaces; a Content Security Policy; locked webview navigation; Tauri capabilities naming webview labels.
- **Why withdrawn:** there is no webview, no script and no Tauri (decision 33).

### Requirement 10.8: No debugging interface in release builds
- **Statement:** Release builds must contain no debugging interface reachable from outside the process.
- **Reason:** A debugging port gives control of the interface to anything that reaches it.
- **Source:** Decision 4; prototype finding (Chromium Embedded Framework trial).
- **Verification:** Review of the release build.
- **State:** Met: there is none.

### Requirement 10.9: Dropped files go to the trusted core
- **Statement:** Files dropped on a window must be handled by the trusted core, which opens them through a decoder. The interface process must never read them.
- **Reason:** A dropped file is untrusted input.
- **Source:** Architecture.
- **Verification:** Test.
- **State:** Not met: dropping files is not built.

### Requirements 10.10 and 10.11: Withdrawn
- **Were:** a check of the Linux webview's version; page code kept off the network.
- **Why withdrawn:** there is no webview and no page code (decision 33).

### Requirement 10.12: No unsafe code
- **Statement:** The interface process, like every crate of the application, must forbid `unsafe` code in its own source. There are three exceptions: the `dock-menu` crate, whose macOS module reaches AppKit through the Objective-C runtime; the `web-opener` crate, whose Windows module calls the shell's `ShellExecuteW` (hyperlink decision 7); and the `confine` crate, whose Linux module makes C library calls for the network namespace (`getuid`, `getgid`, `unshare`) and no-new-privileges (`prctl`), whose Unix module passes pipes between processes as descriptors with `sendmsg` and `recvmsg` (decision 56) and reads the local zone with `tzset` and `localtime_r` (decision 57), and whose Windows module makes the Win32 calls that create a confined process and hand it pipes (decisions 52 and 56). Each denies `unsafe_code` everywhere else, and each unsafe block states why it is sound.
- **Reason:** A memory-safety fault is the most likely way to take over a process. AppKit offers the Dock menu only through calls Rust cannot check (decision 39).
- **Source:** Architecture; decision 39.
- **Verification:** Check: the workspace lints forbid `unsafe_code`; `dock-menu` denies it and allows it only in `src/mac.rs`, `web-opener` only in `src/windows.rs`, and `confine` only in `src/linux.rs`, `src/unix.rs` and `src/windows.rs`, each with Clippy's `undocumented_unsafe_blocks`.
- **State:** Met; amended 22 October 2026 (decisions 52 and 56) for `confine`.
- *Amended 23 October 2026 (decision 75):* a fourth exception, the `os-install` crate, whose macOS module calls CoreFoundation's alert for the launcher, which has no window, and whose Windows module calls `WinVerifyTrust` and reads the signer's certificate, writes and removes the per-user uninstall entry, and shows a message box (`installation-and-updates.md` sections 4 and 5). It denies `unsafe_code` everywhere else and allows it only in `src/macos.rs` and `src/windows.rs`. `confine`'s Windows module also grants `ALL APPLICATION PACKAGES` read on the programs' folder and removes the containers' profiles. The launcher, the updater, the installer, the stub and the release tool forbid unsafe code; the folder exchange is `rustix`'s safe interface.

### Requirement 10.13: The interface process ends on a broken message
- **Statement:** The interface process must end at the first malformed message from the trusted core, rather than try to continue.
- **Reason:** A broken stream means a fault somewhere; continuing could show something false.
- **Source:** Architecture.
- **Verification:** Test.
- **State:** Met.

---

## 11. The trusted strip

### Requirement 11.1: The trusted strip and strip input
- **Statement:** Every window must have a trusted strip, drawn by the interface process in that window, outside the page area, showing only the state the trusted core sends. It must never be a window of its own. Its input must reach the trusted core as strip input, message kinds that page intents never use.
- **Reason:** A strip in a window of its own can be covered by any other window (`prototypes/docs/proofs.md` section 3); in the page area a page image could imitate it.
- **Source:** Decisions 4, 18 and 33.
- **Verification:** Test: no page intent produces a navigation; review of every layout.
- **State:** Met.

### Requirement 11.2: What the trusted strip holds
- **Statement:** The trusted strip must hold the address field, the channel's identity, whether the channel is encrypted, Back, Forward, Reload and Home, the site-change screen, and every setting that affects security.
- **Reason:** Everything that decides or shows trust is in one place.
- **Source:** Decision 4.
- **Verification:** Review.
- **State:** Met, 23 October 2026: the address, identity, encryption state, Back, Forward, Reload (requirement 11.6, decision 63) and Home, the bookmark star, the site-change screen (requirements 8.4 and 8.5, for links and Fastext) and the security settings in the settings panel (decision 51).
- *Amended 23 October 2026 (decision 63):* Reload is built (requirement 11.6), between Forward and Home. State: partly met; the site-change screen is not built.

### Requirement 11.3: The address field
- **Statement:** In display mode the address field must show the address as formatted by the address rules, with the host emphasised and internationalised names shown safely. In edit mode its text must be sent, on Enter, to the trusted core, which opens it or says why it cannot.
- **Reason:** The address is the user's main security indicator.
- **Source:** Decision 9.
- **Verification:** Test with deceptive and internationalised addresses.
- **State:** Partly met: the host is emphasised; addresses with a user name or hidden characters are refused; internationalised hosts are refused until they can be shown safely. Why an address was refused is written to the log, not shown in the strip.

### Requirement 11.4: Always visible
- **Statement:** The trusted strip must be visible in every layout, including the zoomed page. Where a full-screen layout hides it, it must reappear on any interaction.
- **Reason:** A hidden indicator protects nothing.
- **Source:** Decisions 4 and 24.
- **Verification:** Review of every layout.
- **State:** Met: no layout hides it; there is no full-screen layout yet.

### Requirement 11.5: Untrusted text is cleaned before display
- **Statement:** Text from pages and channels (titles, section names, notices, addresses) must have control characters, bidirectional controls and zero-width characters removed and be cut to a fixed length before it is shown anywhere.
- **Reason:** These characters can make text read as something it is not.
- **Source:** Decision 4.
- **Verification:** Test.
- **State:** Met, with tests: `protocol::clean` on the sending side and a check on the receiving side; the address editor never inserts hidden characters.

### Requirement 11.6: Reload
- **Statement:** The strip must show a Reload button between Forward and Home; it, Command-R on macOS and Control-R elsewhere must open the active tab's channel afresh, with a new decoder and fetcher reading its pages again, at the tab's place in its history, which must not change. Reload is strip input carrying nothing; the trusted core decides what is opened.
- **Reason:** A channel that stalls, or a file that changed, is otherwise opened again only by typing its address, which adds to the tab's history.
- **Source:** Decision 63.
- **Verification:** Trusted-core test: a reload gives the tab a new channel at the same address and leaves its history as it was. Interface tests of the button, its place and the shortcuts.
- **State:** Built 23 October 2026 (`ToCore::Reload`, `Session::reload`), and seen on macOS: the strip shows Back, Forward, Reload and Home; the button sends Reload (`a_tip_shows_after_a_rest_and_goes_on_a_press`); `a_reload_opens_the_channel_afresh_where_the_tab_is` (a new channel at the same address, the history as it was, a reload waiting to be recorded); in history a reload counts no visit (requirement 20.1). The button's place and the shortcut: `reload_sits_between_forward_and_home_and_has_its_keys`.
- *Amended 23 October 2026 (decision 64):* entering the address shown in the address field, or following a link to it, is a reload, as Chrome makes it (`chrome.history`'s reload transition: "pressing Enter in the address bar"; `ShouldTreatNavigationAsReload`): no new entry, and not a typed visit. State: met, `a_reload_opens_the_channel_afresh_where_the_tab_is`.

---

## 12. Window features

### Requirement 12.1: Tabs
- **Statement:** The interface must draw a tab strip with close buttons and a new-tab button; the list and order of tabs, their creation and closing, and session restore must be decided by the trusted core.
- **Reason:** The tab strip is drawn by the interface; tab state is trusted.
- **Source:** Decisions 33 and 35.
- **Verification:** Test.
- **State:** Met, 23 October 2026: tabs, close, new tab, closing by the middle button (`the_middle_button_closes_the_tab_under_it`, decision 63), dragging to reorder and overflow (requirements 12.13 to 12.15, decision 69), session restore (requirement 20.22, decision 59), and a tab dragged off the strip into a window of its own or onto another window's strip (requirement 12.8, decision 72; `a_tab_dragged_off_the_strip_leaves_the_window`).
- *Amended 23 October 2026 (decision 63):* a press of the middle button on a tab must close it, as the tab's close button does. State: built, `the_middle_button_closes_the_tab_under_it`. Session restore has been built since 22 October 2026 (requirement 20.22).
- *Amended 23 October 2026 (decision 69):* dragging to reorder and overflow are built (requirements 12.13 to 12.15); a tab dragged between windows waits on multiple windows (requirement 12.8).

### Requirement 12.2: Page walls and transitions
- **Statement:** The interface must show the window of decision 35 (banks, page list, page area, overview), with its transitions, and must respect the platform's reduced-motion preference.
- **Reason:** Navigating many pages at once is the purpose of the interface.
- **Source:** Decisions 7 and 35.
- **Verification:** Review; test with reduced motion.
- **State:** Partly met: the window and its transitions are built. Reduced motion is not read from the platform.

### Requirement 12.3: Clicks on pages are cell clicks
- **Statement:** A click on a page must be converted to a cell (column and row) from the pointer position and the rectangle the page was drawn in, and sent as an intent, with the version of the frame it was made on. The trusted core must decide, from its own link table, whether the cell is a link (`hyperlinks.md` section 8; decision 41).
- **Reason:** A page image can draw fake buttons; the click must mean only "this cell".
- **Source:** Decision 4.
- **Verification:** Test under every layout.
- **State:** Met: a click carries the version of the image it was made on (test: `a_click_names_the_image_it_was_made_on`), and the trusted core follows links by its own rules.

### Requirement 12.4: Keys work in the tab
- **Statement:** With the page area focused, page-number keys, Fastext colour keys, reveal, hold and mix must work. A key must only select among pages already held (requirement 4.7). A key that would leave the channel must go through the site-change screen. A page number the page links with `LK`, or the descriptor aliases, follows that link (`hyperlinks.md` section 6; decision 41).
- **Reason:** Teletext is driven by these keys.
- **Source:** Decision 22.
- **Verification:** Test each key.
- **State:** Met, 23 October 2026: three digits choose a page of the channel, or follow the open page's `LK` line or the channel's alias (test: `a_number_the_page_or_channel_links_goes_to_the_trusted_core`), Up and Down step through pages, and Fastext, reveal, hold and mix are built (requirements 22.1 to 22.4, decision 67: `the_viewers_keys_ask_for_the_controls`, `fastext_keys_follow_as_links_do`, `reveal_and_mix_change_the_page_drawn`, `a_held_page_keeps_its_version`).

### Requirement 12.5: Menus and shortcuts
- **Statement:** The application must provide a native menu bar (File, Edit, View, History, Window) carrying the browser's shortcuts. Menu items and buttons must send the same intents.
- **Reason:** Shortcuts must work whatever has focus.
- **Source:** Prototype finding.
- **Verification:** Test each shortcut.
- **State:** Partly met, 23 October 2026: the menu button's menu is built (`the_menu_holds_history_and_bookmarks`), with New tab, New window, New private tab, New private window, Move tab to new window, Reopen closed tab, History, Bookmarks, the bookmarks bar, Bookmark this page, the phone keyboard, the developer console, the remote control and Settings, each with its shortcut where it has one (decisions 57, 68 and 72); the native menu bar is not built.
- *Amended 22 October 2026 (decision 57):* until the menu bar is built, the menu button's menu holds New tab, New private tab, Reopen closed tab, History, Bookmarks, the bookmarks bar, Bookmark this page, the phone keyboard, the developer console and Settings, each with its shortcut; the History and Bookmarks menus hold what `history-and-bookmarks.md` section 6 lists. State: the menu button's menu is built (`the_menu_holds_history_and_bookmarks`); the menu bar is not.

### Requirement 12.6: Internal pages
- **Statement:** Bookmarks, history and settings must be shown from data the trusted core supplies, except settings that affect security, which must be changed only in the trusted strip.
- **Reason:** Security settings must stay in the strip.
- **Source:** Decision 4.
- **Verification:** Test.
- **State:** Met, 22 October 2026: settings in the strip's panel, security settings changed only there (decision 51); history in the panel of requirement 20.4 and bookmarks in the manager of requirement 20.17, both from the trusted core's views (decision 57).
- *Amended 22 October 2026 (decision 57):* history has the panel of requirement 20.4 and bookmarks the manager of requirement 20.17, both from the trusted core's views.

### Requirement 12.7: Developer tools
- **Statement:** A docked developer tools panel must show, for the current channel: the packet log, the pages, network requests and notices, all from the trusted core and shown as text; never a cookie (amended 21 October 2026, decision 51; requirement 19.8).
- **Reason:** Diagnosis without exposing the trusted core.
- **Source:** Architecture.
- **Verification:** Test that hostile packet contents appear as text.
- **State:** Met by requirements 19.8 and 19.9 (decision 51).
- *Amended 23 October 2026 (decision 63):* the panel also manages cookies, in a tab of its own showing the rows of requirement 21.6; it still never shows a cookie's name or value.

### Requirement 12.8: Windows
- **Statement:** Each window must have one trusted strip. Moving a tab between windows must be a change of state in the trusted core. Frames for a page must go only to the window showing it.
- **Reason:** Windows share one trusted state.
- **Source:** Architecture.
- **Verification:** Test.
- **State:** Met (23 October 2026, decision 72, `multiple-windows.md`): one interface process draws every window, each with its own strip from what the trusted core sent for it; a tab moves between windows only as strip input the trusted core carries out (`TabToWindow`); a channel's frames go only to its tab's window (`Target`). Tests: `windows_keep_their_own_tabs`, `a_closed_window_reopens_whole`, `every_window_is_kept_and_restored`, `tabs_are_bounded_across_windows`, `messages_and_intents_name_their_window`, `a_tab_dragged_off_the_strip_leaves_the_window`, `window_messages_are_held_to_their_bounds`; by hand on macOS, a second window opened, closed and was restored.

### Requirement 12.9: Accessibility
- **Statement:** The window must be usable with a screen reader on every desktop, through AccessKit or the platform's own interface, with each page labelled with its cleaned text.
- **Reason:** Teletext must be readable without sight.
- **Source:** Architecture.
- **Verification:** Test with VoiceOver, Narrator and Orca.
- **State:** Not met.

### Requirement 12.10: Updated pages are shown
- **Statement:** When a page the channel already delivered arrives again with different content, the window must show that it changed, in the overview, the page list and the page area, and for how long ago.
- **Reason:** A live channel's value is in its changes; the user must see which pages are live.
- **Source:** Owner's request of 15 October 2026 (`native-interface-design.md` section 12.3).
- **Verification:** Test.
- **State:** Met, with tests of the decoder's change detection; the window's marks were checked by eye. Since 22 October 2026 (decision 53) an update asks for frames only where its page is drawn, the wall and the list, and for the overview's ring while it bursts (`an_update_in_another_bank_asks_for_frames_only_while_its_ring_bursts`, which failed on the earlier code).

### Requirement 12.11: The clipboard and input methods
- **Statement:** The address field must support copy, paste and text composed by an input method, shown while it is composed.
- **Reason:** Addresses are pasted more often than typed, and many scripts need an input method.
- **Source:** Architecture.
- **Verification:** Test on each desktop.
- **State:** Partly met: composed text is inserted when committed. The address field has no copy or paste, and does not show text while it is composed; `arboard` (decision 49) is in the interface but used only for paste to a channel (REQ-INT-136).

### Requirement 12.12: Tips
- **Statement:** Every control the window draws as an icon, a symbol or text that may be cut short must have a tip: a label naming it, saying what it does, and giving its shortcut on the platform where it has one. A tip must show when the pointer has rested on its control for half a second, beside the pointer and inside the window, over everything else; a press must put it away until the pointer leaves the control, and leaving must put it away. Tips must be the interface's own words, except a tab's title and a bookmark's name, which must be shown as the cleaned text they are (requirement 11.5). While a panel or list is open over the window, only its tips and the strip's must show.
- **Reason:** An icon alone does not say what it does, and the shortcuts are otherwise found only in the menu.
- **Source:** Decision 63.
- **Verification:** Interface tests: a tip shows after the rest and not before, with each platform's shortcut, and goes on a press and on leaving; a review that every icon, symbol and cut text has one.
- **State:** Built 23 October 2026, with tests `a_tip_shows_after_a_rest_and_goes_on_a_press` and `tips_name_each_platforms_shortcut`; seen on macOS. Tips cover the tab strip (each tab's title, its close and sound buttons, new tab, the window buttons elsewhere than macOS), the toolbar (Back, Forward, Reload, Home, the address field, the star, mute all, settings, the menu), the bookmarks bar's chips, the banks, the page size's buttons, the overview's fold, the console's tabs and buttons, the cookies' buttons, and the close, remove and volume buttons of the settings, history and bookmarks panels and the phone keyboard's. The review of every control was done on 23 October 2026 and gave the open page's close button its tip (`a0efff5`).

### Requirement 12.13: Tabs are reordered by dragging
- **Statement:** A tab pressed and moved more than 4 points must be dragged, a mark showing where it will go, and released must move there, by strip input the trusted core carries out, keeping the active tab active and saving the session; released where it started, or after Escape, nothing changes.
- **Reason:** Every browser lets the user order tabs (requirement 12.1).
- **Source:** Decision 69.
- **Verification:** Tests of the interface and the trusted core.
- **State:** Met, 23 October 2026: `a_dragged_tab_moves_where_it_is_let_go`, `tabs_move_and_the_active_one_stays_active`; driven by tests only, since posted pointer events do not reach the window.

### Requirement 12.14: Tabs that do not fit wrap into a panel under the strip
- **Statement:** When the tabs do not fit at their smallest width, the strip must show those that fit, always with the active tab, and a count of the rest; while the pointer is on the strip or the panel, or after the count is pressed, a panel joined to the strip's underside must show the rest wrapped in rows, each one chosen, closed, muted and dragged as on the strip.
- **Reason:** The owner's request; before, a tab that did not fit could not be reached.
- **Source:** Decision 69.
- **Verification:** Interface tests; seen on macOS.
- **State:** Met: `the_active_tab_is_always_shown`, `overflowing_tabs_wrap_into_a_panel_under_the_strip`; the count and the active tab's place seen on macOS with 25 tabs; the panel's opening by the pointer driven by tests only.

### Requirement 12.15: Keys choose tabs
- **Statement:** Command-1 to 8 must choose the first eight tabs and Command-9 the last, and Command-Option-Left and Right the tab before and after, round the ends (Control and Alt away from macOS), whether the tab is shown on the strip or not.
- **Reason:** Chrome's keys, and the only way by keyboard to a tab in the panel.
- **Source:** Decision 69.
- **Verification:** Test.
- **State:** Met: `tab_keys_choose_tabs`.

---

## 13. The application update mechanism

Requirements 13.1 to 13.13 are unchanged from the earlier design, and are stated there in full (`prototypes/docs/requirements.md` section 13). In brief:

| Requirement | Statement |
|---|---|
| 13.1 | No failure at any point of an update may leave the user without a complete, working version |
| 13.2 | The running version is never modified; a new one is installed beside it |
| 13.3 | Nothing is switched to until it is downloaded, verified against The Update Framework metadata and the platform's code signature, and installed |
| 13.4 | The switch is one atomic rename of a pointer |
| 13.5 | The previous version stays until the new one passes its health check; two failed starts roll back |
| 13.6 | Profiles are copied and migrated, never changed in place |
| 13.7 | A small launcher, fully reviewed, rarely changed, updated only by atomic directory exchange or through a fixed stub |
| 13.8 | The Update Framework, implemented by `tough`, with threshold root keys built into the launcher |
| 13.9 | Root keys: three hardware security keys held by three people, threshold two, one in a safe |
| 13.10 | One launcher scheme on every desktop; no package manager as the update path |
| 13.11 | Updates from `update.neoteletext.com` only, contacted only by the updater |
| 13.12 | Automated tests of every failure on every desktop |
| 13.13 | Phones update only through their stores |

- **State of 13.1 to 13.13**, 23 October 2026 (`installation-and-updates.md` sections 14 and 15; decisions 73 to 82): built on every desktop. 13.1 to 13.5: `a_staged_version_is_accepted_tried_and_committed`, `a_broken_version_is_rolled_back_after_two_starts` (crash, hang and silence), `a_version_not_signed_or_not_whole_is_refused`, `a_lost_state_is_rebuilt_from_what_is_installed` and `the_launcher_killed_at_every_step_still_starts_and_finishes_the_update` (`native/launcher/tests/launcher.rs`), on macOS, Linux and Windows; the state written to a new file, flushed and renamed (`update::files::write_atomically`). 13.6: `a_new_format_copies_and_the_old_file_is_kept_as_it_was` and the settings read through the profile rule. 13.7: the macOS bundle exchanged with `renamex_np`, Linux's launcher folder with `renameat2` (`a_launcher_release_exchanges_the_launcher_folder`), Windows's stub and two copies; the release tool's launcher-unchanged check (`a_changed_launcher_needs_a_launcher_release`). 13.8: `tough` 0.24 in the confined updater, the root's threshold, expiry, version and rotation tested in `native/tuf/release/tests/updates.rs`; the root keys built in through the release identity. 13.9: the release tool signs with hardware keys through a program and names an absent root holder's key by its public half; the key ceremony is the owner's (not met until it is held). 13.10: one launcher scheme on all three desktops. 13.11: the updater alone reaches the update host, its sandbox one port (`the_updater_reaches_one_port_and_writes_only_its_staging_folder`, Linux and macOS); the host's name pinned by TLS and by the transport; `update.neoteletext.com` not yet chosen. 13.12: the tests above and `the_updater_killed_at_every_step_still_stages_whole`, run in the gates on macOS and Linux and by hand on Windows, and the end-to-end runs of section 15 of the design. 13.13: no phone builds yet.

### Requirement 13.7 (amended)
*Amended 23 October 2026 (decisions 54 and 73):* on macOS the launcher is the bundle's main executable, inside the bundle, exchanged with it, and the release tool's `check` refuses a launcher that differs from the last launcher release's unless the release says it is one (`installation-and-updates.md` sections 4.1 and 7).

### Requirement 13.8 (amended)
*Amended 23 October 2026 (decisions 54, 73 and 74):* `tough` 0.24 runs in the confined updater, in a workspace of its own (`native/tuf`); its first trusted root is the release identity's (decision 76).

### Requirement 13.10 (amended)
*Amended 23 October 2026 (decision 54):* "the launcher scheme" is `installation-and-updates.md` section 3's model on every desktop.

### Requirement 13.14: Two independent signatures
- **Statement:** A version must run only after the updater verified it against The Update Framework's metadata and the launcher verified it was signed by the platform's pinned signer: the macOS bundle's designated requirement, every Windows program's Authenticode signer's public key, or on Linux the release key's signature of the manifest and of every file. The launcher must also find the folder holding exactly the files the release's manifest names, each of its length and SHA-256, and the version newer than the current one and not known bad, read from what the platform signed.
- **Reason:** Shipping malware then needs both the targets key and a platform key, held apart (`installation-and-updates.md` section 5).
- **Source:** Decision 54; decisions 73 and 80.
- **Verification:** Test: a release the targets key signed but the platform's signer did not is refused; a release with a file changed, missing or extra is refused.
- **State:** Met for the build, 23 October 2026: `a_complete_signed_version_is_accepted_and_every_fault_refused` (`update::accept`, the Linux signer) and `a_version_not_signed_or_not_whole_is_refused`; end to end with each platform's own check and test signers (macOS ad hoc with an identifier requirement, a test Authenticode certificate, a test release key), section 15 of the design. Not met until the owner's certificates and release key exist.

### Requirement 13.15: Installation without an administrator
- **Statement:** The browser must install in the user's own folders with no administrator on every desktop: per user on Windows (`%LOCALAPPDATA%\Programs\NeoTeletext`) and Linux (`~/.local/share/neoteletext`), in `/Applications` for an administrator on macOS, else `~/Applications`. Nothing it installs may need elevation to update.
- **Reason:** An elevated updater would be the most powerful process doing the most dangerous thing (`installation-and-updates.md` section 4.2).
- **Source:** Decision 54.
- **Verification:** Demonstration on each desktop, as an ordinary user; review of what the installer writes.
- **State:** Met, 23 October 2026: `NeoTeletext-Setup.exe` in a limited desktop session, `neoteletext --install` and the macOS bundle copied, each in section 15 of the design. On a Linux that restricts user namespaces the browser runs only once an administrator installs its AppArmor profile (requirement 13.16).

### Requirement 13.16: Supported systems
- **Statement:** Releases must start on macOS 11 and later; Windows 10 version 1703 and later; and Linux with the C library 2.28 and later, kernel 6.10 with Landlock, and unprivileged user namespaces allowed for the browser's programs, which on a system that restricts them (Ubuntu from 24.04) means its AppArmor profile, `neoteletext.apparmor`, installed once by an administrator.
- **Reason:** Every sandbox layer is required (decision 58), and a per-user install cannot install an AppArmor profile (`sandboxing.md` section 8.3).
- **Source:** Decision 54; decision 77.
- **Verification:** Check: the release build's newest C library symbol; demonstration on each system's oldest version.
- **State:** Partly met, 23 October 2026: the profile ships with every Linux version and `--install` says how to install it; the C library 2.28 build is a script not yet run (it needs an AlmaLinux 8 image); macOS 11 and Windows 10 1703 not run.

### Requirement 13.17: No forced restart, nothing imposed
- **Statement:** The browser must offer a newer version (once in the strip, and in the settings panel while it stands) and download and stage nothing until the user accepts. A staged version must start at the browser's next start, never by restarting it under the user. "Check for updates", on by default, must stop the updater from starting at all when off.
- **Reason:** The owner's answer to section 12, item 9 of the design: updates are suggested, never imposed.
- **Source:** Decision 54; decision 81.
- **Verification:** Test.
- **State:** Met, 23 October 2026: `a_newer_version_is_offered_once_and_downloaded_only_when_accepted` and `the_panel_offers_an_update_only_when_the_trusted_core_does`; the launcher accepts staged versions only at its start.

### Requirement 13.18: The updater is confined
- **Statement:** The updater must run confined like the other helpers, every layer required: TCP to one port only, the update host's or the user's proxy's, the system's name resolution, and its staging folder written; no other file written, no program started, no secret of the user's held. Its folder and port must be the broker's rules', never the trusted core's word.
- **Reason:** It is the process that parses the update host's data (`installation-and-updates.md` section 8.1).
- **Source:** Decision 54.
- **Verification:** Test on each desktop.
- **State:** Met, 23 October 2026: `the_updater_reaches_one_port_and_writes_only_its_staging_folder` on Linux (Landlock's port rule and the fetcher's socket families) and as a profile on macOS; on Windows an `AppContainer` with the internet client capability, which cannot be told a port, the host pinned by TLS; the updater confined end to end on each desktop (section 15 of the design).

### Requirement 13.19: The health check decides
- **Statement:** A trial version must be committed only when, within 30 seconds of its start, its window's first frame has reached the screen, its settings were read or found absent, and no decoder ended in error; one that is not healthy within 45 seconds must be ended with every process it started, and counted as a failed start.
- **Reason:** The check is what the user would notice: a window that draws, with sandboxes that hold (`installation-and-updates.md` section 3.2).
- **Source:** Decision 54; decision 80.
- **Verification:** Test.
- **State:** Met, 23 October 2026: `a_version_is_healthy_only_when_it_drew_read_its_settings_and_lost_no_helper`, and the launcher's rollback of a version that hangs or crashes.

### Requirement 13.20: Uninstall
- **Statement:** Removing the browser must remove its programs, its update files and its menu entries, and must ask before removing the user's settings, bookmarks, history and cookies.
- **Reason:** "Leaves nothing behind when removed" (`installation-and-updates.md` section 1), and the user's data is the user's.
- **Source:** Decision 54; decision 82.
- **Verification:** Demonstration on each desktop.
- **State:** Met, 23 October 2026, end to end (section 15 of the design): `--uninstall` on Linux and macOS; on Windows the uninstaller's copy removes the folder, the shortcut, the uninstall entry and the containers' profiles. Its message boxes were not pressed by a hand; on Windows its temporary copy stays in `%TEMP%`.

### Requirement 13.21: The release's identity is built in
- **Statement:** What a build trusts for updates (the update host, The Update Framework's first root, the platform signers) must be fixed when it is built, and no setting, file or variable read at run time may change it.
- **Reason:** A trust anyone running as the user could redirect would be a downgrade (decision 58).
- **Source:** Decision 76.
- **Verification:** Review; test: the production identity names the update host of decision 25.
- **State:** Met, 23 October 2026: `native/release-identity`, `the_production_identity_names_the_update_host`.

---

## 14. Build, dependencies and verification

### Requirement 14.1: One check script and hooks
- **Statement:** One local check script must run every check in this document. Git hooks must run the fast checks before each commit and the full checks before each push, and must not be bypassed.
- **Reason:** Requirements that are not checked drift.
- **Source:** Owner's baseline instructions.
- **Verification:** Review of the hooks.
- **State:** Partly met. `scripts/check.sh commit`, run before each commit, formats, lints and tests only the parts the staged files touch, and refuses to run while those parts have changes that are not staged, so what it checks is what is committed. `scripts/check.sh full`, run before each push on a clean tree at the commit pushed, adds release tests, `cargo-deny`, `cargo-machete`, fuzzing, and the Linux build and tests on the Linux host, for the browser's workspace and the updater's (`native/tuf`); since 21 October 2026 it builds no prototypes, which were removed (decision 50). Neither skips a step: a missing tool or an unreachable host fails the run. The compiler and every tool are pinned (`rust-toolchain.toml`, `scripts/tools.sh`). The full gate passes at each push (the `pre-push` hook), the last on 23 October 2026. Many checks named in this document are not written yet.

### Requirement 14.2: Strict compilation and linting
- **Statement:** Rust code must build with `unsafe` code forbidden, the Clippy lint group `pedantic` enabled and warnings treated as errors; any suppression must be narrow, at its site, with a written reason.
- **Reason:** Warnings are faults found early.
- **Source:** Owner's baseline instructions.
- **Verification:** Check.
- **State:** Met. Beyond the statement: every public item is documented, and the crates that read input from outside their process (every crate in `NO_PANIC_ROOTS` of `scripts/check.sh`) also deny every construct that can panic, with one list the check script keeps identical in each.

### Requirement 14.3: Dependencies
- **Statement:** Lockfiles must be committed and their changes reviewed. `cargo-deny` and `cargo audit` must pass. Every library compiled into the browser must be under a permissive licence (decision 34) and needs the owner's approval. System libraries linked at run time follow `licensing.md`, which admits LGPL ones linked dynamically and not shipped (decision 40).
- **Reason:** The supply chain is a route to compromise.
- **Source:** Decisions 4 and 34; owner's baseline instructions.
- **Verification:** Check.
- **State:** Partly met: every dependency is pinned to an exact version. `cargo-deny` runs in the full gate on both workspaces (`native/deny.toml`, `native/tuf/deny.toml`): licences, sources, and the RustSec advisories that `cargo audit` also reads; `cargo audit` itself is not run. Four advisories are ignored: RUSTSEC-2026-0192, `ttf-parser` unmaintained, which only winit's Wayland window decorations use, pending the owner's decision; and RUSTSEC-2026-0370 to 0372, `zvariant`, `zbus_names` and `zvariant_derive` merged into zbus 6, which the Linux Secret Service client (decision 62) has not moved to. The libraries in use were approved by name on 20 October 2026 (decision 41) and since by decisions 49, 62 and 74.

### Requirement 14.4: Withdrawn
- **Was:** a runtime caller-attribution test before any change of Tauri or webview runtime.
- **Why withdrawn:** there is no Tauri (decision 33). Its successor is requirement 12.8's test that each window's messages are attributed to that window.

### Requirement 14.5: Spoofing review
- **Statement:** Every new layout, zoom level or full-screen mode must be reviewed to confirm that the trusted strip remains visible and that nothing in the page area can be mistaken for it, and the review must be recorded.
- **Reason:** An interface built for animation can hide or imitate security indicators.
- **Source:** Decision 4.
- **Verification:** Recorded review.
- **State:** Not met: no review is recorded for the window of decision 35.

### Requirement 14.6: Broadcast content is never committed
- **Statement:** Teletext recordings and sites built from them, which are broadcast material, must never be committed; they stay in ignored folders.
- **Reason:** The project has no right to redistribute them.
- **Source:** Owner's request of 15 October 2026.
- **Verification:** Review of every commit.
- **State:** Met: the development site is built into the ignored `native/target/sites`.

---

## 15. Platforms

### Requirement 15.1: Desktop platforms
- **Statement:** The application must run on macOS, Windows and Linux with every requirement in this document met.
- **Reason:** The first version is desktop only (decision 24).
- **Source:** Architecture.
- **Verification:** The full check on each platform.
- **State:** Partly met, 23 October 2026: built and run on macOS, Linux (Ubuntu 26.04, X11 under Xvfb on `foundintelquiet4060`) and Windows 11 (the virtual machine's desktop): every process confined (requirement 3.7, decisions 52, 55, 56 and 58), sound (requirement 16.6, decision 40), session restore (requirement 20.22, decision 59), cookies (requirement 21.1, decision 62) and installation and updates (section 13, `installation-and-updates.md` section 15, decision 73). The full gate builds and tests on Linux (`check_linux`); Windows is built and tested by hand (`DEVELOPMENT.md` section 12.1). Not met on every platform: the Dock menu's counterparts (requirement 16.9), the interface's file reads on Windows (requirement 3.6), the XDG Desktop Portal (requirement 17.9).

### Requirement 15.2: Phones (after the first version)
- **Statement:** When phones are added, Android must run the decoder in an isolated service process, and on iOS, where there is one process, the decoder must have a wall of its own (decision 41: `wasmi`). Every requirement not tied to process separation must still be met.
- **Reason:** Phones limit process separation.
- **Source:** Architecture.
- **Verification:** The full check on each phone platform.
- **State:** Not met.

### Requirement 15.3: Microcontrollers
- **Statement:** The interface core must build without the standard library and draw through the microcontroller back end. A microcontroller build must keep pages, page-number and Fastext keys, flash and a simple page wall, with the cuts the owner decides (decision 41: `architecture.md` section 13), and must have its own security analysis.
- **Reason:** Microcontrollers are a target (decision 34).
- **Source:** Decision 34.
- **Verification:** Check: the interface core and the microcontroller back end build for a microcontroller target; a test draws the window into a display in memory.
- **State:** Partly met: the interface core is `no_std` and a test draws the window through `embedded-graphics` into a 1024 by 600 display in memory. No microcontroller target is built.

### Requirement 15.4: No graphics processor needed
- **Statement:** Every target must work with drawing on the processor only; a graphics processor may make it faster but must never be required.
- **Reason:** Decision 34.
- **Source:** Decision 34.
- **Verification:** Test on a machine or virtual machine with no graphics processor.
- **State:** Met by design on every desktop (`vello_cpu` and softbuffer; no graphics library is linked); on 22 October 2026 the browser drew under Xvfb on Linux, a display with no graphics processor (`sandboxing.md` section 4). Not tested on a macOS or Windows machine without one.

---

## 16. Sound

### Requirement 16.1: Sound follows the engine's audio profile
- **Statement:** Sounds a channel carries, on EN 300 708 independent data lines of data channel 12, must be played as the engine's audio profile specifies (its area AUD, decisions E.84 to E.86): the latest command of each channel wins, and a page's sounds play while that page is open. The browser adds no rules of its own to what plays.
- **Reason:** One set of rules for every receiver, kept in the engine (decision 36), so that a channel sounds the same in the browser as on a microcontroller.
- **Source:** Decision 37; the engine's decisions E.84 to E.86.
- **Verification:** Decoder tests: a page's sound is sent when its page is opened and faded when it is left; a live sound streams and fades two seconds after its last block.
- **State:** Met on macOS.
- *Amended 23 October 2026:* met on macOS, Windows and Linux: its tests are not tied to a platform and run in the Linux gate, and page sound played through all three desktops (requirement 16.6).

### Requirement 16.2: Sound is read only in the decoder
- **Statement:** Audio packets must be read, reassembled, stored and decoded only in the decoder process. The trusted core must not parse them, and the sound helper must receive only 16-bit samples and voice operations the trusted core checked (decision 55).
- **Reason:** Untrusted bytes are read only in a process with no rights (architecture principle 2), and nothing in the interface parses data from outside (principle 3).
- **Source:** Decisions 18 and 37.
- **Verification:** Inspection: the audio receiver is a dependency of the decoder alone; the sound helper takes only `protocol::sound::ToSound`, whose audio is `protocol::audio::Audio`; the interface takes no sound at all, only which channels are sounding.
- **State:** Met; since 22 October 2026 the sound helper, not the interface (decision 55).

### Requirement 16.3: Every tab is heard
- **Statement:** The trusted core must pass on every channel's sound, tagged with its channel, and the sound helper must play each channel through its own mixer, so a background tab keeps playing, as in a web browser. A background tab keeps the page it had open. When a channel's decoder is gone, the trusted core must tell the sound helper to forget that channel's sounds.
- **Reason:** The owner asked for tabs that play sound like Chrome's (decision 38); a decoder cannot touch another channel's sound.
- **Source:** Decisions 37 and 38.
- **Verification:** Tests: `hearing::tests::every_channel_is_heard_and_its_operations_are_tagged_with_it`, `the_opened_page_goes_to_the_active_channels_decoder_and_others_keep_theirs`; the sound helper's `every_channel_is_heard_and_a_muted_one_is_silent_but_still_sounding`; by hand on 19 October 2026, a live sound kept playing after its tab went to the background.
- **State:** Met on macOS.

### Requirement 16.4: Every voice operation is checked twice
- **Statement:** Every voice operation must be bounded by the protocol (a channel of 0 to 3, a gain of 0 to 100, a fade of 5 to 10 000 milliseconds, a profile sample rate, at most two minutes a sound and 32 768 samples a message), and checked again by the trusted core: a sound must be defined before it is filled or started, samples must lie within it, a live sound takes only its next samples, and a channel may hold at most 32 MB of samples in the sound helper, every channel together 64 MB, a live sound counting as two seconds at 48 kHz. Only the trusted core may tell the sound helper to forget a channel's sounds, or mute it.
- **Reason:** A compromised decoder could otherwise make the sound helper hold memory without bound, or reset another channel's sound.
- **Source:** Requirements 4.2 and 4.4; decision 37.
- **Verification:** Tests: the protocol's audio tests of every bound and of a decoder's reset or mute; `hearing::tests::operations_are_checked_against_what_was_defined`, `a_live_sound_takes_stream_samples_only` and `the_ledger_bounds_what_the_interface_holds`.
- **State:** Met.

### Requirement 16.5: The opened page
- **Statement:** A page's sounds must play while it is open: the page zoomed on the wall, or else the page chosen by number or in the list. Page sounds are on by default. A setting to turn them off, and a control for the master volume, must be added to the window; muting is requirements 16.7 to 16.9.
- **Reason:** Teletext attaches sounds to pages, and a television goes to the page whose number is typed.
- **Source:** Decision 37; the engine's REQ-AUD-012.
- **Verification:** Test: `the_opened_page_is_the_zoomed_one_or_else_the_one_chosen`; an end-to-end check by hand on 19 October 2026 (typing 100 played the page's chime).
- **State:** Met: the setting and the volume are in the settings panel (requirement 19.4).

### Requirement 16.6: Sound output
- **Statement:** The sound helper must mix with the engine's mixer (`neoteletext-audio-mix`) and play through the operating system's output device with `cpal`, rendering on the audio thread without allocating. With no output device, or no system sound library, the browser must run silent.
- **Reason:** One mixer that behaves as the microcontrollers' does; `cpal` keeps unsafe code out of the browser's crates.
- **Source:** Decisions 37, 40 and 55.
- **Verification:** The sound helper's tests of applying operations and rendering; `a_tone_plays_through_the_default_output_device` and `a_tone_plays_through_the_confined_sound_helper`, run by hand because they need a device (`cargo test -p neoteletext-sound -- --ignored`); setting `NEOTELETEXT_AUDIO_TRACE` makes the sound helper report the operations it applies and the loudest sample played.
- **State:** Met on macOS, Windows (WASAPI) and Linux (ALSA): on 19 October 2026 a page's sound played through each, as the device's peak level showed (decision 40). Since 22 October 2026 (decision 53) the device runs only while a channel sounds: it is stopped at start and started on the first sound by a watcher the mixer wakes (`the_watcher_is_woken_only_when_silence_starts_or_ends`; `a_tone_plays_through_the_default_output_device` by hand on macOS, where the idle browser's wakeups fell from 1,030 to about 150 in 10 seconds); on Windows the device opened in the confined interface in the virtual machine's desktop session; the stop and start on Windows and Linux were not watched. Since 22 October 2026 (decision 55) the sound helper plays, confined: `a_tone_plays_through_the_confined_sound_helper` passed on macOS, on Linux through PipeWire and on Windows in the desktop session, each reporting the channel sounding and then silent, the second consumed in real time; on macOS a podcast played through the whole browser and the tab showed its speaker; on Linux with `libasound.so.2` hidden the browser started, drew and ran silent (`sandboxing.md` section 7.5).

### Requirement 16.7: A tab shows its sound and mutes
- **Statement:** A tab whose channel plays sound must show a speaker before its close button, and a muted tab a crossed-out speaker, whether or not it plays. A click on it must mute the tab, or unmute it, without selecting it. A tab's mute must follow it to the next channel it shows. The window's title must be the selected tab's, followed by "Audio playing" or "Muted" while it plays sound.
- **Reason:** The owner asked for Chrome's behaviour (decision 38).
- **Source:** Decision 38.
- **Verification:** Tests: `a_tab_playing_sound_shows_a_speaker_that_mutes_it`, `muting_tells_the_interface_once_per_change`; by hand on 19 October 2026: the speaker appeared on a background tab playing a live sound, a click muted it (the device's peak fell to nothing), and the title read "livechannel.t42 – Muted – NeoTeletext".
- **State:** Met on macOS.

### Requirement 16.8: Mute every tab
- **Statement:** A button beside the menu must mute every tab, or, once every tab is muted, unmute every tab; its speaker is crossed out while every tab is muted.
- **Reason:** The owner asked for it (decision 38).
- **Source:** Decision 38.
- **Verification:** Test: `the_mute_all_button_mutes_every_tab_until_all_are_muted`; by hand on 19 October 2026.
- **State:** Met on macOS.

### Requirement 16.9: The Dock menu
- **Statement:** On macOS, right-clicking the browser's Dock icon must list its tabs, the selected one ticked and each playing or muted one marked with a speaker, then "Mute All Tabs", or "Unmute All Tabs" once every tab is muted. Choosing a tab must select it.
- **Reason:** The owner asked for a Dock menu like a web browser's (decision 39).
- **Source:** Decision 39.
- **Verification:** Tests: `a_tab_is_marked_as_its_sound_is`, `mute_all_unmutes_once_every_tab_is_muted`; by hand on 19 October 2026: the menu ticked the selected tab and marked the playing one, choosing a tab selected it, and both mute items worked.
- **State:** Met on macOS. Windows' taskbar jump list and Linux's launcher menus are not built.

### Requirement 16.10: Live sounds
- **Statement:** A live sound (the engine's REQ-AUD-014) must play as its blocks arrive, from the newest block when its play starts, and fade out two seconds after its last block. Each decoder keeps the clock of its live sounds itself, waking when the audio receiver says it next needs a tick (the engine's decision E.98), and not otherwise. *Amended 21 October 2026 (decision 47):* the trusted core ticked every decoder twelve times a second for this.
- **Reason:** The owner asked for live sounds (decision 38).
- **Source:** Decision 38; the engine's decision E.87.
- **Verification:** Tests: the decoder's `a_live_sound_streams_and_fades_two_seconds_after_its_last_block`; the interface's `a_live_sound_plays_once_its_buffer_fills`; by hand on 19 October 2026, a ten-minute live sound written by `neottx sound --live` played from a file channel.
- **State:** Met on macOS.

---

## 17. Hyperlinks

Adopted on 20 October 2026 (decision 41) from [`hyperlinks.md`](hyperlinks.md), whose section numbers these cite; [`hyperlinks-security.md`](hyperlinks-security.md) is their security analysis. Built the same day behind the `links` feature, on by default; the browser builds and passes its tests with it compiled out too (`--no-default-features`).

### Requirement 17.1: No teletext underline
- **Statement:** Teletext's underline attribute is not drawn on any page shown by NeoTeletext; separated mosaics are drawn as before.
- **Reason:** An underline must mean only a link.
- **Source:** Decisions 41 and 42; `hyperlinks.md`.
- **Verification:** Test: a Level 2.5 page with underlined text and separated mosaics.
- **State:** Met: the decoder renders with the engine's `underline` option off (decision E.88). Test: `the_pages_own_underline_is_not_drawn`, which fails without the change.

### Requirement 17.2: Links are drawn by the browser
- **Statement:** Links are drawn only by the interface, over the page image, in one style the page cannot change (`hyperlinks.md` section 9.2), and only when a teletext dot is at least one and a half device pixels.
  *Amended 20 October 2026:* two device pixels, as adopted, showed no links on a page shown at its own size on an ordinary screen (`hyperlinks.md` section 19).
- **Reason:** A page must not be able to draw the browser's link.
- **Source:** Decisions 41, 42 and 43; `hyperlinks.md`.
- **Verification:** Test of the drawing at several scales; review.
- **State:** Met: the interface draws a hairline at screen resolution with a contrasting edge, only over the image the links belong to. Tests: `links_are_drawn_only_when_a_dot_is_one_and_a_half_device_pixels`, `the_underline_is_along_the_bottom_of_the_cells_and_finer_than_a_dot`, `links_are_drawn_only_over_their_own_image`; seen on macOS on 20 October 2026.

### Requirement 17.3: The trusted core decides every link
- **Statement:** The trusted core decides every link, from the decoder's and the fetcher's candidates, checked as `hyperlinks.md` section 8.2 requires. The interface receives only where to draw and the link's kind.
- **Reason:** The decoder and the page are untrusted; the interface holds no channel data.
- **Source:** Decision 41; `hyperlinks.md`.
- **Verification:** Test with out-of-range, overlapping, excessive and invalid links.
- **State:** Met: the trusted core checks rows, bounds, overlaps and targets, and the interface receives places and kinds only. Tests: `links_are_checked_and_decided`, `malformed_links_are_refused`, `a_target_too_long_is_refused_never_cut`.

### Requirement 17.4: Page numbers in the text are links
- **Statement:** Page numbers in the text are links when, and only when, the rules of `hyperlinks.md` section 6.1 hold.
- **Reason:** Navigation teletext readers already expect, without false links.
- **Source:** Decision 41; `hyperlinks.md`.
- **Verification:** Test of each rule, including currency, times, ranges and absent pages.
- **State:** Met, with the refinements of `hyperlinks.md` section 19. Tests: `page_numbers_stand_alone`, `a_pages_links_are_found_in_its_text_and_its_link_lines`.

### Requirement 17.5: Links are classified from their targets
- **Statement:** Every link, from a page, a Fastext packet or the descriptor's aliases and services, is classified from its target alone, before any request, as `hyperlinks.md` section 7.1 requires; a named service is another channel.
- **Reason:** No request before consent; the strip can say where a link leads.
- **Source:** Decision 41; `hyperlinks.md`.
- **Verification:** Test of every row of the table.
- **State:** Met. Tests: `targets_are_classified_from_the_target_alone`, `a_file_channels_links_to_the_web_leave_it`, `a_pages_number_links_and_the_channels_aliases_remap_numbers`.

### Requirement 17.6: The destination is shown in the strip
- **Statement:** Pointing at or focusing a link shows its destination in the trusted strip.
- **Reason:** The strip, not the underline, is the proof.
- **Source:** Decision 41; `hyperlinks.md`.
- **Verification:** Test; review.
- **State:** Met. Tests: `the_pointer_on_a_link_is_reported_once_per_link_with_its_shape`, `a_new_image_under_the_pointer_is_reported_again`, `the_strip_shows_where_a_link_leads_in_place_of_the_address`; by hand on macOS on 20 October 2026.

### Requirement 17.7: A click follows what the user saw
- **Statement:** A click follows only the link of the frame it was made on, whose target equals the one the strip is showing, unchanged for at least 500 milliseconds.
- **Reason:** Against bait and switch.
- **Source:** Decision 41; `hyperlinks.md`.
- **Verification:** Test with a fake clock: a page that changes a link's target each frame.
- **State:** Met. Tests: `a_click_follows_only_the_link_the_strip_shows_once_it_has_settled`, `a_link_that_changes_under_the_pointer_is_not_followed`, `an_unchanged_link_keeps_its_age_across_new_images`, `a_click_names_the_image_it_was_made_on`.

### Requirement 17.8: Leaving goes through the strip
- **Statement:** A link that leaves the channel goes through the site-change screen; a link to the web goes through the web-browser prompt; neither can be confirmed from the page area.
- **Reason:** Leaving is the user's decision, made where a page cannot reach.
- **Source:** Decision 41; `hyperlinks.md`.
- **Verification:** Test for each kind, with clicks in the page area.
- **State:** Met. Tests: `leaving_and_the_web_are_asked_about_and_answered_only_after_half_a_second`, `the_question_waits_half_a_second_and_never_takes_the_focus`; by hand on macOS on 20 October 2026: a link to another channel showed the site-change screen, Go left, Back returned.

### Requirement 17.9: The hand-off to the system's web browser
- **Statement:** Only an `https` or `http` address the core's address rules accept, re-serialised from its parsed parts, is handed to the system's web browser, by the trusted core, as one argument with no shell, at most once per two seconds, from the selected tab of a focused window, and logged.
- **Reason:** Opening an address must never run a program.
- **Source:** Decision 41; `hyperlinks.md`.
- **Verification:** Test with `file:`, custom schemes, paths, a leading `-` and over-long addresses; the "nothing else opens" test of `hyperlinks-security.md` section 15.
- **State:** Partly met. macOS: `/usr/bin/open -u`, by hand on 20 October 2026 (an `https` address opened in the default browser and was logged). Linux: `xdg-open`; the XDG Desktop Portal awaits the approval of a D-Bus crate (hyperlink decision 10). Windows: `ShellExecuteW`, by hand on 20 October 2026 in the Windows 11 VM's desktop (`an_address_opens_in_the_default_browser`: Edge opened the address); every test passed there with links compiled in and out. Tests: `only_plain_https_and_http_addresses_are_handed_over`, `nothing_but_a_confirmation_hands_a_web_page_over`, `the_web_is_handed_over_only_with_the_focus_and_at_most_every_two_seconds`.

### Requirement 17.10: Links by keyboard
- **Statement:** Links can be reached and followed with the keyboard, and are in the accessibility tree.
- **Reason:** Not everyone uses a pointer.
- **Source:** Decision 41; `hyperlinks.md`.
- **Verification:** Test.
- **State:** Partly met: Tab, Shift-Tab, Enter and Escape, with a test (`tab_moves_between_links_and_enter_follows_the_focused_one`). The accessibility tree does not exist yet (requirement 12.9), so screen readers see no links.

### Requirement 17.11: Prompts never take the focus
- **Statement:** The web-browser prompt and the site-change screen never take the keyboard focus by themselves, and accept Enter only after the user moved the focus into them.
- **Reason:** A confirmation key pressed for something else must not confirm.
- **Source:** Decision 41; `hyperlinks.md`.
- **Verification:** Test.
- **State:** Met. Test: `the_question_waits_half_a_second_and_never_takes_the_focus`.

### Requirement 17.12: The link-target parser
- **Statement:** The core's link-target parser contains no panicking constructs and is fuzzed, as the message readers are.
- **Reason:** It is the first page data the trusted core parses.
- **Source:** Decision 41; `hyperlinks.md`.
- **Verification:** The fuzz target `link_target` in the full gate; the lint.
- **State:** Met: the fuzz target `link_target` (seven million runs in its first 30 seconds, no fault) runs in the full gate, and the core's library carries the no-panic lint list.

## 18. Push, typing and the cursor

From `interaction.md` (decision 44), built on 20 October 2026 (decision 45). On 21 October 2026 the input system's own requirements moved to `~/devel/neoteletext-common/INPUT.md` (decision 48); 18.1, push, stays here.

### Requirement 18.1: Push over every transport, with a heartbeat
- **Statement:** The fetcher receives push over WebSocket, plain and TLS, and Server-Sent Events, with a heartbeat; pushed packets reach the decoder at once.
- **Reason:** A channel pushes lines as a transmitter does.
- **Source:** Decision 44; `interaction.md` section 4.
- **Verification:** Tests against `neoteletext-site` in each mode.
- **State:** Partly met. WebSocket push, plain, is exercised against `neoteletext-site terminal` by the latency test; TLS is `rustls` (requirement 5.8), not yet tried against a `wss` channel; Server-Sent Events against `neoteletext-site serve`. The heartbeat is the engine's (its decision E.92), with tests there. A pushed message and a fetched file are read whole and their pages completed at once (`a_pushed_page_shows_as_it_arrives`).

### Requirement 18.2: When keys go to a channel
- **State:** Withdrawn on 21 October 2026 (decision 48): replaced by REQ-INT-130 and REQ-INT-136 of `~/devel/neoteletext-common/INPUT.md`, below. Its text as built for version 1 is in this repository's history.

### Requirement 18.3: The strip says where keys go
- **State:** Withdrawn on 21 October 2026 (decision 48): replaced by REQ-INT-131 of `~/devel/neoteletext-common/INPUT.md`, below. Its text as built for version 1 is in this repository's history.

### Requirement 18.4: The fetcher takes only key records
- **State:** Withdrawn on 21 October 2026 (decision 48): replaced by REQ-INT-133 of `~/devel/neoteletext-common/INPUT.md`, below. Its text as built for version 1 is in this repository's history.

### Requirement 18.5: Latency
- **State:** Withdrawn on 21 October 2026 (decision 48): replaced by REQ-INT-135 of `~/devel/neoteletext-common/INPUT.md`, below. Its text as built for version 1 is in this repository's history.

### Requirement 18.6: Cursor records are checked
- **State:** Withdrawn on 21 October 2026 (decision 48): replaced by REQ-INT-134 of `~/devel/neoteletext-common/INPUT.md`, below. Its text as built for version 1 is in this repository's history.

### Requirement 18.7: Cursor and echo
- **State:** Withdrawn on 21 October 2026 (decision 48): replaced by REQ-INT-134 of `~/devel/neoteletext-common/INPUT.md`, below. Its text as built for version 1 is in this repository's history.

### Requirement 18.8: The round trip, and no key logged
- **State:** Withdrawn on 21 October 2026 (decision 48): replaced by REQ-INT-137 of `~/devel/neoteletext-common/INPUT.md`, below. Its text as built for version 1 is in this repository's history.

### Requirement 18.9: The phone keyboard
- **State:** Withdrawn on 21 October 2026 (decision 48): replaced by REQ-INT-138 of `~/devel/neoteletext-common/INPUT.md`, below. Its text as built for version 1 is in this repository's history.

### Requirement 18.10: Both compile out
- **State:** Withdrawn on 21 October 2026 (decision 48): replaced by REQ-INT-139 of `~/devel/neoteletext-common/INPUT.md`, below. Its text as built for version 1 is in this repository's history.

### The input system: REQ-INT-130 to REQ-INT-139

The input system's requirements for NeoTeletext are in `~/devel/neoteletext-common/INPUT.md` section 12, which holds each one's statement, reason and verification; the engine's REQ-INT-101 to REQ-INT-116 are the protocol's, built in its `neoteletext-input` and traced by the engine. Here is where each of NeoTeletext's stands.

#### REQ-INT-130: When keys go
- **State:** Met in tests: `input_goes_only_under_every_condition`, `a_session_starts_with_hello_and_waits_for_welcome`, `leaving_the_page_says_focus_out_and_page_zero`, `editing_the_address_takes_the_keys_from_the_page_and_says_so`, `keys_go_only_while_the_core_says_so`, `command_never_goes`. Version 2 has not been tried by hand in the window.

#### REQ-INT-131: The strip
- **State:** Met in tests: `the_strip_says_where_keys_go` ("Keys go to", the round trip, "Not secure", and "did not answer" for a channel that did not welcome input).

#### REQ-INT-132: The trusted core encodes
- **State:** Met: the interface sends intents (`interface_core::input::Intent`), which the protocol reads whole and refuses out of bounds, and the trusted core's session alone encodes, HELLO, RESIZE, PAGE and FOCUS included; tests `every_intent_round_trips`, `what_breaks_a_bound_is_refused`, `a_session_starts_with_hello_and_waits_for_welcome`.

#### REQ-INT-133: The fetcher
- **State:** Met: `read_to_fetcher` takes only input messages, whole items of the stream of at most 4 096 bytes, read by the engine's reader on both sides; test `a_message_for_the_fetcher_is_whole_and_bounded`; the fuzz target `fetcher_commands`.

#### REQ-INT-134: Cursor and echo drawn safely
- **State:** Met: the engine's `cursor::from_body` in the trusted core, checked again by the interface's reader; tests `echo_stays_in_its_width_is_masked_and_clears_on_acknowledgement_or_time`, `the_cursor_is_drawn_only_while_typing`, `the_acknowledgement_clears_echo_and_measures_the_round_trip`, `typing_round_trips_and_a_bad_cursor_or_mode_is_refused`; the fuzz target `input_records`.

#### REQ-INT-135: Latency
- **State:** Met on macOS, by `keys_round_trip_and_pages_reach_the_screen_within_budget`, which now opens the session with HELLO and WELCOME and types text: on 21 October 2026, release build, key to the channel's pushed page median 0.22 ms, worst 0.42 ms; render 0.90 ms, encoding 1.73 ms and the interface's decoding 0.17 ms (medians), the three together at worst 3.19 ms. It runs in the full gate's Linux stage; its Linux figures are not recorded, and it has not been run on Windows.

#### REQ-INT-136: Mouse, paste and composition
- **State:** Met in tests. Clicks, moves and the wheel go only in a mouse mode, over the page, the browser's modifier keeping a click the browser's; the preedit goes as COMPOSE in that mode; the input method is enabled while keys go; the browser's paste shortcut (Command-V on macOS, Control-Shift-V elsewhere) sends the clipboard's text, through `arboard` (decision 49), as one PASTE: tests `a_click_is_the_hosts_only_in_a_mouse_mode`, `the_wheel_and_moves_go_only_when_asked_for`, `paste_is_one_intent_and_composition_only_in_its_mode`, `the_mouse_goes_only_when_the_page_or_channel_asks`. The clipboard itself and the preedit are untested with a real clipboard and input method.

#### REQ-INT-116: International keyboards (NeoTeletext's side)
- **State:** Met in tests: the interface's keys of Russian, Hebrew, German (AltGr), macOS Option, French (dead keys) and AZERTY layouts (`international_layouts`); text and a KEY's text in Normalization Form C, through `unicode-normalization` (decision 49; `text_goes_in_normalization_form_c`); the reference channel's native digits (`the_digits_of_any_script_choose_a_page`). A real input method is untested.

#### REQ-INT-137: No key is logged, and the round trip is measured
- **State:** Met: measured from acknowledgements, at most 256 sends remembered (the engine's `at_most_256_sends_are_remembered`), and shown in the strip; no code that handles input prints or logs, and the keyboard process prints only why it stopped (test `no_input_code_logs`).

#### REQ-INT-138: The phone keyboard
- **State:** As requirement 18.9 stood, with the phone's keys now the protocol's: text as text, named keys their functional codes, no modifiers (test `a_phones_keys_have_no_modifiers`); the keyboard process counts 120 input units a second, a character of text each. The panel has not yet paired with a real phone.

#### REQ-INT-139: Compiled out
- **State:** Met: the commit gate lints the workspace with every optional area out, and with hyperlinks, typing, and both, alone.


## 19. Settings, the home page, the developer console and proxies

From `settings-home-console-proxy.md` and its security analysis (decision 51).

### Requirement 19.1: Settings are kept between runs
- **Statement:** The trusted core must keep the settings of `settings-home-console-proxy.md` section 1.1 and the bookmarks in one file, `settings.txt`, in the platform's place for an application's settings (macOS `~/Library/Application Support/NeoTeletext/`, Windows `%APPDATA%\\NeoTeletext\\`, elsewhere `$XDG_CONFIG_HOME/neoteletext/` or `~/.config/neoteletext/`), write it whole to a temporary file renamed over it, and read it at start, missing meaning the defaults.
- **Reason:** A browser that forgets its home page and bookmarks each run is not a browser.
- **Source:** Decision 51.
- **Verification:** Test: settings written, read back, and a write interrupted before the rename leaves the old file.
- **State:** Met: `settings_round_trip_through_their_file`, `the_file_is_written_whole_or_not_at_all`; at start the trusted core reads it, missing meaning the defaults; by hand on macOS on 21 October 2026, the star wrote `bookmark neoteletext:home` to the file and a later start read it.
- *Amended 22 October 2026 (decision 57):* bookmarks are kept in `bookmarks.txt` beside it (requirement 20.19), and history in `history.txt` (requirement 20.2); `settings.txt` adds `history on|off` and `bar on|off`.
- *Amended 22 October 2026 (decision 59):* `settings.txt` adds `restore on|off`, and the session is kept in `session.txt` beside it (requirement 20.21).
- *Amended 22 October 2026 (decision 62):* `settings.txt` adds `cookies keep|close|block` and `cookie-block` lines, and kept cookies are in `cookies.bin` beside it, sealed (requirement 21.4); the file's bound is 3 MiB, for 1 000 blocked channels.

### Requirement 19.2: The settings file is read strictly
- **Statement:** The settings file must be read as section 1.2 defines it: a version line, at most 256 KB, a line at most its name, a space and a value of 2 048 bytes (amended 21 October 2026 on building it: a line of 2 048 bytes could not hold the longest address the address rules accept, so a bookmark of that length was dropped at the next start), each value checked by its setting's own rule; a line that breaks a rule or names nothing known is dropped and logged, and the rest stand.
- **Reason:** The file is outside input: another program can write it.
- **Source:** Decision 51; `settings-home-console-proxy-security.md` section 3.
- **Verification:** Test of each rule; the fuzz target `settings_file`.
- **State:** Met in tests: `the_file_is_read_strictly`, `at_most_16_local_addresses`, `the_longest_address_is_kept` (which failed with the line bound of 2 048 bytes); the fuzz target `settings_file` checks every bound and that what was read writes a file that reads back the same, in the full gate.
- *Amended 22 October 2026 (decision 57):* a `bookmark` line is no longer a setting: it is dropped and logged like any line naming nothing known (`the_file_is_read_strictly`).

### Requirement 19.3: The settings panel
- **Statement:** The settings must be shown and changed in a panel of the trusted strip, opened by its gear button, Command-comma on macOS and Control-comma elsewhere; each change must be strip input the trusted core checks, a refused one said in the panel; a change of a security setting (the proxy, the local addresses) must be said in the strip's notice and logged. The interface receives the settings only as the text the panel shows.
- **Reason:** Security settings belong in the strip (requirement 11.2); the interface decides nothing (requirement 3.6).
- **Source:** Decision 51.
- **Verification:** Interface and trusted-core tests of each control and each refusal.
- **State:** Met in tests: `each_control_asks_for_its_change`, `the_panel_shows_the_settings_and_the_refusal`, `the_gear_and_its_shortcut_open_the_panel`, `each_setting_change_is_checked` (every rule and refusal), `the_strip_says_the_notice_and_names_the_start_page`. By hand on macOS: Command-comma opens the panel and Escape closes it. Its controls were not pressed by hand: clicks posted to the window do not reach it.

### Requirement 19.4: Sound and its volume
- **Statement:** The sound setting must silence every page's sound, and the volume scale it, from 0 to 100, in the sound helper's mixer (decision 55).
- **Reason:** Requirement 16.5 named the setting and the volume.
- **Source:** Decision 51.
- **Verification:** A mixer test of silence and of each volume's gain.
- **State:** Met in tests: `the_settings_silence_and_scale_every_channel` (sound off silent; volumes 0, 10, 25, 50, 80 and 100 each within 1 per cent of their gain; a muted tab stays muted; a channel that starts later follows the settings). Not yet listened to.

### Requirement 19.5: Withdrawn: Bookmarks are kept
- Withdrawn on 22 October 2026 (decision 57): replaced by requirements 20.15, 20.16 and 20.19, which keep bookmarks with names in folders, the star keeping the page shown.
- **Was:** The star must add or remove the open channel's address, at most 1 000 kept, in the order added, between runs.

### Requirement 19.6: The Home button
- **Statement:** The strip must show a Home button after Forward; it, Command-Shift-H on macOS and Alt-Home elsewhere must open the home address in the active tab as a navigation the user starts (requirement 8.2), and a new tab must open the home address.
- **Reason:** Requirement 11.2 named it; home is where a browser starts.
- **Source:** Decision 51.
- **Verification:** Interface and trusted-core tests.
- **State:** Met: `home_is_asked_for_by_its_button_and_its_keys`, the trusted core's `ToCore::Home` navigation; by hand on macOS, Command-Shift-H opened the start page in the tab. The interface accepts each shortcut on every platform (`settings-home-console-proxy.md` section 7).

### Requirement 19.7: The start page
- **Statement:** The address `neoteletext:home` must be a channel the trusted core serves itself as teletext, with no network: page 100 telling how to open a channel and listing the bookmarks from page 201, and pages 201 to 299 one for each bookmark, which open it as the user's navigation. Any other `neoteletext:` address must be refused. Its text must be built only from addresses the address rules accepted and cleaned (requirement 11.5).
- **Reason:** A home that needs no network, and shows the user's own channels, is always there.
- **Source:** Decision 51.
- **Verification:** Tests of the pages, of choosing a bookmark, and of hostile addresses in bookmarks.
- **State:** Met: `the_start_page_is_the_one_built_in_address`, `the_pages_tell_how_to_open_a_channel_and_list_the_bookmarks`, `a_hostile_address_cannot_break_out_of_its_line`, `the_start_page_opens_its_bookmarks`; by hand on macOS, typing 202 on the start page opened its bookmark, and Back returned to page 100.
- *Amended 22 October 2026 (decision 57):* the start page lists the Bookmarks bar's bookmarks and then Other bookmarks', by name, each folder's name as a heading; a bookmark's page shows its name and its address (`the_pages_tell_how_to_open_a_channel_and_list_the_bookmarks`).

### Requirement 19.8: The developer console
- **Statement:** A panel docked at the bottom of the window, opened by Command-Option-I on macOS and Control-Shift-I elsewhere, must show for the active tab's channel the tabs Network, Packets, Pages and Notices of `settings-home-console-proxy.md` section 3.1, each keeping its newest 1 000 entries a channel, with pause and clear. It must never show a cookie, a form's values, a key typed into a page or the pairing secret.
- **Reason:** Diagnosis of a channel without exposing the trusted core (requirement 12.7).
- **Source:** Decision 51.
- **Verification:** Interface tests of each tab; a test that hostile text and bytes are shown as text.
- **State:** Met in tests: `each_tab_shows_its_entries` (hostile text shown as the text it is), `pause_holds_and_clear_empties`, `its_shortcut_on_each_platform`, `the_console_takes_the_overviews_place`; by hand on macOS, Command-Option-I docked it with the reference channel's exchanges and push events. No cookie, form value, typed key or pairing secret reaches the trusted core's console entries (review of `console_fetched` and `decoded`).
- *Amended 23 October 2026 (decision 63):* a fifth tab, Cookies, holds the cookies panel of requirement 21.6, whose rows the trusted core sends only while that tab is shown with the console open; Pause and Clear are the other tabs' and are not shown on it. "Never shows a cookie" stays: the tab shows the rows' counts, sizes and last use, never a name or value. State: built; `the_search_asks_for_rows` checks that closing the console stops the rows and opening it asks for them again; seen on macOS.
- *Amended 23 October 2026 (decision 70):* the Notices tab becomes **Log**, as a web browser's console: every line has a level (error, warning, information), shown in red, yellow or white, the browser's own lines (each process's confinement, the settings, history, bookmarks and session read at start, the cookie file's key, a helper that stopped) shown in every channel's Log beside the channel's own; buttons show or hide each level and a search keeps the lines that hold its text, in any case. The channel's own log lines, a TTI page's `LG` lines (the engine's E.109), join it marked "channel" with their page, cleaned, at most 64 held between two of the decoder's reports. State: met, `the_log_filters_by_level_and_search`, `the_browsers_lines_show_in_every_channels_log`, `a_channels_log_lines_are_taken_with_their_page`.

### Requirement 19.9: The console's data
- **Statement:** The fetcher must report each exchange and each push event to the trusted core (the engine's REQ-WEB-033), bounded and read by a reader that refuses anything else; the trusted core must read each packet's address with the engine, keep each tab's entries, and send the open tab's as cleaned text, at most 200 entries a message, only while the console is open.
- **Reason:** The data is the channel's; it must reach the interface as text, never as anything it acts on.
- **Source:** Decision 51.
- **Verification:** Protocol tests of the bounds; the fuzz target `fetcher_messages`; a trusted-core test of the packet entries.
- **State:** Met: `FromFetcher::Exchange` and `ProxyFailed` bounded and refused when malformed (the fetcher protocol's `every_kind_round_trips` and `malformed_contents_are_refused`), read by the fuzz target `fetcher_messages` in the full gate; `packets_are_read_with_the_engine_and_shown_as_text`, `each_tab_keeps_its_newest_1000`, `only_new_entries_go_and_only_while_open`.

### Requirement 19.10: Proxies
- **Statement:** The fetcher's and the phone keyboard's connections must go through the proxy the settings name: an HTTP proxy by `CONNECT`, or a SOCKS5 proxy with no authentication sending the host's name (the engine's REQ-WEB-032). A proxy that refuses, fails or does not answer in time must fail the connection, with the reason in the strip's notice, and nothing must go direct instead.
- **Reason:** Many networks reach the web only through a proxy; a fallback would leak what the user chose to send through it.
- **Source:** Decision 51.
- **Verification:** Tests against a local HTTP proxy and a local SOCKS5 proxy, of success, refusal and silence.
- **State:** Met: `a_connection_goes_through_an_http_proxy`, `a_connection_goes_through_a_socks5_proxy`, `a_refusal_fails_the_connection`, `a_silent_proxy_fails_in_time_and_holds_nothing_up` against proxies on this computer; by hand on macOS on 21 October 2026 through a local HTTP proxy, the fetch and push connections were tunnels, and with the proxy stopped the strip said "The proxy refused the connection to 127.0.0.1" and nothing went direct. The phone keyboard takes `--proxy` (`the_keyboard_reads_its_relay_pin_and_proxy`); its relay through a proxy is untested. Since 22 October 2026 (decision 53) a connection, its tunnel and its handshake are opened on a thread of their own and the agent's connect limit bounds them, so a silent proxy no longer holds the fetcher's loop (`a_silent_proxy_fails_in_time_and_holds_nothing_up`, which failed on the earlier code).

### Requirement 19.11: The local network through a proxy
- **Statement:** With a proxy, an address written as a local IP address, `localhost` and names under `.local` and `.internal` must be refused unless allowed (requirement 5.10), before anything is sent to the proxy.
- **Reason:** The proxy resolves names, so the browser cannot see where every name leads; it keeps the rule where it can.
- **Source:** Decision 51; `settings-home-console-proxy-security.md` section 3.
- **Verification:** Test of each refused form.
- **State:** Met by the engine's agent, which the fetcher runs with `proxied` set: the engine's `local_hosts_are_seen_before_the_proxy` (each refused form) and `through_a_proxy_local_names_are_refused_and_the_proxy_may_be_local` (the agent refuses before connecting), REQ-WEB-032.

### Requirement 19.12: The fetcher takes its proxy only when started
- **Statement:** The trusted core must give a fetcher its proxy as a command-line argument when it starts it, and a change of the proxy must take effect for fetchers started after it; no message may change where a running fetcher connects (requirement 4.7).
- **Reason:** Requirement 4.7: nothing the trusted core sends a running fetcher changes what it fetches.
- **Source:** Decision 51.
- **Verification:** Review; a test that the fetcher reads its proxy argument.
- **State:** Met: `the_fetcher_reads_its_proxy_argument`, `a_fetcher_is_given_the_proxy_and_the_local_addresses`, `the_keyboard_reads_its_relay_pin_and_proxy`; review: no message to a running fetcher carries a proxy (`ToFetcher` has input messages only).

## 20. History and bookmarks

Adopted and built on 22 October 2026 by decision 57, while the owner was away, as the owner instructed; the design is `history-and-bookmarks.md`, its security analysis `history-and-bookmarks-security.md`.

### Requirement 20.1: Visits are recorded
- **Statement:** The trusted core must record each page a tab shows, as one row per address and local day, with the channel's and page's titles cleaned, the last visit's time, the day's visits and how many the user typed; a visit is a navigation recorded when the tab draws that page's first frame, and a pushed update, a subpage or a redraw is not one. The browser's own `neoteletext:` pages, private tabs and everything while "Remember history" is off must not be recorded.
- **Reason:** History is the record of what the user saw, and only that.
- **Source:** Decision 57.
- **Verification:** Test of each kind of navigation, of a page that never draws, of updates, of the start page, of a private tab and of the setting.
- **State:** Met, 22 October 2026: `a_visit_is_recorded_once_its_page_draws` (recorded on the first frame drawn, once; not in a private tab, not the start page, not while history is not remembered), `navigations_wait_to_be_recorded` (each kind of navigation waits, typed or not; within a channel at once), `a_page_is_one_row_a_day`; by hand on macOS, a channel visited wrote its row to `history.txt`. A page chosen on the wall by its number or a click is the interface's own and not a navigation, so not a visit (`history-and-bookmarks.md` section 14).
- *Amended 23 October 2026 (decision 63):* a reload (requirement 11.6) moves its row's last visit to the time its page draws, and makes the day's row if there is none, but adds to neither the day's visits nor those typed, as Chrome and Firefox count no reload (Chromium's `HistoryBackend::AddPageVisit`; Firefox's `VISIT_COUNT_INC` in `nsPlacesTriggers.h`); suggestions, ranked by visits, are not raised by reloading. State: met, `a_reload_moves_the_last_visit_and_counts_none`, which fails when a reload is counted.
- *Amended 23 October 2026 (decision 65):* "Remember history" is withdrawn (requirement 20.7): every page of an ordinary tab is recorded; private tabs and the browser's own pages are not.

### Requirement 20.2: History is kept for 90 days
- **Statement:** History must be kept between runs in `history.txt` in the settings folder, readable and writable by the user alone on macOS and Linux, appended at each visit and written whole to a temporary file renamed over it at start, at quit, when it holds twice as many lines as rows, and at once after any removal; rows older than 90 days, beyond 50 000, or beyond a file of 16 MiB must be removed, the oldest first.
- **Reason:** A browser's history outlives a run, within bounds that keep start fast and the record from growing forever.
- **Source:** Decision 57.
- **Verification:** Tests of round trip, of each bound, of a broken last line, and of compaction.
- **State:** Met in tests, 22 October 2026: `history_round_trips_through_its_file` (appended, read back, compacted, written readable by the user alone), `history_is_kept_ninety_days_and_bounded`; by hand on macOS the file was created `-rw-------`.

### Requirement 20.3: The history file is read strictly
- **Statement:** `history.txt` must be read as section 2.3 of `history-and-bookmarks.md` defines it: a version line, at most 16 MiB, each line seven tab-separated fields each within its bound and rule; a line that breaks a rule is dropped and logged, and the rest stand.
- **Reason:** The file is outside input: another program can write it.
- **Source:** Decision 57.
- **Verification:** Test of each rule; the fuzz target `history_file`.
- **State:** Met: `the_file_is_read_strictly` (each rule, a broken last line); the fuzz target `history_file` reads anything, keeps the bounds, and what it read writes a file that reads back the same (20 seconds, no fault, 22 October 2026).

### Requirement 20.4: The history panel
- **Statement:** A panel of the trusted strip, opened by Command-Y on macOS and Control-H elsewhere and from the menus, must show history newest first under a heading for each day, each row with its time, channel title, page number, page title and address, with a search of addresses and titles; choosing a row must open it as a navigation the user starts, in the active tab or a new one.
- **Reason:** The browser-style view of what was seen.
- **Source:** Decision 57.
- **Verification:** Interface and trusted-core tests.
- **State:** Met in tests: `the_panel_shows_rows_and_opens_them_by_number`, `the_search_and_the_wheel_ask_for_rows`; by hand on macOS, Command-Y opened it with the day's heading and the row's time, title and address. A row opens in a new tab with Shift-click, Command-click (Control-click elsewhere) or the middle button (`the_middle_button_opens_a_row_in_a_new_tab`, decision 71).
- *Amended 23 October 2026 (decision 64):* each row has Chrome's menu, "More from this site", which makes the search `host:` and the row's host, "Remove from history", and "Remove bookmark", which removes every bookmark of the address, when it is bookmarked (`a_rows_bookmarks_are_removed_from_its_menu`), in place of its remove button; a search beginning `host:` answers the rows of that host alone, in any case. State: met, `rows_are_removed_by_number` (the menu's two items), `history_is_removed_as_asked` (`host:`).

### Requirement 20.5: The interface sees only history on screen
- **Statement:** The trusted core must send the history panel at most 100 rows at a time, each with a number for that view, and must act on the interface's requests to open or remove rows only by those numbers.
- **Reason:** Requirement 3.6: a compromised interface finds nothing beyond what is on screen, and cannot name an address of its own through history.
- **Source:** Decision 57.
- **Verification:** Test that no view holds more than 100 rows and that an unknown or stale number is refused.
- **State:** Met: `rows_are_removed_by_their_numbers_and_from_the_file` (a number from an earlier view is refused), `the_largest_views_fit_one_message`.

### Requirement 20.6: History can be removed
- **Statement:** The user must be able to remove a row, selected rows, every row of a channel, and every row whose last visit is within the last hour, 24 hours, 7 days, 4 weeks or ever; removing everything must also empty recently closed tabs and the typed record; removed rows must be gone from `history.txt` before the panel shows them removed.
- **Reason:** The user owns their history.
- **Source:** Decision 57.
- **Verification:** Tests of each, reading the file after each.
- **State:** Met: `history_is_removed_as_asked`, `rows_are_removed_by_their_numbers_and_from_the_file` (the file read after each removal), `rows_are_removed_by_number`.
- *Amended 22 October 2026 (decision 62):* the clearing offers cookies, alone or with history (requirement 21.8).
- *Amended 23 October 2026 (decision 64):* as Chrome removes them (`browsing_data::RemoveNavigationEntries`), removing history also removes from every tab's back-and-forward list, but never the entry a tab is at, and from the recently closed tabs, a closed tab going whole when the entry it was at is removed: for a time, the entries the tab came to in it; for rows, the entries of addresses no day has a row of any more. Private tabs are not touched. The ranges begin with Chrome's last 15 minutes. State: met, `removed_history_leaves_the_tabs_lists`.
- *Amended 23 October 2026 (decision 65):* "every row of a channel" is withdrawn with "Forget this channel", which Chrome does not have; a site's rows are found with "More from this site" (requirement 20.4), selected and removed. The protocol's request code 5 is not reused.

### Requirement 20.7: Withdrawn: "Remember history"
Withdrawn on 23 October 2026 (decision 65): history follows Chrome (decision 64), which has no such setting; a user who wants no record browses in a private tab (requirement 20.8). Removed from the settings panel, `settings.txt` and the protocol, whose change code 8 is not reused; a `history` line in an older settings file is dropped and logged as an unknown line (`the_file_is_read_strictly`).
- **Was:** A setting, on by default and kept in `settings.txt`, must stop all recording when off, and turning it off must offer to clear what is kept.

### Requirement 20.8: Private tabs
- **Statement:** A private tab, opened by Command-Shift-N on macOS and Control-Shift-N elsewhere, must record no history, no typed address and no recently closed entry, and must be marked "Private" in the strip and in its tab.
- **Reason:** Browser-style browsing that leaves no record.
- **Source:** Decision 57.
- **Verification:** Test of each, and that the mark is shown.
- **State:** Met: `closed_tabs_reopen_where_they_were` (not kept), `navigations_wait_to_be_recorded` (marked), `a_visit_is_recorded_once_its_page_draws` (not recorded); by hand on macOS, Command-Shift-N opened a tab with the mark in the tab and "Private" in the strip, and nothing was recorded.

### Requirement 20.9: Recently closed tabs
- **Statement:** The trusted core must keep the last 25 closed tabs, other than private ones, in memory with their whole back-and-forward lists; Command-Shift-T on macOS and Control-Shift-T elsewhere must reopen the most recent where it was.
- **Reason:** Closing a tab by mistake must not lose it.
- **Source:** Decision 57.
- **Verification:** Test.
- **State:** Met: `closed_tabs_reopen_where_they_were`, `rows_are_removed_by_number` (reopened from the panel).
- *Amended 23 October 2026 (decision 64):* the closed tabs are kept in `session.txt` with the session and come back at the next start, as Chrome keeps them (`TabRestoreServiceImpl`). State: met, `closed_tabs_are_kept_between_runs`.

### Requirement 20.10: Back and Forward lists
- **Statement:** Holding Back or Forward, or a secondary click on it, must list at most 15 of the tab's entries that way, nearest first; choosing one must move the tab there.
- **Reason:** Browser-style navigation through a tab's list.
- **Source:** Decision 57.
- **Verification:** Interface and trusted-core tests.
- **State:** Met: `back_and_forward_lists_go_several_steps`, `a_list_entry_goes_there`, `back_held_opens_its_list` (held 500 ms or a secondary click).
- *Amended 23 October 2026 (decision 64):* the list holds at most 12 entries, Chrome's `kMaxHistoryItems`; when it is full, up to 5 chapter stops follow it after a rule, each the nearest entry beyond where the host changes, as Chrome's `kMaxChapterStops` and `FindChapterStop`, the host standing for Chrome's registrable domain since decision 62 dropped the Public Suffix List; then "Show all history". A tab keeps at most 50 entries, the oldest dropped, Chrome's `kMaxSessionHistoryEntries`. State: met, `back_lists_twelve_then_chapter_stops_and_a_tab_keeps_fifty`, `back_held_opens_its_list` (a chapter stop goes there).

### Requirement 20.11: Suggestions in the address field
- **Statement:** While the user types an address, the trusted core must offer at most 8 suggestions from bookmarks, then from history, each shown with its address as the address rules print it; an inline completion must come only from addresses the user typed before.
- **Reason:** What was seen before is quicker to reach; a channel must not choose what the field completes to.
- **Source:** Decision 57.
- **Verification:** Tests of the order, the bound, and that a link followed or a title never completes the field.
- **State:** Met: `completion_comes_only_from_what_was_typed`, `suggestions_come_from_bookmarks_then_history`, `typing_asks_for_suggestions_and_completes_inline`.

### Requirement 20.12: History never reaches a channel
- **Statement:** No history row, title, count or typed address may be sent to any decoder, fetcher or phone keyboard.
- **Reason:** A channel must not learn what else the user has seen (requirement 7.3).
- **Source:** Decision 57.
- **Verification:** Review of every message to those processes.
- **State:** Met by review, 22 October 2026: history is held by the trusted core's `library` module and sent only in `ToInterface` views; no `ToDecoder`, `ToFetcher` or keyboard message carries a row, title, count or typed address.

### Requirement 20.13: Hostile titles stay text
- **Statement:** Every channel title, page title and bookmark name must be cleaned and cut wherever it is shown, and shown beside the address it leads to, never in the address's place.
- **Reason:** Titles are the channel author's text.
- **Source:** Decision 57.
- **Verification:** Tests with hostile titles in every place they are shown.
- **State:** Met in tests: every view shows the address beside the title (`the_panel_shows_rows_and_opens_them_by_number`, `items_open_by_number`); titles are cleaned and cut when kept (`the_file_is_read_strictly`) and on the start page (`a_hostile_address_cannot_break_out_of_its_line`, hostile names too).

### Requirement 20.14: History and bookmarks do not slow the start
- **Statement:** With 50 000 history rows and 10 000 bookmarks, the time from start to the first frame must not grow by more than 50 milliseconds over a start with none.
- **Reason:** The browser must open at once.
- **Source:** Decision 57.
- **Verification:** Measurement, release build, recorded with its date.
- **State:** Met, measured 22 October 2026 on macOS, release build, the reference channel: from launch to the first page frame, five starts each, median 375 ms with no files and 318 ms with 50 000 history rows and 10 000 bookmarks (the files are read before the trusted core's first message to the interface, so they are on the measured path). The measurement found the bookmarks reader keeping only 9 998 of 10 000 (`the_most_bookmarks_are_read_whole` failed on it); fixed.

### Requirement 20.15: Bookmarks and folders
- **Statement:** Bookmarks must have a name and an address, and folders a name and an order of bookmarks and folders; the folders Bookmarks bar and Other bookmarks must always exist; at most 10 000 bookmarks and folders together, 16 folders deep, a name of 256 bytes and an address of 2 048.
- **Reason:** Browser-style bookmarks, bounded.
- **Source:** Decision 57.
- **Verification:** Tests of each bound.
- **State:** Met: `bookmarks_live_in_folders`, `bookmarks_are_bounded`.

### Requirement 20.16: The star
- **Statement:** The star must be filled when the strip's address is bookmarked; clicking an empty star, Command-D on macOS or Control-D elsewhere must bookmark the address the strip shows, page included, and open a popover to change its name and folder, keep the whole channel instead, or remove it.
- **Reason:** Browser-style bookmarking of what is shown.
- **Source:** Decision 57.
- **Verification:** Interface and trusted-core tests.
- **State:** Met: `the_star_bookmarks_the_page_shown`, `the_popover_asks_for_each_change`; by hand on macOS, Command-D opened the popover with the channel's name in Other bookmarks, and Return closed it.

### Requirement 20.17: The bookmarks manager
- **Statement:** A panel of the strip, opened by Command-Option-B on macOS and Control-Shift-O elsewhere and from the menus, must show the folder tree and a folder's contents, search names and addresses, and open, edit, remove (with Undo for ten seconds), add folders, move and sort; the trusted core must send it at most 100 items at a time with view numbers and act only by those numbers.
- **Reason:** Bookmarks must be managed, and requirement 3.6 holds for them as for history.
- **Source:** Decision 57.
- **Verification:** Interface and trusted-core tests.
- **State:** Met in tests: `the_manager_edits_by_number`, `bookmarks_are_managed`, `items_open_by_number`, `the_manager_asks_for_each_change` (a drag onto a folder moves); by hand on macOS, Command-Option-B opened it. A move is by dragging, or by a row's "Move…" and the folder chosen (`move_to_chooses_a_folder`, decision 71).

### Requirement 20.18: The bookmarks bar
- **Statement:** An optional row of the strip, off by default and shown or hidden by Command-Shift-B on macOS and Control-Shift-B elsewhere, must show the Bookmarks bar folder, drawn from the trusted core's view.
- **Reason:** Browser-style quick access, kept in the trusted strip.
- **Source:** Decision 57.
- **Verification:** Interface test.
- **State:** Met: `the_bar_shows_its_folder`, `the_bar_opens_its_bookmarks`; by hand on macOS, Command-Shift-B showed it below the address and wrote `bar on`.

### Requirement 20.19: Bookmarks are kept
- **Statement:** Bookmarks must be kept in `bookmarks.txt` in the settings folder, written whole to a temporary file renamed over it at each change.
- **Reason:** As requirement 19.1, for bookmarks.
- **Source:** Decision 57.
- **Verification:** Tests of round trip and of an interrupted write.
- **State:** Met: `bookmarks_round_trip_through_their_file`; by hand on macOS the file was created `-rw-------`.

### Requirement 20.20: The bookmarks file is read strictly
- **Statement:** `bookmarks.txt` must be read as section 3.6 of `history-and-bookmarks.md` defines it: a version line, at most 24 MiB, unique numbers, each folder before its contents so no cycle can be read, depth at most 16, each field within its bound and rule; a line that breaks a rule is dropped and logged with anything inside it.
- **Reason:** The file is outside input.
- **Source:** Decision 57.
- **Verification:** Test of each rule; the fuzz target `bookmarks_file`.
- **State:** Met: `the_file_is_read_strictly` (bookmarks; no cycle, depth, each rule), `the_most_bookmarks_are_read_whole`; the fuzz target `bookmarks_file` (every item reached once from the two folders, every bound, round trip; 20 seconds, no fault).

### Requirement 20.21: The session is always kept
- **Statement:** The trusted core must keep the window's place, size and maximised state and every tab but private ones, each with its back-and-forward list (at most 50 entries, nearest its place), its place and its mute, and which tab is active, in `session.txt` in the settings folder, readable by the user alone, written whole after every change of the tabs and within a second of the window's last move or resize.
- **Reason:** The owner's instruction: the system must always be able to restore the current state of windows and tabs, whatever ends it.
- **Source:** Decision 59.
- **Verification:** Tests: each kind of change writes the file; private tabs are never in it; a write stopped before its rename leaves the last one.
- **State:** Met: `the_session_is_kept_after_every_change` (a mute, a page drawing with its title, the window only once still, a private tab never), `the_session_is_kept_and_restored`, `the_session_round_trips_through_its_file` (mode 0600), `a_long_list_is_kept_around_its_place`; a temporary file left before its rename is ignored (`the_session_round_trips_through_its_file`). By hand on 22 October 2026: on macOS the browser killed with `SIGKILL` left `session.txt` with both tabs, without the private tab, and the window's place; on Linux and Windows likewise (requirement 20.22).
- *Amended 23 October 2026 (decision 64):* each entry keeps the time the tab last came to it, and the recently closed tabs are kept too (requirement 20.9); the file is version 2, and a version 1 file is not read, so the session of a build before is not restored once. State: met, `the_session_round_trips_through_its_file`.
- *Amended 23 October 2026 (decision 72):* every window is kept, each with its place, size, active tab and tabs, at most 16 windows and 64 tabs; the closed tabs of a window that closed whole carry its number; the file is version 3, and a version 2 file is not read. State: met, `every_window_is_kept_and_restored`, `a_closed_window_reopens_whole`.

### Requirement 20.22: The session is restored at start
- **Statement:** With "Restore previous session" on, the browser must start with the kept session's window, tabs, lists and active tab, the command line's addresses opening as further tabs, the last of them active; with it off or with no session, as before.
- **Reason:** The owner's instruction: restoring is what happens when the application starts.
- **Source:** Decision 59.
- **Verification:** Tests of the restore, with and without the setting and with command-line addresses; a demonstration on each desktop.
- **State:** Met in tests: `the_tabs_at_start` (as it was; addresses after it, the last active; off, as before; nothing kept, the home address), `the_session_is_kept_and_restored`. By hand on macOS on 22 October 2026: the next start had both tabs, the second active, and the window at the place and size written into the file (300, 200, 1100 by 700); with restoring off, one tab at the home address. On Linux (X11 under Xvfb, foundintelquiet4060) and on Windows (the virtual machine's desktop), the same day: a browser killed with both tabs open kept them in `session.txt` (0600 on Linux), and the next start restored them with the window at the place and size written into the file (`xwininfo` 1100×700+200+100; `GetWindowRect` 150, 120, 1100 by 700).
- *Amended 23 October 2026 (decision 72):* every kept window is restored, each where it was. State: met in tests, `every_window_is_kept_and_restored`; by hand on macOS, two windows restored at their places (`multiple-windows.md` section 8).

### Requirement 20.23: The session file is read strictly
- **Statement:** `session.txt` must be read as `history-and-bookmarks.md` section 15.4 defines it, every field within its bound and rule; what breaks a rule is dropped and logged.
- **Reason:** The file is outside input.
- **Source:** Decision 59.
- **Verification:** Test of each rule; the fuzz target `session_file`.
- **State:** Met: `the_file_is_read_strictly` (session; the exact lines and tabs dropped); the fuzz target `session_file` (every bound, round trip; 2.6 million runs from a seeded file, no fault). A restored address passes the address rules again (`Session::restore`).
- *Amended 23 October 2026 (decision 64):* version 2: an entry's time must be a whole number of seconds from 1970 or later, and a `closed` line begins a closed tab as `tab` begins a tab. State: met, `the_file_is_read_strictly`.
- *Amended 23 October 2026 (decision 72):* version 3: a `window` line begins each window (`history-and-bookmarks.md` section 15.4). State: met, `the_file_is_read_strictly`.

### Requirement 20.24: Restoring is a setting
- **Statement:** The settings must include "Restore previous session", on by default, kept in `settings.txt` and changed in the settings panel.
- **Reason:** The owner's instruction.
- **Source:** Decision 59.
- **Verification:** Test.
- **State:** Met: `restore on|off` in `settings.txt` (`settings_round_trip_through_their_file`), the panel's row "Tabs at start: Restore, Home" (`each_control_asks_for_its_change`), checked as strip input (`each_setting_change_is_checked`); seen on macOS.

### Requirement 20.25: Bookmarks from another browser
- **Statement:** The bookmarks manager's Import must read another browser's Netscape bookmark file as well as this browser's own, into "Imported" under Other bookmarks, its folders kept. The file must be read by a decoder process started for it, confined as every decoder is, never by the trusted core; the decoder must report only folders, their ends and bookmarks, each bounded; the trusted core must keep only addresses its address rules accept, clean every name, and keep the bounds of requirement 20.15.
- **Reason:** Every browser writes the file, and the owner asked for the to-do list's item done; the trusted core reads no outside format (requirement 3.2).
- **Source:** Decision 71.
- **Verification:** Tests, and fuzzing of the reader.
- **State:** Met (23 October 2026): `a_browsers_file_gives_its_folders_and_bookmarks_in_order`, `folders_deeper_than_the_bookmarks_keep_join_the_folder_around_them`, `lengths_and_counts_are_bounded` and four more in `decoder/src/netscape.rs`; `a_confined_decoder_reads_another_browsers_bookmarks` (a Firefox file, the decoder under its sandbox); `another_browsers_bookmarks_come_in_as_channels_only` (`place:` and `javascript:` refused, a hidden character cleaned); `a_bookmark_entry_is_held_to_its_bounds`; the fuzz target `netscape_file` (1.77 million runs without a fault on 23 October 2026). Exporting in that format is not built: this browser's bookmarks are channels, which another browser opens only as web pages.

---

## 21. Cookies

**Proposed and built 22 October 2026 (decision 62, `cookies.md`, with `cookies-security.md`).**

### Requirement 21.1: The trusted core owns the cookies
- **Statement:** Every cookie must be kept by the trusted core, one jar per channel under the engine's rules (REQ-WEB-006, REQ-WEB-026); a fetcher must hold none, receiving for each request only that request's `Cookie` header and returning each `Set-Cookie` line to the trusted core.
- **Reason:** A fetcher compromised by its channel's server then sees only the cookies of requests in flight (requirement 7.5).
- **Source:** Decision 62.
- **Verification:** Test: the fetcher has no jar; a request carries the trusted core's header; a `Set-Cookie` line reaches the trusted core's jar.
- **State:** Met, 22 October 2026: `each_fetcher_is_answered_with_its_channels_cookies` (a fetcher of another channel, or of none, asking for this host is answered with nothing; one cannot plant another channel's cookie), `a_channels_cookies_stay_in_the_channel`, `the_fetcher_keeps_no_cookie` (the agent's own jar holds none, `CookieJar<0, 0>`), and the engine's `a_port_keeps_the_cookies` (E.104); `the_agent_asks_for_each_requests_cookies` (the engine's `cookies` feature compiled in; it fails without); the fuzz targets `fetcher_messages` and `fetcher_commands` read the questions and answers. By hand on 22 October 2026 on macOS, Linux (Xvfb) and Windows (the virtual machine's desktop), against a local channel whose server sets a persistent and a session cookie: every request after the first carried both.

### Requirement 21.2: Tabs on a channel share its cookies
- **Statement:** Every ordinary tab showing a channel must use the same jar.
- **Reason:** Decision 11, requirement 7.4.
- **Source:** Decision 62.
- **Verification:** Test with two tabs.
- **State:** Met: `each_fetcher_is_answered_with_its_channels_cookies` (a second tab on the channel is answered with the first's cookie).

### Requirement 21.3: Private tabs have their own cookies
- **Statement:** Private tabs must share one jar per channel, separate from the ordinary jars, held in memory only and emptied when the last private tab closes.
- **Reason:** What a private window means in every browser.
- **Source:** Decision 62.
- **Verification:** Test.
- **State:** Met: `each_fetcher_is_answered_with_its_channels_cookies` (a private tab's cookie not in the ordinary tabs, gone with the last private tab), `private_and_blocked_cookies`.

### Requirement 21.4: Only persistent cookies are kept, and only encrypted
- **Statement:** Between runs only unexpired cookies with a lifetime may be kept, never session cookies, private cookies or cookies of a blocked channel, in `cookies.bin`, readable by the user alone, sealed under a key kept in the platform's secret store; when the key cannot be had, nothing is written.
- **Reason:** A cookie file is a set of credentials; decision 58 forbids a weaker mode.
- **Source:** Decision 62.
- **Verification:** Tests: round trip; a file unreadable without the key; no file written when the key is refused; no session cookie in the file.
- **State:** Met: `the_file_is_sealed` (no cookie or channel in the clear, another key or a changed byte refused, a fresh nonce, mode 0600), `kept_cookies_are_written_sealed_and_removed_on_close` (no session cookie written), `without_the_key_nothing_is_written`. The key is kept by the broker with `keyring` 4.2.0 in the Keychain, the Credential Manager or the Secret Service (`cookies.md` section 13 item 7). By hand on 22 October 2026 on macOS (a keychain of the test's own), Linux (GNOME Keyring as the Secret Service) and Windows (the Credential Manager): `cookies.bin` sealed, mode 0600 on macOS and Linux, no cookie or host in it in the clear, the key in the platform's store; after the browser was killed and started again, its first requests carried the persistent cookie and not the session cookie.

### Requirement 21.5: The cookie file is read strictly
- **Statement:** `cookies.bin` must be read with every field within its bound, each cookie passed through the engine's rules again; what breaks a rule is dropped and logged.
- **Reason:** The file is outside input once decrypted, since another program of the user can replace it with the key.
- **Source:** Decision 62.
- **Verification:** Test of each rule; the fuzz target `cookie_file`.
- **State:** Met: `the_file_keeps_persistent_cookies_of_named_channels` (each broken channel line dropped, cookies of no channel dropped, a store that never kept a cookie reads its own file); the fuzz target `cookie_file`, which found the last of these before release (`cookies.md` section 15 item 10).

### Requirement 21.6: The cookies panel
- **Statement:** A panel must list the channels and sites with cookies, with their counts, sizes and last use, never a cookie's name or value, and must delete a channel's cookies, a site's, or all, and block or unblock a channel, by view numbers alone.
- **Reason:** The control every browser gives, without giving the interface any cookie (requirement 3.6).
- **Source:** Decision 62.
- **Verification:** Interface test; a test that no view carries a name or value.
- **State:** Met: `the_panel_lists_deletes_and_blocks` (no name or value in the view; delete by number; a number of an earlier view names nothing), `the_panel_lists_and_asks_by_number` (interface), `views_and_requests_round_trip` (the largest view fits a message).
- *Amended 23 October 2026 (decision 63):* the panel is the developer console's Cookies tab (requirement 19.8), opened there and by the settings panel's "Manage…", which opens the console at it; the menu's "Cookies and site data" is removed. Its rows, requests and the absence of any name or value are unchanged. State: met, `the_panel_lists_and_asks_by_number` and `the_search_asks_for_rows` updated to the console; seen on macOS.

### Requirement 21.7: The cookie settings
- **Statement:** The settings must offer Keep, Delete when the browser closes, and Block all, Keep by default, and a list of blocked channels, kept in `settings.txt`.
- **Reason:** As in every browser.
- **Source:** Decision 62.
- **Verification:** Test of each.
- **State:** Met: `settings_round_trip_through_their_file` (`cookies close`, a `cookie-block` line), `each_control_asks_for_its_change` (Keep, On close, Block and Manage…), `private_and_blocked_cookies`, `kept_cookies_are_written_sealed_and_removed_on_close` (On close removes the file).

### Requirement 21.8: Cookies are cleared with history
- **Statement:** The history panel's clearing must offer cookies, alone or with history, for its time ranges, by the cookies' creation time.
- **Reason:** Browsers' "Clear browsing data".
- **Source:** Decision 62.
- **Verification:** Test.
- **State:** Met: `rows_are_removed_by_number` (History and Cookies switches beside the ranges), `clearing_with_history_removes_cookies_of_the_range`, `listing_and_deleting` (by creation time).

### Requirement 21.9: `Clear-Site-Data` empties the channel's own jar
- **Statement:** A response from the channel over `https` naming `"cookies"` or `"*"` in `Clear-Site-Data` must empty that channel's jar and no other; over plain HTTP it must be ignored.
- **Reason:** The current way for a server to sign a user out everywhere; limited to the channel by REQ-WEB-026.
- **Source:** Decision 62.
- **Verification:** Test.
- **State:** Met: the engine's `clear_site_data_names_cookies` and `clear_site_data_clears_before_the_response_sets` (over plain HTTP ignored), and `listing_and_deleting` (the channel alone, never over plain HTTP).

### Requirement 21.10: Cookies are bounded
- **Statement:** At most 180 cookies a channel and 3 000 in all, each within the engine's sizes and 400 days, the file within 32 MiB, and the `Set-Cookie` lines from a fetcher within the engine's bounds on one response's headers (16 KiB, 64 headers).
- **Reason:** Bounded memory and input (the baseline instructions).
- **Source:** Decision 62.
- **Verification:** Test at each bound.
- **State:** Met: `cookies_are_bounded` (180 a channel, the oldest going first; 3 000 in all), `cookie_messages_are_bounded`; the file's bound in `cookies::load`; the engine's tests of its sizes and its 400 days.

### Requirement 21.11: Legacy cookie mechanisms are not supported
- **Statement:** `Set-Cookie2`, `Cookie2` and `$`-prefixed attributes must be ignored, `Domain` must not widen a cookie (the engine's host-only rule), and no cookie may be shared across schemes, ports or channels.
- **Reason:** Section 1's table.
- **Source:** Decision 62.
- **Verification:** Test.
- **State:** Met: `a_channels_cookies_stay_in_the_channel` (another folder, scheme or port has none); the engine reads `Set-Cookie` alone of the cookie headers and ignores unknown attributes and `Domain` (its REQ-WEB-006 tests).

---

## 22. Fastext, reveal, hold, mix and sub-pages

From `page-controls.md` (decision 67, taken while the owner was away, on the owner's instruction "implement fastext, reveal, subpages").

### Requirement 22.1: Fastext
- **Statement:** While a page is open, its four coloured keys and Index must be followable by `r`, `g`, `y`, `c` (or F1 to F4) and `i`, as the engine's viewer takes them, the targets read by the engine from the page's packet X/27/0, which it writes from a TTI page's `FL` and `LN` lines too; a page of the channel must open at once, at the sub-page the link names; a page of a service the channel's descriptor lists must be asked about in the strip first (requirement 8.4).
- **Reason:** The coloured keys are teletext's own navigation; the engine has them (its REQ-NAV-003).
- **Source:** Decision 67.
- **Verification:** Tests of the decoder (targets from a link line), the trusted core (following, another service, a key leading nowhere) and the interface (the keys).
- **State:** Met, 23 October 2026: `fastext_comes_from_the_link_line_and_needs_row_24`, `fastext_keys_follow_as_links_do`, `the_viewers_keys_ask_for_the_controls`. Since 23 October 2026 the decoder draws the receiver's row 24 of a page with `LN` lines (the engine's REQ-NAV-011, with its `links::receiver_links`, E.108), a link to another service blank (`link_lines_draw_the_receivers_row_24`).

### Requirement 22.2: The keys act only while their row is shown
- **Statement:** A coloured key or Index must do nothing on an image of the page whose row 24 was blank or boxed away, as the engine's viewer decides it (its REQ-SEC-024), and a key must be checked against the image the user was shown, as a click is (requirement 12.3).
- **Reason:** A page could otherwise hide where its keys lead.
- **Source:** Decision 67.
- **Verification:** Test.
- **State:** Met: `fastext_keys_follow_as_links_do` (row 24 hidden: no key acts), `fastext_comes_from_the_link_line_and_needs_row_24`.

### Requirement 22.3: Reveal
- **Statement:** `?` must show the open page's concealed text and hide it again (the engine's REQ-L1-010), as strip input; it must lapse when another page is opened.
- **Reason:** Quiz answers and punchlines are concealed until the viewer asks.
- **Source:** Decision 67.
- **Verification:** Tests: a concealed row drawn only while revealed; the view forgotten with the page.
- **State:** Met: `reveal_and_mix_change_the_page_drawn`, `a_view_belongs_to_its_open_page`.

### Requirement 22.4: Hold and mix
- **Statement:** `H` must hold the open page at the version it shows, a new version or sub-page not drawn until it is let go (REQ-NAV-006); `m` must draw its cells' backgrounds transparent (REQ-L1-020); both lapse when another page is opened.
- **Reason:** A rotating or changing page cannot be read otherwise; subtitle and newsflash pages are made for mix.
- **Source:** Decision 67.
- **Verification:** Decoder tests.
- **State:** Met: `a_held_page_keeps_its_version`, `reveal_and_mix_change_the_page_drawn`.

### Requirement 22.5: Sub-pages
- **Statement:** Every sub-page the engine's store keeps must be listed for a page on screen, at most 64 (the engine viewer's bound); the latest must be drawn as it arrives unless the user chooses one, by Left and Right while the page is open or by the control on the page, which shows how many there are and which is shown, and returns to the latest when pressed; an address naming a sub-page (`#302:0002`) must open its page there.
- **Reason:** Rotating pages hold most of a service's text; only the latest was ever shown.
- **Source:** Decision 67.
- **Verification:** Tests of the decoder, the trusted core's stepping, the address and the interface's control.
- **State:** Met: `sub_pages_are_listed_and_one_can_be_chosen`, `sub_pages_step_round`, `a_fragment_names_a_page_of_the_channel_and_is_never_fetched` (the sub-page kept), `the_sub_page_control_steps_and_returns_to_the_latest`. The engine's bound, `viewer::MAX_LISTED_SUBPAGES`, is used by name in the decoder and asserted equal to the interface core's (requirement 2.4).

### Requirement 22.6: The controls are the trusted core's to keep
- **Statement:** The interface must learn of the open page's controls only which keys lead somewhere, which of reveal, hold and mix are on, and how many sub-pages there are and which is shown; every control must reach the trusted core as strip input, which alone tells the decoder.
- **Reason:** As every other strip input (requirement 11.1); the interface decides nothing.
- **Source:** Decision 67.
- **Verification:** Protocol tests of both messages' bounds; review.
- **State:** Met: `page_controls_keep_their_bounds`, `every_kind_round_trips` (both messages).

---

## 23. The remote control

From `remote-control.md` (decision 68, taken while the owner was away).

### Requirement 23.1: A remote with every page control
- **Statement:** The window must offer a remote control holding the page number being entered or shown, the number pad, page up and down, Fastext's four coloured keys and Index, the sub-page steps and count, and Reveal, Hold and Mix, each lit only while it applies (requirements 22.1 to 22.5), each with its tip (requirement 12.12).
- **Reason:** The page controls are keys otherwise hidden from a mouse user, as a set's buttons are on its remote.
- **Source:** Decision 68.
- **Verification:** Interface tests; seen on macOS.
- **State:** Met, 23 October 2026: `the_remotes_keys_are_the_existing_inputs`; seen docked on macOS.

### Requirement 23.2: Docked, floating or away
- **Statement:** The remote must dock along the window's right edge, the content narrowing and the remote sliding in and out over 200 milliseconds, or float over the page area, moved by its title row; Command-Shift-R on macOS and Control-Shift-R elsewhere, the menu and a handle at the right edge must show and hide it.
- **Reason:** The owner's request: "a sidebar that can pop in and out of the right hand side or undocked to float".
- **Source:** Decision 68.
- **Verification:** Interface tests.
- **State:** Met: `the_remote_docks_slides_and_goes_away`, `a_floating_remote_moves_and_stays_below_the_strip`.

### Requirement 23.3: The remote never covers the trusted strip
- **Statement:** A floating remote must be kept inside the window and below the trusted strip and the bookmarks bar, however it is dragged and however the window is sized.
- **Reason:** The strip must always be visible (requirement 11.4).
- **Source:** Decision 68.
- **Verification:** Test.
- **State:** Met: `a_floating_remote_moves_and_stays_below_the_strip`.

### Requirement 23.4: Its place is kept by the trusted core
- **Statement:** Whether the remote is hidden, docked or floating, and a floating remote's corner, must be kept in `settings.txt` as a `remote` line, read strictly (each number at most 16 384), and restored at the next start; the interface stores nothing (requirement 7.9).
- **Reason:** A remote put somewhere should be there next time.
- **Source:** Decision 68.
- **Verification:** Tests of the settings file and the protocol.
- **State:** Met: `the_remotes_place_is_read_strictly`, `settings_round_trip_through_their_file`, `the_view_and_every_change_round_trip`, `every_kind_round_trips`.
