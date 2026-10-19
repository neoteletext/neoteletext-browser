# T42 Browser: requirements

| | |
|---|---|
| **Status** | Adopted 16 October 2026 with the native architecture (decision 33). Every limit these requirements refer to is a constant defined in neoteletext (decision 26; requirement 2.4). |
| **Companion documents** | [`architecture.md`](architecture.md) explains the design these requirements describe. [`security-analysis.md`](security-analysis.md) analyses its security. The earlier design's requirements are in [`../prototypes/docs/requirements.md`](../prototypes/docs/requirements.md). |

## How to read this document

- **No abbreviations.** Every technical term is written out in full. Initials appear only where they are a literal identifier in code, configuration, a file name or a standard's title. `architecture.md` section 17 is the glossary.
- **Numbering.** Each requirement is numbered by section: "Requirement 6.3" is the third requirement of section 6. **Numbers are carried over from the earlier design's requirements and never reused.** A requirement that existed only to contain a web engine keeps its number and is marked **withdrawn**, with the reason. New requirements take the next free number in their section.
- **"Must"** means the requirement is mandatory. **"Should"** means it is expected unless a recorded reason says otherwise.
- **Each requirement states:**
  - **Statement:** what must be true.
  - **Reason:** why it matters.
  - **Source:** the owner's decision it comes from (numbered in `architecture.md` section 2), or "Architecture" when it follows from the design, or "Prototype finding" when it was learned on this codebase.
  - **Verification:** how it is shown to hold. "Test" means an automated test. "Check" means an automated check in the project's check script. "Review" means a recorded human review.
  - **State:** whether it is met today, **met**, **partly met** or **not met**, as of 16 October 2026, with what is missing.
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

---

## 1. Purpose and scope

### Requirement 1.1: What the application is
- **Statement:** T42 Browser must be an application for browsing teletext delivered over the web and from local files. Each tab must show one teletext channel, and every page shown must be a teletext page.
- **Reason:** This defines the product and bounds what it may fetch and show.
- **Source:** Decisions 1, 12 and 33.
- **Verification:** Review.
- **State:** Met.

### Requirement 1.2: Security comes first
- **Statement:** Where a requirement trades capability, convenience or performance against security, the choice that is more secure must be made, and the trade-off must be recorded in `security-analysis.md`.
- **Reason:** The owner has stated that being hacked is not acceptable under any circumstances.
- **Source:** Decision 4.
- **Verification:** Review of every change that touches a requirement in sections 3 to 8, 10, 11 or 13.
- **State:** Met as a practice.

### Requirement 1.3: No executable content
- **Statement:** The application must never execute code supplied by a page, a channel, a server or a file. It must contain no interpreter reachable from outside data, except, if open decision 2 adopts it, the decoder's WebAssembly interpreter running only the application's own decoder module.
- **Reason:** Downloaded code is the most direct route to compromise.
- **Source:** Decision 4; `neoteletext` decision E.32.
- **Verification:** Review; check that no crate in the application's dependency tree is a script engine, a web engine or a general interpreter.
- **State:** Met; the check is not written.

---

## 2. The teletext engine

### Requirement 2.1: The engine is `neoteletext`
- **Statement:** All teletext recognition, decoding and rendering must be done by the `neoteletext` crates. All web protocol reading must be done by `neoteletext-web`. T42 Browser must not contain a second implementation of any of these.
- **Reason:** One implementation, already tested and fuzzed, is safer than two.
- **Source:** Decisions 1 and 36.
- **Verification:** Review.
- **State:** Met: the decoder uses the engine's readers, decoder and renderer, and the fetcher the engine's user agent.

### Requirement 2.2: The engine does not depend on a user interface toolkit
- **Statement:** No `neoteletext` crate that T42 Browser uses may depend on a user interface or graphics toolkit.
- **Reason:** The trusted processes must not carry toolkit code they do not need.
- **Source:** Decision 1.
- **Verification:** Check: a script runs `cargo tree` on each `neoteletext` crate the application uses and fails if any toolkit appears.
- **State:** Met; the check is not written.

### Requirement 2.3: The engine's own guarantees are kept
- **Statement:** The `neoteletext` crates must be built with their own checks, fuzzing and feature rules, and T42 Browser must not enable any feature of them that the engine's rules forbid in a browser build.
- **Reason:** The engine's safety rests on those rules.
- **Source:** Architecture.
- **Verification:** Check: the T42 Browser check script runs the `neoteletext` check script for the commit it builds against.
- **State:** Not met: the check is not written.

### Requirement 2.4: Limits and fetching behaviour come from neoteletext
- **Statement:** Every limit on fetching, refreshing and input must be a constant defined once in neoteletext and used by T42 Browser by name. The fetching policy of requirements 4.7 and 5.15 to 5.17 must be implemented in neoteletext. Where neoteletext does not yet define a limit or behaviour, it must be added there (`architecture.md` section 5.1), not in T42 Browser.
- **Reason:** One definition of every value and one implementation of channel fetching.
- **Source:** Decision 26.
- **Verification:** Check: T42 Browser's source defines no constant for any of these limits; review.
- **State:** Partly met. The fetcher restates three of the engine's values instead of naming them: the cookie store's size (100 cookies, 32 kilobytes) and the connection time-out (10 seconds). The trusted core's pace for file channels, 0.2 seconds, is the engine's fetch interval written again.

### Requirement 2.5: The engine is pinned and never patched
- **Statement:** T42 Browser must depend on the engine by one pinned commit, read from the engine's repository, never from a copy of its source or its working tree, and must never patch it locally.
- **Reason:** The engine is maintained in its own repository, by its own process (decision 36).
- **Source:** Decision 36.
- **Verification:** Check: every `neoteletext` dependency in `native/Cargo.toml` names the same `rev`.
- **State:** Met; the check is not written. The pinned commit is in the engine's local repository and is not yet published.

---

## 3. Processes and isolation

### Requirement 3.1: Separate processes
- **Statement:** The application must run as separate processes: a trusted core, one fetcher per web channel, one decoder per channel, an interface process, and, when built, a font helper, an updater and a launcher.
- **Reason:** Separation lets each part hold only what it needs, so that a compromise of one part gains little.
- **Source:** Decisions 18 and 33.
- **Verification:** Test: the running application's process list matches this set.
- **State:** Met for the four processes built; the test is not written.

### Requirement 3.2: The trusted core contains no web engine and no format reader
- **Statement:** The trusted core must not link or load a web engine, a script engine or any reader of network or teletext formats. It must receive only typed, bounded results from the other processes.
- **Reason:** The trusted core holds everything the user trusts; it must be the smallest and least exposed part.
- **Source:** Decision 4.
- **Verification:** Check: the trusted core's dependency tree contains no web engine, script engine, `neoteletext-formats` or `neoteletext-web`.
- **State:** Met: the trusted core depends on `neoteletext-core` for flash timing and `neoteletext-image` for the image writer only. The check is not written.

### Requirement 3.3: The fetcher has only the network
- **Statement:** The fetcher must be able to reach the network and nothing else: no files, and no other processes except through its pipe to the trusted core.
- **Reason:** The fetcher reads untrusted protocol data and must be worth little to an attacker who compromises it.
- **Source:** Architecture; `neoteletext` requirement REQ-SEC-018.
- **Verification:** Test on each platform: the fetcher's attempts to read or write a file, or start a process, fail.
- **State:** Partly met. On macOS the fetcher cannot write files, but its profile allows reading them; elsewhere it is not sandboxed.

### Requirement 3.4: The decoder has no rights
- **Statement:** Each decoder must have no network access, no file access and no secrets.
- **Reason:** The decoder reads the most untrusted data; a fault in it must stay inside its own process.
- **Source:** Architecture.
- **Verification:** Test on each platform: the decoder's attempts to open a file or a network connection fail.
- **State:** Partly met. On macOS the decoder has no network and cannot write files, but its profile allows reading them; elsewhere it is not sandboxed. Whether it also runs inside `wasmi` is open decision 2.

### Requirement 3.5: The font helper has no rights
- **Statement:** Fonts fetched from the network must be read by a confined font helper with no network access, no file access and no secrets. Its output must be checked by the trusted core (hash and table shape) and checked again by the decoder before use.
- **Reason:** A font file is untrusted input in its own format.
- **Source:** Decision 14; `neoteletext/docs/font-security.md`.
- **Verification:** Test: a malformed font is rejected; a font table whose shape is wrong is rejected by both checks.
- **State:** Not met: fonts are not fetched yet.

### Requirement 3.6: The interface process holds no secrets and decides nothing
- **Statement:** The interface process must never receive cookies, credentials, history or bookmarks beyond what is on screen, or any setting value that affects security. It must have **no network on every platform**, and no file access beyond its own resources and fonts. It must make no decision: it shows the state the trusted core sends and passes on what the user did.
- **Reason:** An attacker who took over the interface must find nothing of value.
- **Source:** Decisions 4, 18 and 33.
- **Verification:** Test on each platform: network and file access fail from the interface process; review of every message type sent to it.
- **State:** Partly met. No secret is ever sent to it (review of `protocol`). On macOS it has no network and cannot write files, but its profile allows reading them; elsewhere it is not sandboxed.

### Requirement 3.7: Operating-system sandboxes
- **Statement:** On each platform the processes must be sandboxed as in `architecture.md` section 6.5, starting from "deny everything" and allowing only what each process needs.
- **Reason:** Each sandbox is a second wall behind the first.
- **Source:** Architecture.
- **Verification:** Test on each platform, as in requirements 3.3 to 3.6.
- **State:** Partly met. macOS has `sandbox-exec` profiles for the decoder, fetcher and interface that start from "allow everything" and deny the network (decoder and interface) and file writes (all three). The trusted core is not sandboxed. Windows, Linux, Android and iOS have nothing yet.

### Requirement 3.8: The macOS sandboxing gap is closed
- **Statement:** The fetcher and decoder must be sandboxed on macOS before the first release.
- **Reason:** Without it the macOS build lacks the second wall.
- **Source:** Architecture.
- **Verification:** Test, as in requirements 3.3 and 3.4, on macOS.
- **State:** Partly met, as requirement 3.7 says. `sandbox-exec` is deprecated by Apple; the release build needs the App Sandbox with each process as a helper of its own.

### Requirement 3.9: Security does not depend on the interface's code
- **Statement:** Every security property in this document must hold even if the interface process runs code written by an attacker, **except** the integrity of the trusted strip and of strip input, which the interface process draws and sends (`architecture.md` section 6.1, open decision 3).
- **Reason:** The owner's principle that the interface's code must make no difference to security (decision 20). In this architecture the strip is drawn by the interface process, so its integrity depends on that process; every other property must not.
- **Source:** Decisions 20 and 33.
- **Verification:** Test: a hostile interface harness replaces the interface process with an adversarial client that sends every intent kind with arbitrary values, malformed messages, floods and replayed messages; the test shows that no secret is disclosed, no file or network is reached, no channel's data reaches another channel's requests, and no intent other than strip input changes channel.
- **State:** Not met: the harness is not written. The protocol already refuses every kind the interface may not send, and its tests cover malformed and oversized messages.

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

---

## 4. Messages between processes

### Requirement 4.1: One framed protocol
- **Statement:** All processes must communicate with the engine's framing (one kind byte, a 32-bit big-endian length and a payload), defined once in the `protocol` crate.
- **Reason:** One small, strict protocol for every boundary.
- **Source:** Architecture.
- **Verification:** Review.
- **State:** Met.

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

### Requirement 4.4: Every message is checked by the receiver
- **Statement:** The trusted core must check every message from every other process against the state it holds: known channel, known tab, sizes within limits, palette within 32 entries, every palette index within the palette, page numbers within 100 to 8FF.
- **Reason:** A compromised process can send well-formed but false messages.
- **Source:** Architecture.
- **Verification:** Test: messages naming an unknown tab, a wrong channel or out-of-range values are refused.
- **State:** Met for the messages that exist: frames are checked on arrival, a visibility report naming another channel's pages is filtered out, a tab index out of range is ignored.

### Requirement 4.5: Intents cannot change channel
- **Statement:** No page intent may cause a tab to navigate to another channel, change a security setting or read a cookie. Navigating must be possible only through strip input, message kinds of their own (requirement 11.1).
- **Reason:** Page intents carry what the user did in the page area, where a page image can draw anything.
- **Source:** Decisions 4 and 18.
- **Verification:** Test: every intent kind is sent with values that would leave the channel or change a setting, and none succeeds.
- **State:** Met by construction: no intent kind carries an address. The test is not written.

### Requirement 4.6: The readers are fuzzed
- **Statement:** The trusted core's readers of messages from the fetchers, the decoders and the interface, and the interface's reader of messages and images from the trusted core, must be fuzzed continuously.
- **Reason:** These readers are each process's only exposure to the others.
- **Source:** Architecture.
- **Verification:** Check: fuzzing targets exist for each reader and run in the full check.
- **State:** Not met.

### Requirement 4.7: No interface request or key affects fetching
- **Statement:** No intent from the interface and no key press, however received, may cause, delay, reorder or prioritise a network request. Selecting a page may only show a page the channel's decoder already holds. **Exception (decision 23):** on a channel that publishes only separate page files, selecting a page not yet held fetches that page's file, within the tab's own channel and the limits of requirement 5.12.
- **Reason:** A hostile interface sees pages of every open channel; if it could influence fetching it could encode one channel's pages in the pattern of requests to another.
- **Source:** Decisions 20 and 22.
- **Verification:** Test with the hostile interface harness: across every intent kind and key, with arbitrary values and timing, the requests reaching a test server are identical to a run with no interface activity.
- **State:** Met by construction: the fetcher takes no input from the trusted core at all, and the interface's visibility reports reach only the decoder's rendering. The test is not written. Channels with only separate page files are not supported yet.

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

### Requirement 5.7: Fonts are fetched only when named
- **Statement:** A font must be fetched only when a page or channel names it, and used only after the checks of requirement 3.5. Teletext fonts from the network must never reach the interface process.
- **Reason:** Fonts are untrusted input.
- **Source:** Decision 14.
- **Verification:** Test.
- **State:** Not met: fonts are not fetched yet.

### Requirement 5.8: Transports
- **Statement:** The fetcher must support the HyperText Transfer Protocol version 1.1 with and without Transport Layer Security, WebSocket, Server-Sent Events and the zxnet live feed. It must not support the push mechanism of the HyperText Transfer Protocol version 2.
- **Reason:** These are the standard ways teletext services publish pages and push updates.
- **Source:** Decision 12.
- **Verification:** Test against a test server for each transport.
- **State:** Partly met: plain HyperText Transfer Protocol and Server-Sent Events, exercised against the development site. Transport Layer Security needs a library (open decision 1); WebSocket and zxnet are not wired. No automated test yet.

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
- **State:** Partly met: refused by the engine's user agent unless allowed, but allowed only with `--allow-local` on the command line; there is no setting in the strip yet.

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
- **State:** Not met: the site-change screen is not built.

### Requirement 5.14: No referrer and a minimal user agent
- **Statement:** The fetcher must send no `Referer` header and only the engine's minimal user agent profile.
- **Reason:** Requests must reveal as little as possible about the user.
- **Source:** Architecture.
- **Verification:** Test: inspect requests received by a test server.
- **State:** Met by the engine's user agent; the test is not written here.

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
- **State:** Met: every channel has its own decoder and fetcher, and the fetcher's cookies are its channel's.

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
- **State:** Not met: every tab opens its own channel.

### Requirement 7.5: The trusted core owns the cookie store
- **Statement:** The cookie store must live in the trusted core. For each request the trusted core must give the fetcher only the cookies that request carries, and the fetcher must return any `Set-Cookie` headers to the trusted core.
- **Reason:** A compromised fetcher must see only the cookies of requests in flight.
- **Source:** Decision 4.
- **Verification:** Test: the fetcher process holds no cookie store.
- **State:** Not met: each fetcher holds its own channel's store, inside the engine's user agent, in memory. Needs the engine change of `architecture.md` section 5.1.

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
- **State:** Partly met: it writes no file (denied on macOS) and holds no cookies. It keeps the images of pages it has been sent until they are replaced.

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
- **State:** Not met: Transport Layer Security is not built.

### Requirement 7.12: Local files
- **Statement:** Each local file must be its own channel with no cookies, read by the trusted core and decoded by its own decoder, and links from it to web addresses must go through the site-change screen.
- **Reason:** A file must not borrow the identity or cookies of a channel.
- **Source:** Architecture.
- **Verification:** Test.
- **State:** Partly met: a file is its own channel with its own decoder, at most 64 megabytes or 4,096 page files. Links are not followed yet.

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
- **State:** Partly met: only user-started navigations exist.

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
- **State:** Not met: page-started navigations do not exist yet.

### Requirement 8.5: The site-change screen cannot be faked by a page
- **Statement:** The site-change screen must be drawn only in the trusted strip, never in the page area, and a confirmation must not be accepted from a click inside the page area or from pasted input.
- **Reason:** A page image can draw anything.
- **Source:** Decisions 4 and 24.
- **Verification:** Test; review.
- **State:** Not met: not built.

### Requirement 8.6: Cancelling returns to the committed page
- **Statement:** Cancelling the site-change screen must return the tab to its committed page unchanged.
- **Reason:** Declining must have no effect.
- **Source:** Architecture.
- **Verification:** Test.
- **State:** Not met: not built.

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

### Requirement 9.7: Flash follows the engine's timing
- **Statement:** Flash must be rendered by the decoder at the engine's flash timing (one second in twelve units, its decision E.48), for shown pages only, and step 3 of `architecture.md` section 10 must pass on only the frames that changed.
- **Reason:** The engine defines flash; rendering only shown pages keeps it cheap.
- **Source:** Architecture. (The earlier requirement had the trusted core alternate two stored phases; rendering in the decoder was measured at about 7 seconds of processor time a minute for the shown channel and is to be made cheaper by rendering only when a flashing page changes.)
- **Verification:** Test.
- **State:** Met.

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
- **State:** Partly met: about 6 kilobytes a page on the stand-in engine (`../prototypes/docs/proofs.md` section 4); not yet measured on real channels.

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
- **Statement:** The interface process, like every crate of the application, must forbid `unsafe` code in its own source. The one exception is the `dock-menu` crate, whose macOS module reaches AppKit through the Objective-C runtime; it denies `unsafe_code` everywhere else, and each unsafe block there states why it is sound.
- **Reason:** A memory-safety fault is the most likely way to take over a process. AppKit offers the Dock menu only through calls Rust cannot check (decision 39).
- **Source:** Architecture; decision 39.
- **Verification:** Check: the workspace lints forbid `unsafe_code`; `dock-menu` denies it and allows it only in `src/mac.rs`, with Clippy's `undocumented_unsafe_blocks`.
- **State:** Met.

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
- **Reason:** A strip in a window of its own can be covered by any other window (`../prototypes/docs/proofs.md` section 3); in the page area a page image could imitate it.
- **Source:** Decisions 4, 18 and 33.
- **Verification:** Test: no page intent produces a navigation; review of every layout.
- **State:** Met.

### Requirement 11.2: What the trusted strip holds
- **Statement:** The trusted strip must hold the address field, the channel's identity, whether the channel is encrypted, Back, Forward, Reload and Home, the site-change screen, and every setting that affects security.
- **Reason:** Everything that decides or shows trust is in one place.
- **Source:** Decision 4.
- **Verification:** Review.
- **State:** Partly met: the address, identity, encryption state, Back, Forward and the bookmark star. Reload, Home, the site-change screen and the security settings are not built.

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

---

## 12. Window features

### Requirement 12.1: Tabs
- **Statement:** The interface must draw a tab strip with close buttons and a new-tab button; the list and order of tabs, their creation and closing, and session restore must be decided by the trusted core.
- **Reason:** The tab strip is drawn by the interface; tab state is trusted.
- **Source:** Decisions 33 and 35.
- **Verification:** Test.
- **State:** Partly met: tabs, close and new tab. Drag to reorder, overflow and session restore are not built.

### Requirement 12.2: Page walls and transitions
- **Statement:** The interface must show the window of decision 35 (banks, page list, page area, overview), with its transitions, and must respect the platform's reduced-motion preference.
- **Reason:** Navigating many pages at once is the purpose of the interface.
- **Source:** Decisions 7 and 35.
- **Verification:** Review; test with reduced motion.
- **State:** Partly met: the window and its transitions are built. Reduced motion is not read from the platform.

### Requirement 12.3: Clicks on pages are cell clicks
- **Statement:** A click on a page must be converted to a cell (column and row) from the pointer position and the rectangle the page was drawn in, and sent as an intent. The trusted core must decide, with the decoder, whether the cell is a link.
- **Reason:** A page image can draw fake buttons; the click must mean only "this cell".
- **Source:** Decision 4.
- **Verification:** Test under every layout.
- **State:** Partly met: cell clicks are computed and sent, with a test. The trusted core does not yet follow links.

### Requirement 12.4: Keys work in the tab
- **Statement:** With the page area focused, page-number keys, Fastext colour keys, reveal, hold and mix must work. A key must only select among pages already held (requirement 4.7). A key that would leave the channel must go through the site-change screen.
- **Reason:** Teletext is driven by these keys.
- **Source:** Decision 22.
- **Verification:** Test each key.
- **State:** Partly met: three digits choose a page of the channel, and Up and Down step through pages. Fastext, reveal, hold and mix are not built.

### Requirement 12.5: Menus and shortcuts
- **Statement:** The application must provide a native menu bar (File, Edit, View, History, Window) carrying the browser's shortcuts. Menu items and buttons must send the same intents.
- **Reason:** Shortcuts must work whatever has focus.
- **Source:** Prototype finding.
- **Verification:** Test each shortcut.
- **State:** Not met: the menu button exists with no menu.

### Requirement 12.6: Internal pages
- **Statement:** Bookmarks, history and settings must be shown from data the trusted core supplies, except settings that affect security, which must be changed only in the trusted strip.
- **Reason:** Security settings must stay in the strip.
- **Source:** Decision 4.
- **Verification:** Test.
- **State:** Not met.

### Requirement 12.7: Developer tools
- **Statement:** A docked developer tools panel must show, for the current channel: the packet log, the page store, network requests, cookies, frame statistics and engine warnings, all from the trusted core and shown as text.
- **Reason:** Diagnosis without exposing the trusted core.
- **Source:** Architecture.
- **Verification:** Test that hostile packet contents appear as text.
- **State:** Not met.

### Requirement 12.8: Windows
- **Statement:** Each window must have one trusted strip. Moving a tab between windows must be a change of state in the trusted core. Frames for a page must go only to the window showing it.
- **Reason:** Windows share one trusted state.
- **Source:** Architecture.
- **Verification:** Test.
- **State:** Not met: one window only.

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
- **State:** Met, with tests of the decoder's change detection; the window's marks were checked by eye.

### Requirement 12.11: The clipboard and input methods
- **Statement:** The address field must support copy, paste and text composed by an input method, shown while it is composed.
- **Reason:** Addresses are pasted more often than typed, and many scripts need an input method.
- **Source:** Architecture.
- **Verification:** Test on each desktop.
- **State:** Partly met: composed text is inserted when committed. The clipboard (needs a library) and showing text while it is composed are not built.

---

## 13. The application update mechanism

Requirements 13.1 to 13.13 are unchanged from the earlier design, and are stated there in full (`../prototypes/docs/requirements.md` section 13). In brief:

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

- **State of all of section 13:** Not met: the updater and launcher are not built.

---

## 14. Build, dependencies and verification

### Requirement 14.1: One check script and hooks
- **Statement:** One local check script must run every check in this document. Git hooks must run the fast checks before each commit and the full checks before each push, and must not be bypassed.
- **Reason:** Requirements that are not checked drift.
- **Source:** Owner's baseline instructions.
- **Verification:** Review of the hooks.
- **State:** Partly met. `scripts/check.sh commit`, run before each commit, formats, lints and tests only the parts the staged files touch, and refuses to run while those parts have changes that are not staged, so what it checks is what is committed. `scripts/check.sh full`, run before each push on a clean tree at the commit pushed, adds release tests, `cargo-deny`, `cargo-machete`, fuzzing, the prototypes' WebAssembly and frontend builds, and the Linux build and tests on the Linux host. Neither skips a step: a missing tool or an unreachable host fails the run. The compiler and every tool are pinned (`rust-toolchain.toml`, `scripts/tools.sh`). The full gate has not yet passed as a whole. Many checks named in this document are not written yet.

### Requirement 14.2: Strict compilation and linting
- **Statement:** Rust code must build with `unsafe` code forbidden, the Clippy lint group `pedantic` enabled and warnings treated as errors; any suppression must be narrow, at its site, with a written reason.
- **Reason:** Warnings are faults found early.
- **Source:** Owner's baseline instructions.
- **Verification:** Check.
- **State:** Met. Beyond the statement: every public item is documented, and the crates that read input from outside their process (protocol, decoder, fetcher, core, interface) also deny every construct that can panic, with one list the check script keeps identical in each.

### Requirement 14.3: Dependencies
- **Statement:** Lockfiles must be committed and their changes reviewed. `cargo-deny` and `cargo audit` must pass. Every library must be under a permissive licence (decision 34) and needs the owner's approval.
- **Reason:** The supply chain is a route to compromise.
- **Source:** Decisions 4 and 34; owner's baseline instructions.
- **Verification:** Check.
- **State:** Partly met: every dependency is pinned to an exact version. `cargo-deny` runs in the full gate (`native/deny.toml`): licences, sources, and the RustSec advisories that `cargo audit` also reads; `cargo audit` itself is not run. One advisory is ignored pending the owner's decision: RUSTSEC-2026-0192, `ttf-parser` unmaintained, which only winit's Wayland window decorations use. The libraries in use await the owner's approval by name (open decision 6).

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
- **State:** Partly met: macOS only.

### Requirement 15.2: Phones (after the first version)
- **Statement:** When phones are added, Android must run the decoder in an isolated service process, and on iOS, where there is one process, the decoder must have a wall of its own (open decision 2). Every requirement not tied to process separation must still be met.
- **Reason:** Phones limit process separation.
- **Source:** Architecture.
- **Verification:** The full check on each phone platform.
- **State:** Not met.

### Requirement 15.3: Microcontrollers
- **Statement:** The interface core must build without the standard library and draw through the microcontroller back end. A microcontroller build must keep pages, page-number and Fastext keys, flash and a simple page wall, with the cuts the owner decides (open decision 5), and must have its own security analysis.
- **Reason:** Microcontrollers are a target (decision 34).
- **Source:** Decision 34.
- **Verification:** Check: the interface core and the microcontroller back end build for a microcontroller target; a test draws the window into a display in memory.
- **State:** Partly met: the interface core is `no_std` and a test draws the window through `embedded-graphics` into a 1024 by 600 display in memory. No microcontroller target is built.

### Requirement 15.4: No graphics processor needed
- **Statement:** Every target must work with drawing on the processor only; a graphics processor may make it faster but must never be required.
- **Reason:** Decision 34.
- **Source:** Decision 34.
- **Verification:** Test on a machine or virtual machine with no graphics processor.
- **State:** Met on macOS by design (`vello_cpu` and softbuffer); not tested without a graphics processor.

---

## 16. Sound

### Requirement 16.1: Sound follows the engine's audio profile
- **Statement:** Sounds a channel carries, on EN 300 708 independent data lines of data channel 12, must be played as the engine's audio profile specifies (its area AUD, decisions E.84 to E.86): the latest command of each channel wins, and a page's sounds play while that page is open. The browser adds no rules of its own to what plays.
- **Reason:** One set of rules for every receiver, kept in the engine (decision 36), so that a channel sounds the same in the browser as on a microcontroller.
- **Source:** Decision 37; the engine's decisions E.84 to E.86.
- **Verification:** Decoder tests: a page's sound is sent when its page is opened and faded when it is left; a live sound streams and fades two seconds after its last block.
- **State:** Met on macOS.

### Requirement 16.2: Sound is read only in the decoder
- **Statement:** Audio packets must be read, reassembled, stored and decoded only in the decoder process. The trusted core must not parse them, and the interface process must receive only 16-bit samples and voice operations the trusted core checked.
- **Reason:** Untrusted bytes are read only in a process with no rights (architecture principle 2), and nothing in the interface parses data from outside (principle 3).
- **Source:** Decisions 18 and 37.
- **Verification:** Inspection: the audio receiver is a dependency of the decoder alone; the interface's sound module takes only `protocol::audio::Audio`.
- **State:** Met.

### Requirement 16.3: Every tab is heard
- **Statement:** The trusted core must pass on every channel's sound, tagged with its channel, and the interface must play each channel through its own mixer, so a background tab keeps playing, as in a web browser. A background tab keeps the page it had open. When a channel's decoder is gone, the trusted core must tell the interface to forget that channel's sounds.
- **Reason:** The owner asked for tabs that play sound like Chrome's (decision 38); a decoder cannot touch another channel's sound.
- **Source:** Decisions 37 and 38.
- **Verification:** Tests: `hearing::tests::every_channel_is_heard_and_its_operations_are_tagged_with_it`, `the_opened_page_goes_to_the_active_channels_decoder_and_others_keep_theirs`; the interface's `every_channel_is_heard_and_a_muted_one_is_silent_but_still_sounding`; by hand on 19 October 2026, a live sound kept playing after its tab went to the background.
- **State:** Met on macOS.

### Requirement 16.4: Every voice operation is checked twice
- **Statement:** Every voice operation must be bounded by the protocol (a channel of 0 to 3, a gain of 0 to 100, a fade of 5 to 10 000 milliseconds, a profile sample rate, at most two minutes a sound and 32 768 samples a message), and checked again by the trusted core: a sound must be defined before it is filled or started, samples must lie within it, a live sound takes only its next samples, and a channel may hold at most 32 MB of samples in the interface, every channel together 64 MB, a live sound counting as two seconds at 48 kHz. Only the trusted core may tell the interface to forget a channel's sounds, or mute it.
- **Reason:** A compromised decoder could otherwise make the interface hold memory without bound, or reset another channel's sound.
- **Source:** Requirements 4.2 and 4.4; decision 37.
- **Verification:** Tests: the protocol's audio tests of every bound and of a decoder's reset or mute; `hearing::tests::operations_are_checked_against_what_was_defined`, `a_live_sound_takes_stream_samples_only` and `the_ledger_bounds_what_the_interface_holds`.
- **State:** Met.

### Requirement 16.5: The opened page
- **Statement:** A page's sounds must play while it is open: the page zoomed on the wall, or else the page chosen by number or in the list. Page sounds are on by default. A setting to turn them off, and a control for the master volume, must be added to the window; muting is requirements 16.7 to 16.9.
- **Reason:** Teletext attaches sounds to pages, and a television goes to the page whose number is typed.
- **Source:** Decision 37; the engine's REQ-AUD-012.
- **Verification:** Test: `the_opened_page_is_the_zoomed_one_or_else_the_one_chosen`; an end-to-end check by hand on 19 October 2026 (typing 100 played the page's chime).
- **State:** Partly met: the setting and the volume are not built.

### Requirement 16.6: Sound output
- **Statement:** The interface process must mix with the engine's mixer (`neoteletext-audio-mix`) and play through the operating system's output device with `cpal`, rendering on the audio thread without allocating. With no output device, the browser must run silent.
- **Reason:** One mixer that behaves as the microcontrollers' does; `cpal` keeps unsafe code out of the browser's crates.
- **Source:** Decisions 37 and 40.
- **Verification:** Interface tests of applying operations and rendering; `a_tone_plays_through_the_default_output_device`, run by hand because it needs a device (`cargo test -p t42-interface -- --ignored`); setting `T42_AUDIO_TRACE` makes the interface report the operations it applies and the loudest sample played.
- **State:** Met on macOS, Windows (WASAPI) and Linux (ALSA): on 19 October 2026 a page's sound played through each, as the device's peak level showed (decision 40).

### Requirement 16.7: A tab shows its sound and mutes
- **Statement:** A tab whose channel plays sound must show a speaker before its close button, and a muted tab a crossed-out speaker, whether or not it plays. A click on it must mute the tab, or unmute it, without selecting it. A tab's mute must follow it to the next channel it shows. The window's title must be the selected tab's, followed by "Audio playing" or "Muted" while it plays sound.
- **Reason:** The owner asked for Chrome's behaviour (decision 38).
- **Source:** Decision 38.
- **Verification:** Tests: `a_tab_playing_sound_shows_a_speaker_that_mutes_it`, `muting_tells_the_interface_once_per_change`; by hand on 19 October 2026: the speaker appeared on a background tab playing a live sound, a click muted it (the device's peak fell to nothing), and the title read "livechannel.t42 – Muted – T42 Browser".
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
- **Statement:** A live sound (the engine's REQ-AUD-014) must play as its blocks arrive, from the newest block when its play starts, and fade out two seconds after its last block. The trusted core must tick every decoder, which keeps the clock of its live sounds.
- **Reason:** The owner asked for live sounds (decision 38).
- **Source:** Decision 38; the engine's decision E.87.
- **Verification:** Tests: the decoder's `a_live_sound_streams_and_fades_two_seconds_after_its_last_block`; the interface's `a_live_sound_plays_once_its_buffer_fills`; by hand on 19 October 2026, a ten-minute live sound written by `neottx sound --live` played from a file channel.
- **State:** Met on macOS.
