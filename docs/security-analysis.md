# NeoTeletext: security analysis

| | |
|---|---|
| **Status** | Analysis of the native architecture (decision 33), 16 October 2026. Where a point was observed on this codebase rather than reasoned, it says so. Where the design and what is built differ, both are given. |
| **Companion documents** | [`architecture.md`](architecture.md) describes the design. [`requirements.md`](requirements.md) states each requirement this analysis relies on, numbered, with whether it is met; this document cites them as "Requirement N.N". The earlier design's analysis, with its web engine, is in `prototypes/docs/security-analysis.md`. |
| **The prototypes** | Removed on 21 October 2026 (decision 50); a path under `prototypes/` here is a path at the git tag `prototypes-final`. |

**Abbreviations.** None are used. Technical terms are written out in full; `architecture.md` section 17 is the glossary.

## Contents

1. Summary
2. What is protected
3. Who attacks, and how
4. Trust boundaries
5. Analysis of each boundary
6. What leaving the web engine changed
7. Spoofing and the line of death
8. The application update mechanism
9. Supply chain
10. Local attackers
11. Availability
12. Privacy
13. Platform notes
14. Comparison with the alternatives
15. Findings from this codebase
16. Remaining risks, ranked
17. Checks that keep these properties

---

## 1. Summary

The owner's requirement is that being hacked is not acceptable under any circumstances (decision 4). No design can promise that no component will ever have a fault. This design works to the achievable standard: **compromising any single component must gain an attacker as little as possible, and never the user's secrets.**

It meets that standard by:

- **Reading untrusted bytes only in processes with no rights:** the fetcher (one per web channel, network only), the decoder (one per channel, no network), the phone keyboard (the relay's messages, network only) and the updater (the update host's metadata and files, its one port and staging folder).
- **Confining every process but the broker and, in an installed browser, the launcher that starts it,** on macOS, Windows and Linux, from "deny everything": the helpers, the sound helper and the trusted core itself, which has no network, starts no program and reads only its settings; a small broker, the program the user starts, does those few things for it (decisions 52, 55 and 56).
- **Keeping a trusted core that contains no web engine,** reads no network or teletext format, draws nothing, and owns every secret and every decision.
- **Having no web engine at all.** The interface is NeoTeletext's own Rust code. It runs no script, parses no markup, and reads only images our own encoder made and text the trusted core cleaned. The whole class of web-engine risk that the earlier design spent decisions 27 to 32 containing is gone.
- **Keeping the address and every security decision in the trusted strip,** outside the page area, where no page image is ever drawn.

**What decision 33 accepted.** The trusted strip is drawn by the interface process. A fault in the interface process could therefore draw a false strip and send strip input (section 5.6). In the earlier design that needed a fault in a web engine plus an escape from its sandbox; here it needs a fault in our own interface code or its drawing libraries, reached only through our own images and cleaned text. Open decision 3 asks whether to move the strip into a smaller process.

**The main remaining risks,** detailed in section 16:
1. A fault in the trusted core, which is why it is kept small.
2. Spoofing of the trusted strip by page images, in an interface built for animation.
3. A fault in the interface process, which draws the strip.
4. The broker and the launcher, the processes not confined: the broker starts the others and answers the trusted core's few requests (section 5.8); the launcher chooses, accepts and rolls back versions (section 8). And the interface's file reads on Windows.
5. The update signing keys, and the launcher (section 8).

---

## 2. What is protected

| Asset | Where it lives | Why it matters |
|---|---|---|
| Which channel each tab shows, and the address the user sees | Trusted core; shown in the trusted strip | The user's decisions rest on it |
| Cookies, and anything a channel's login sets | The trusted core, one jar for ordinary tabs and one for private, each channel's under its own tag; persistent ones kept between runs in `cookies.bin`, sealed under a key in the platform's secret store, never a session cookie (decision 62, requirements 7.5 and 21.4) | Access to the user's accounts on teletext services |
| History, bookmarks, settings, sessions | Trusted core: history kept 90 days in `history.txt`, bookmarks in `bookmarks.txt`, settings in `settings.txt`, the session (the window and every tab but private ones, with their lists) in `session.txt`, each in the settings folder and readable by the user alone (decisions 57 and 59) | Private, and history most of all: which pages the user read and when; settings control security |
| The user's own files | The file system; opened by the trusted core when the user names one, and decoded by a decoder | Private |
| The integrity of the application's code | Installed versions, the launcher and its state file, and the update keys (section 8) | Everything else depends on it |
| Availability | Every process | The application must stay usable |

---

## 3. Who attacks, and how

| Adversary | Capabilities |
|---|---|
| **A hostile teletext service** | Sends any bytes in any role: malformed or enormous pages, crafted enhancement data, misleading page text and titles, a hostile contents file, a push feed that never stops, cookies |
| **A hostile recording or page file** | The same, through a file the user opens |
| **A network attacker** | Reads and alters unencrypted traffic, which is allowed (decision 10) and which is all traffic to a plain `http` channel; `https` channels use Transport Layer Security (decision 41) |
| **A compromised update server, a thief of a signing key** | Updates are built (section 8); until the key ceremony the production identity accepts none (decision 76) |
| **A compromised dependency** | Code inside a crate built into a process |
| **A local process running as the user** | Reads the user's files; sends input to windows under X11 |
| **A compromised component** | Any one of our own processes, assumed compromised to test that the others hold |

Out of scope: an attacker who already controls the user's account or has administrator rights, and physical access to the machine.

---

## 4. Trust boundaries

```
network ──[1]──► fetcher (one per web channel) ──[2]──► trusted core ──[3]──► decoder (one per channel)
files the user opens ──► broker ──[10]──► trusted core ──[3]──► decoder
trusted core ──[5]──► interface process (images, cleaned text, state)
interface process ──[6]──► trusted core (intents, and strip input)
trusted core ──[9]──► sound helper (checked voice operations) ──[9]──► trusted core (which channels sound)
trusted core ──[10]──► broker (start a helper, read a file channel, open the web)
update subdomain ──[8]──► updater ──► staging folder ──► launcher ──► versions directory
trusted core ──[11]──► updater (check, download) ──[11]──► trusted core (latest, staged, failed)
state file and staged versions ──[12]──► launcher ──► broker ──[12]──► launcher (healthy)
qrkeyboard relay ──[13]──► phone keyboard ──[13]──► trusted core (keys, text)
```

| Boundary | What crosses | The receiving side treats it as |
|---|---|---|
| 1. Network to fetcher | Raw protocol bytes | Untrusted |
| 2. Fetcher to trusted core | The contents file as typed entries; page bytes, forwarded unread; notices, exchanges for the console, push and input state, the descriptor's links, cookie questions, `Set-Cookie` values and `Clear-Site-Data`, and the kind of a refused certificate, one byte (decision 83, `certificate-errors-security.md`); to the fetcher, input for its push WebSocket and cookie answers | Untrusted, checked as data |
| 3. Trusted core and decoder | Bytes and whole files, shown pages, the open page and its view in; arrivals, updates, frames, links, sub-pages, voice operations, cursor and input-control records, channel log lines and, for an import, bookmarks out | Decoder output untrusted |
| 5. Trusted core to interface | Images our encoder made, cleaned text, tab, strip and channel state | The interface's input; contains nothing secret |
| 6. Interface to trusted core | Intents and strip input | Untrusted intents; strip input trusted only as far as the interface process is (section 5.6) |
| 8. Update subdomain to updater | Metadata and release files | Untrusted until `tough` verifies it against The Update Framework's metadata (`installation-and-updates-security.md` section 4a) |
| 9. Trusted core and sound helper | Checked voice operations and samples, and the sound settings, in; which channels sound, out | The sound helper's report untrusted |
| 10. Trusted core to broker | Requests to start one of the browser's helpers, end or wait for one, read a file channel, open a web address, give the local time, give the cookie file's key, report the version healthy, and read the launcher's state | Untrusted: the broker parses them within bounds and decides every role itself |
| 11. Trusted core and updater | A check or a download in; the newest version, a staged version or a failure out (`update::wire`) | The updater's messages untrusted: at most 512 bytes, read strictly, fuzzed (`updater_messages`) |
| 12. Launcher's inputs | The state file, a staged version's manifest and files, the broker's version mark and its `healthy` line | Untrusted: bounded strict readers, fuzzed (`launcher_state`, `release_manifest`, `version_mark`), then the pinned signer (`installation-and-updates-security.md` section 4a) |
| 13. Relay to phone keyboard to trusted core | The relay's messages; the phone's keys and text | Untrusted: the keyboard process's readers fuzzed (`relay_messages`, `phone_keys`); a compromised keyboard is a compromised local keyboard (`interaction-security.md` sections E and F) |

Boundary 4 of the earlier design, the font helper, is gone: no font is fetched from a channel (decision 66). Boundary 7, the strip in a separate host, is folded into boundary 6.

---

## 5. Analysis of each boundary

### 5.1 Boundary 1: the network into the fetcher

**Threats.** Faults in the protocol readers (the HyperText Transfer Protocol, Server-Sent Events, the descriptor and contents readers); decompression bombs; slow or endless responses; redirect loops; requests aimed at the local network.

**Mitigations.**
- The engine's strict, bounded readers, through its user agent; the fetcher only moves bytes between the agent and its sockets (requirement 2.1).
- The engine's limits on size, time and redirects, and on decompressed size (requirements 5.11 and 5.12).
- The local network refused unless the user names an address (requirement 5.10). Without a proxy, the fetcher's transport also refuses, before connecting, a name that resolves to this computer or the local network, by the engine's rule and the channel's allowed addresses (finding F-62; `a_name_resolving_to_this_computer_opens_no_connection`), and tells the console why.
- **Through a proxy the rule is by name** (finding F-17): the proxy resolves, so the browser sees only the name it asks for. Addresses written as local IP addresses, `localhost` and names under `.localhost`, `.local` and `.internal` are refused before the proxy is asked (requirement 19.11); the engine's decision E.113 adds names of one label and names under `.home.arpa`, `.lan`, `.corp`, `.intranet`, `.private` and `.localdomain`, which reach the browser with the re-pin of 24 October 2026 at `5311f98`. A public name that resolves to a local address, such as `10.0.0.1.nip.io`, stays a residual through a proxy; resolving here and asking the proxy for the address would tell the local resolver every name, against the privacy a proxy is set for.
- One fetcher per channel (requirement 3.12).
- A certificate is checked against Mozilla's roots, compiled in (`webpki-roots`), never the system's store, and revocation is not checked: a certificate revoked but not yet expired is accepted (finding F-95; `certificate-errors.md` section 3).
- A connection, its proxy tunnel and its TLS handshake are opened on a thread of their own and the engine's connect limit bounds the whole, one deadline for resolution, the connects and the handshake (finding F-63; `a_proxy_answering_a_byte_at_a_time_fails_within_the_deadline`), so a slow or silent server or proxy holds up nothing else the fetcher does (decision 53); a connection closed while it opens ends its thread (`closing_a_connection_while_it_opens_ends_its_thread`). What remains: the system's resolver cannot be interrupted, so it runs on a thread of its own, which outlives the deadline until the resolver gives up by its own limits, and a name whose servers never answer, retried at the agent's rate, can hold a few such threads at once; a close during resolution or the TCP connect itself ends the thread only when that step returns, within the deadline.

**If a fetcher is compromised.**
- **Has:** the network; its own channel's connections; the cookies of the requests it is sending, which it asks the trusted core for one request at a time (decision 62).
- **Does not have:** a cookie jar: it can neither read its channel's other cookies nor any other channel's, and a cookie it reports is stored only if its channel's host and scheme sent it (requirement 21.1); the trusted core's state; the screen, except through messages the trusted core checks.
- **Can:** send false pages and contents for its own channel, which that channel's server, or a network attacker on a plain connection, could do anyway.
- **Cannot, on every platform** (decisions 52 and 58): read any file but the system's name-resolution files, start a program, or reach any service but the resolver's: on macOS its profile starts from "deny everything" (section 13); on Linux, Landlock with the resolver's directories alone and a system-call filter allowing only the internet families' sockets and routing netlink, no local socket (decision 86); on Windows an AppContainer with the internet client capability, in a job (`sandboxing.md`).

- **Local network (decision 89):** the refusal of this computer and the local network (requirement 5.10) is the engine's check, inside the fetcher. A compromised fetcher or phone keyboard is kept off this computer's addresses by its macOS profile, which refuses `localhost` unless the user allowed the local network, and on Windows by its AppContainer; it is not kept off the local network beyond this computer on macOS (the profile language cannot name private ranges), nor off either on Linux. Connecting in the broker and passing the socket to the fetcher is the planned design; it needs its own design and security analysis.

**Remaining risk:** low.

### 5.2 Boundary 2: the fetcher into the trusted core

**Threats.** False results: oversized contents, hostile titles and section names, bytes in place of pages.

**Mitigations.**
- The protocol's bounds: at most 1,000 contents entries; every title and name cleaned and cut on arrival (requirements 4.2 and 4.8).
- Page bytes are cut into pieces and passed to the channel's decoder unread (requirement 5.3).
- A malformed message stops the fetcher (requirement 4.3).
- Only the trusted core decides which decoder a fetcher's bytes reach: its own channel's.

**Remaining risk:** low. It depends on the protocol's readers being correct, which is why they must be fuzzed (requirement 4.6, met: `fetcher_messages`).

### 5.3 Boundary 3: the trusted core and the decoders

**Threats.** Faults in content detection, the format readers, the decoder or the renderer, triggered by crafted teletext; a compromised decoder sending false frames or floods of them.

**Mitigations.**
- Every channel has its own decoder process (requirement 7.2), with no network (requirement 3.4).
- Every frame is checked on arrival: the exact raster size, at most 32 colours, every index within the palette, a valid page number (requirement 9.2).
- Frames are passed on only for pages the interface shows, and only when changed (requirements 9.4 and 9.5).
- Titles are cleaned and bounded (requirement 11.5).
- The trusted core writes to each decoder, each fetcher, the phone keyboard and the interface through a queue of its own with a writing thread (`core/src/outbox.rs`), never from the thread that owns the tabs. A decoder that stops reading fills its queue, 16 MB, and is then killed and replaced, with what it was given (decision 47); the core never waits for a process to read, so a decoder that stalls, by fault or on purpose, stalls its own channel only.
- What a helper sends waits for the trusted core in a budget of its own, 16 MB (`core/src/budget.rs`, finding F-10): each helper's reading thread takes a message's bytes from it before passing the message on, and waits while it is spent, until the core has handled earlier ones. A decoder, fetcher, sound helper, phone keyboard, updater or interface that writes faster than the core handles its messages stalls on its own full pipe; the core's memory does not grow with it, and the core's thread never waits on a budget (`a_helper_that_floods_the_core_is_held_at_its_budget`; the updater's reader since 24 October 2026, `an_updater_that_floods_the_core_is_held_at_its_budget`).

**If a decoder is compromised.**
- **Can:** draw any picture into its own channel's page images and give its pages any title, which its server could do anyway.
- **Cannot:** reach the network, other channels or the trusted core's state.
- **Cannot, on every platform** (decisions 52 and 58): read any file but the system's own, start a program, or reach the network: on macOS its profile starts from "deny everything" and allows its pipes alone (section 13); on Linux, Landlock, its own network namespace and a system-call filter; on Windows, a restricted token in an AppContainer with no capabilities, in a job (`sandboxing.md`). A layer that cannot be had stops it from running.

**Sound (requirements 16.1 to 16.4).** A decoder also reads its channel's audio packets, with the engine's bounded, fuzzed audio receiver, and sends voice operations and 16-bit samples. The trusted core passes on every tab's (decision 38), re-checks each against the sounds that decoder defined, and holds the sound helper to 32 MB of a channel's samples and 64 MB in all (`core/src/hearing.rs`, decision 55). A compromised decoder **can** play any sound in its own channel's four voices, as its server could; it **cannot** reset another channel's sounds, exceed the bounds, or play when the user has no output device.

**Another browser's bookmarks (decision 71).** A decoder started with `--bookmarks` reads one Netscape bookmark file, the HTML another browser wrote, and nothing else; the trusted core gives it the file the user named and takes back only folders, their ends and bookmarks, each bounded, then holds every address to its address rules and cleans every name (requirement 20.25). A compromised reader **can** put wrong bookmarks under Imported, each an address the rules accept and the user sees before opening; it **cannot** reach anything a channel's decoder cannot, nor anything but the one file it was given.

**Remaining risk:** low.

### 5.4 Boundary 4: fonts

Removed by decision 66: no font is fetched from a channel, so there is no font file to read and no font helper. Pages are drawn in the engine's own fonts, which ship with the browser.

### 5.5 Boundary 5: the trusted core into the interface process

**Threats.** Teletext data reaching the interface's readers; deceptive characters in page text; secrets leaking into the interface.

**Mitigations.**
- Pages cross only as page images our encoder produced from checked palette indices (`protocol::frame`, decision 61). The interface's reader accepts only that shape: at most 32 colours, the screen colour and every dot within them, the dots filling 480 by 500 exactly, changes only against the frame it holds (requirement 9.3). An attacker controls dot values within a fixed raster and palette, never file structure.
- Text crosses only cleaned and bounded, and the interface refuses text that still holds a hidden character (requirements 4.8 and 11.5).
- Text is only ever drawn as text runs: there is no markup to inject into (requirement 10.2).
- No cookie, credential, setting or history beyond the strip's own state is ever sent (requirement 3.6).
- No sound crosses: the interface is told only which channels are sounding, for the tabs' speakers (decision 55; section 5.7).
- The Dock menu on macOS (decision 39) is the interface's one crate of its own with `unsafe` code, beside `confine`, which confines it at start (requirement 10.12): Objective-C runtime calls in the interface process that add a method to winit's application delegate and build the menu from tab titles the trusted core cleaned. Its input is our own tab list; its output is a tab index or mute-all, sent to the trusted core as ordinary intents.

**Remaining risk:** low. The image reader is our own, about a hundred lines with no third-party decoder, reading only our own encoder's output, and fuzzed (`page_image`, requirement 4.6).

### 5.6 Boundary 6: intents and strip input from the interface process

**Threats.** A compromised interface process sends messages to navigate, confirm a channel change, change security settings, read secrets, influence fetching, or exhaust the trusted core.

**Mitigations.**
- **Page intents carry no address.** They can report visible pages, click a cell, select, close, open or move tabs and windows, and send input to the open page's channel when it takes input (requirement 4.5, decision 48).
- **Visible pages affect only rendering,** never fetching: the fetcher takes from the trusted core only input messages for the channel's own push WebSocket and the answers to its cookie questions, none of which names an address, page or connection, so nothing the interface does can cause, delay or reorder a request (requirement 4.7). This closes the covert channel between channels that the earlier design found (`prototypes/docs/security-analysis.md` section 5.6) for every channel fetched from a contents file or a bundle, and for files.
- **Visible pages of another channel are filtered out** by the trusted core (requirement 4.4).
- Every message is bounded and checked; a malformed one ends the application (requirements 4.2 and 4.3).

**What a compromised interface process can do.** The trusted strip is drawn by the interface process, so its integrity rests on that process (decision 33; requirement 3.9):
- **Can:** draw a false strip; send strip input, and so navigate any tab to any address the trusted core's rules accept, go back and forward, reload (decision 63: the active tab's channel opened again at the same address, which the trusted core holds; the message carries nothing), and set bookmarks; change any setting the settings panel offers, the proxy and the local addresses allowed among them; confirm the strip's questions, the site-change screen among them; install a version the trusted core offers, which runs only if both keys signed it (section 8); type into the open page's channel when it takes input (decision 48); see every page on screen; open the history panel and the bookmarks manager and page through them, reading every row 100 at a time, and remove history or bookmarks by the rows' numbers (decision 57; `history-and-bookmarks-security.md` section 4, its first risk).
- **Cannot:** read cookies, history beyond the panels' views, or the user's files (its sandbox denies them on every platform: on Windows its AppContainer since decision 90, verified in the virtual machine on 24 October 2026); reach the network; change what any fetcher requests. That it can confirm the site-change screen and change a security setting is the cost of option 1 that open decision 3 weighed (decision 41 kept the strip in the interface process); each change is checked by the trusted core's rules and said in the strip (`settings-home-console-proxy-security.md`).
- **What reaching it takes:** a fault in NeoTeletext's own interface code (no `unsafe`, requirement 10.12) or in its drawing libraries (`vello_cpu`, Parley, winit, softbuffer), reached only through our own images, cleaned text and the user's own input.

**Remaining risk:** low likelihood; high impact within its reach (navigation under the user's eyes, without secrets). Controls: small code, no `unsafe`, fuzzing of the interface's readers and the hostile interface harness (requirements 4.6 met, 3.9 met in part), and open decision 3.

---

### 5.7 Boundary 9: the trusted core and the sound helper

**Threats.** A compromised decoder sends voice operations to make the sound helper hold memory without bound, reach another channel's sound, or reach a fault in the mixer or the system's sound library. A compromised sound helper lies about which channels sound, or floods the trusted core.

**Mitigations.**
- Sound crosses only as voice operations and 16-bit samples, each bounded by the protocol and checked by the trusted core (requirement 16.4). The sound helper parses no audio format; it copies samples into one engine mixer per channel, which has no `unsafe` code. Each channel's sound is tagged by the trusted core, not the decoder, so a decoder cannot reach another channel's sound; only the trusted core can reset or mute a channel.
- The sound helper is confined to the sound output alone (decision 55): on macOS the audio services and no network, files or programs; on Linux `AF_UNIX` for the sound server and read of the sound devices; on Windows an AppContainer with no capabilities. The interface no longer reaches the sound output, and no longer loads the system's sound library.
- What it sends back is one message, the channels sounding, bounded by the number of tabs and read by the trusted core's fuzzed reader (`sound_messages`); a lie keeps a tab awake or shows a speaker, nothing more. If it ends or cannot start, the browser runs silent (requirement 16.6).

**Remaining risk:** low. A fault in the system's sound library, reached through checked samples, is contained in a process that holds nothing.

### 5.8 Boundary 10: the trusted core and the broker

**Threats.** A compromised trusted core uses the broker, the one process not confined, to start a program of its choice, start a helper with a wider sandbox, read the user's files, or reach a fault in the broker's parser.

**Mitigations** (`sandboxing.md` section 7).
- The core names one of the browser's own six helpers (decoder, fetcher, interface, phone keyboard, sound helper, updater; `confine::broker::Program`), never a path; the broker decides the role, and so the sandbox, from the program, and starts it from its own folder. The helper checks its arguments as it always has.
- A file channel is read by the browser's own channel rules and bounds (64 MB, 4,096 files), and only when the engine's `detect` recognises it as teletext in a format the decoder is built with, or an import's bookmarks file by its first line; never beneath an entry of the home folder beginning with a dot, the platform's folders of applications' data, or the browser's own folders, the only ones the confined core writes, so no link or hard link it makes there is read; and never through a link, the file being opened without following one and checked by its descriptor. Every refusal is one sentence, so none tells whether a file exists (decision 88). Not built: a file chooser owned by the broker, which would close this boundary; the residual is in rank 4 of section 16.
- A web address is checked by `web-opener`, as for a link (requirement 17.9), and the broker opens one at most once every two seconds, the core's own interval between hand-offs and one constant for both (decision 88, finding F-61). Its address is not checked further: any data fits in a valid address, so a compromised core can still send a little through the system's web browser, as fast as a link could; the fetcher's start arguments are not checked, for the same reason.
- Every request is a kind, a length and a payload read within its bound; at most 512 helpers alive and 64 arguments of 4,096 bytes; a request the broker cannot parse ends the core. The broker's request and reply readers are fuzzed (`broker_requests`, `broker_replies`). The broker reads nothing else from outside.

**Remaining risk:** low. The broker is small and unconfined: a fault in its parser would give a compromised core the user's rights. It parses only a handful of fixed messages.

## 6. What leaving the web engine changed

The earlier design's main concession was the system webview: many millions of lines of C++ and Objective-C with a steady flow of security fixes, which it contained with a window host, a Content Security Policy, the removal of WebRTC, and on Linux an outside confinement, a nested display and a display host of its own (`prototypes/docs/security-analysis.md` section 6).

**With no web engine:**
- **The three levels of compromise collapse to one:** a fault in the interface process (section 5.6).
- **No script runs,** so no package supply chain runs in the interface, and no script can reach the network.
- **The interface process can be denied the network on every platform,** including macOS, where proof A showed a webview cannot (`prototypes/docs/proofs.md` section 2). Built on macOS.
- **On Linux,** the interface needs the display as any application does, and nothing more; no nested display or display host is needed, because nothing in it is a web engine to contain.
- **The cost:** NeoTeletext owns its interface code, including what browsers gave for free: text editing, input methods, accessibility. Each is our code and must be tested as such.

---

## 7. Spoofing and the line of death

**The threat.** A teletext page can draw anything at teletext resolution: a fake address bar, a fake site-change screen, a fake button, a fake system dialog.

**Mitigations.**
- The address, identity, encryption state and the site-change screen are shown only in the trusted strip, which is drawn outside the page area and which no page image is ever drawn over (requirements 11.1 and 11.2).
- **Every page image is kept below the strip,** including the zoomed page and the overview's popup of a page at its native size, which shrinks on a short window rather than cover the strip (observed in this codebase's tests).
- A click on a page means only "this cell" (requirement 12.3).
- The strip stays visible in every layout but the page-only full screen, which ends before anything leaves the channel and brings the strip back at the top edge (requirement 11.4, `full-screen-security.md`).
- Every new layout is reviewed for spoofing (requirement 14.5, no review recorded yet).

**Remaining risk:** medium. This is where safety depends on discipline in visual design: an interface built for animation is exactly where a security indicator could be hidden or imitated.

---

## 8. The application update mechanism

Built 23 October 2026 (decisions 54 and 73 to 82): `installation-and-updates.md`, with its own analysis in `installation-and-updates-security.md`, whose section 4a reviews the build. The Update Framework with offline threshold keys and a second, platform signature by a key held apart; side-by-side installation, an atomic switch, rollback after two failed starts; a confined updater that only stages; updates suggested and installed only when the user accepts. Until the key ceremony the production identity accepts nothing (decision 76). Its remaining risks are key custody, a fault in the launcher, which is not confined, and on Linux the AppArmor profile of decision 77.

---

## 9. Supply chain

**Threat.** Malicious or vulnerable code in a dependency.

**Mitigations.**
- **No run-time JavaScript and no package manager for the interface.** The earlier design's medium-likelihood risk from React and its packages is gone.
- **Rust dependencies pinned to exact versions** in `native/Cargo.toml`, every one permissively licensed (decision 34); lockfile committed. `cargo deny check` runs in the full gate for both workspaces, `native` and `native/tuf`: licences, sources and the RustSec advisories that `cargo audit` also reads; `cargo audit` itself is not run (requirement 14.3, partly met).
- **Where a dependency sits limits what it can do.** The trusted core depends on the engine's `neoteletext-core`, `neoteletext-input` and, of `neoteletext-web`, its cookie jar, its proxy-address parser and its address type; and on the protocol, `confine`, `update`, `release-identity`, `web-opener` (for the broker), `ring`, `keyring` and `unicode-normalization` (requirement 3.2, which needs the owner's word on the cookie jar). The drawing libraries sit in the interface process, the engine's readers in the decoder and fetcher.
- **The engine** is pinned to one commit in its own repository and never patched here (requirement 2.5).

**Remaining risk:** low for the trusted core. Low to medium for the interface process's drawing libraries, which are young (`vello_cpu` 0.3, Parley 0.12) and run in the process that draws the strip.

---

## 10. Local attackers

- **No network ports are opened** by the application. Messages travel over pipes between our own processes.
- **No debugging interface** in any build (requirement 10.8).
- **Under X11** any local process can send input to the window, as to every X11 application; the trusted core still applies its rules to everything it receives.
- **History, bookmarks, settings, the session and the sealed cookie file are kept in the settings folder,** readable by the user alone (decisions 57, 59 and 62); the trusted core is confined to that folder (decision 56), not to an App Sandbox container.

**Remaining risk:** low. A process running as the user can read the user's own files; that is outside this analysis's scope.

---

## 11. Availability

**Threats.** Floods of pages or frames, channels with thousands of pages, endless push feeds, decompression bombs, decoders that stop answering, many tabs.

**Mitigations.**
- Frames only for visible pages, and only when changed (requirements 9.4 and 9.5).
- Every message bounded; at most 1,000 contents entries; files at most 64 megabytes or 4,096 page files.
- The engine's response and decompression limits (requirements 5.11 and 5.12).
- A decoder or fetcher that breaks the protocol is stopped; its channel's pages stay.
- A decoder that stops reading what the trusted core sends is killed once 16 MB wait for it, and started again with what it was given; the trusted core's own thread never waits on a process (`core/src/outbox.rs`, section 5.3). An interface that stops reading ends the browser with a message, since without it there is no browser.
- The window redraws only on input, change or animation; a folded overview draws nothing and starts no animation (observed in this codebase's tests).

**Not yet:** the live-feed threshold (requirement 6.5). At most 64 tabs in at most 16 windows (`protocol::MAX_TABS`, `MAX_WINDOWS`, decision 72) bound the decoders and fetchers.

**Remaining risk:** low.

---

## 12. Privacy

- **Fetching never depends on viewing** (decision 22): a channel's server cannot see which of its pages a viewer reads (requirements 4.7 and 5.15).
- **Cookies are host-only and channel-only,** held by the trusted core and sent only with their own channel's requests; there is no third-party context (requirements 7.6, 7.7 and 21.1). Persistent cookies are kept between runs only sealed, never a session or private one; the cookies panel deletes them by channel, site or all, the settings keep them, delete them when the browser closes or block them, and history's clearing takes them by time (requirements 21.4 to 21.8).
- **No `Referer` header, and a minimal user agent** (requirement 5.14).
- **The interface process stores nothing** (requirement 7.9).
- **History stays on the computer** (decision 57): kept 90 days in `history.txt`, readable by the user alone, sent to no channel, decoder or fetcher (requirement 20.12) and never over the network; the user removes any of it, or all, from the panel, or browses in a private tab that records nothing (requirements 20.6 and 20.8; "Remember history" was withdrawn by decision 65, as Chrome has none). Like every browser's, it is not encrypted on the disk. The session (decision 59) holds the open tabs' addresses the same way, never a private tab's; turning restoring off does not remove it.
- **Removing history reaches the tabs** (decision 64, as Chrome): the removed entries leave every tab's back-and-forward list but the entry it is at, and the recently closed tabs. Closed tabs are kept on the disk with the session, as Chrome keeps them (requirement 20.21).
- **No telemetry.**
- **A plain-HTTP channel is unencrypted;** `https` channels use Transport Layer Security (decision 41). The strip says "Not secure" for plain HTTP (requirement 5.9), where a network observer can read what is fetched.
- **Closing a tab** stops its fetcher at once, which the channel's server can see; the grace period is required (requirement 5.17).

---

## 13. Platform notes

| Platform | Notes |
|---|---|
| macOS | The trusted core, decoder, fetcher, phone keyboard, interface and sound helper run under `sandbox-exec` profiles that start from "deny everything" and import Apple's baseline for any process (`system.sb`: the loader, system libraries, time zones, random numbers), then allow what each is for (`confine/src/macos.rs`): the decoder nothing but its pipes; the fetcher and keyboard outbound TCP, the name resolver's socket and files, and no file reads otherwise; the interface the system's folders, metadata of any path, this program's own cache and temporary folders under `/private/var/folders` (finding F-52, not all of it), and the services of the window, fonts, text input, the pasteboard, notifications, preferences and the Dock, with no network and, since decision 55, no sound; the sound helper the audio services alone; the trusted core read and write in its settings folder and nothing else, no network and no services. None may start a program. The profiles were made by running each process under candidates (21 October 2026): the kernel reports denials only to administrators' logs, so a denied operation shows as a crash, and the sound services' absence as a silent browser. Children get an environment of only what they need (`spawn`), and every path a profile names is passed as a `sandbox-exec -D` parameter, never written into the profile's text (decision 97). **The trusted core is confined behind a broker** (decision 56): macOS refuses a nested `sandbox-exec`, so the broker, the program the user starts and the one process not confined, starts the core under its profile and every helper on the core's request under the helper's own (section 5.8). `sandbox-exec` is deprecated by Apple and still present. |
| Windows | Built 22 October 2026 (`sandboxing.md`, decisions 52, 56 and 58): the broker starts every helper in a job object (kill-on-close, one process, UI restrictions for those without a window), with the process mitigation policies (DEP, ASLR, control-flow guard, no dynamic code, no child processes, no remote or low-integrity images, and the Win32k filter for the decoder and fetcher), with only the two pipe handles inherited and the environment the core chose; the decoder in a less privileged AppContainer with no capabilities and the fetcher and keyboard in an AppContainer with the internet client capability (and the private network one when the user allowed local addresses); the interface in an AppContainer of its own with no capabilities (decision 90: before, a restricted token at low integrity, which could still read the user's files and reach the network; its refusals verified in the virtual machine on 24 October 2026, its window in the container still to be checked by hand); the sound helper in an AppContainer with no capabilities; the trusted core, started by the broker, in a less privileged AppContainer with no capabilities and the Win32k filter, its settings folder granted to it (decision 56; `shell32.dll` is delay-loaded in `neoteletext.exe` so that the core, denied Win32k, can start). An AppContainer reads only what grants read to `ALL APPLICATION PACKAGES`: packaging sets that on the program folder, and without it the process is not started at all (decision 58). Built and tested in the virtual machine on `foundintelquiet4060`. |
| Linux | Built 22 October 2026 (`sandboxing.md`, decisions 52 and 58): each helper confines itself first thing in `main`, before any thread: no new privileges; for every role without the network an empty network namespace; Landlock 5, enforced whole, with only the directories the role may read (none for a decoder; the resolver's for a fetcher; fonts, preferences and the system's for the interface) and no TCP for the roles without the network; and a seccomp deny-list of system calls: no programs, no tracing, no mounting, no kernel interfaces a helper has no use for, and sockets only of the families the role needs (none for a decoder or the trusted core; the internet's and routing netlink for the fetcher, keyboard and updater; `AF_UNIX` for the interface and the sound helper, which since decision 86 have their own `/run`, `/tmp` and runtime folder holding only their display's or sound server's sockets, so no helper reaches the session bus). The trusted core confines itself the same way, with its settings folder writable, before it reads its broker's channel (decision 56). Every layer is required (decision 58): a machine without Landlock 5, without unprivileged user namespaces (a default Ubuntu from 24.04 needs an AppArmor profile for the browser, which every Linux release carries as `neoteletext.apparmor` for an administrator to install, decision 77) or without a filter for its architecture does not run the browser, and says why. Measured on Ubuntu 26.04 (kernel 7.0): the crate's probe tests pass and the browser runs under Xvfb with every helper reporting its layers. |
| Android, iOS | Later. On iOS, with one process, the decoder's only wall is whatever open decision 2 chooses. |
| Microcontrollers | A single-purpose device with one program; it needs its own analysis once its features are decided (open decision 5). |

The updater (decision 73) is confined on all three as `installation-and-updates.md` section 8.1 describes: the update host's one port and its staging folder. The launcher, like the broker, is not confined.

---

## 14. Comparison with the alternatives

| Design | Web engine | Script at run time | What compromising the interface gains | Interface richness |
|---|---|---|---|---|
| The earlier design (React in a webview, window host, native strip; `prototypes/docs/`) | Yes, contained | Yes, as page code | Level 1 (page code): wrong pixels and the use of open channels; level 3 (an escape into the window host): a false strip and forged strip input, no secrets | Full |
| **This design, adopted** (our own native interface in its own process, drawing the strip) | **No** | **None** | A false strip and forged strip input, no secrets; reaching it needs a fault in our own code or its drawing libraries | Full, by our own drawing |
| The same, with the strip in a smaller separate process (open decision 3) | No | None | Wrong pixels in the page area only | Full |
| egui (the engine's own design) | No | None | Everything, in one process | Lowest |

---

## 15. Findings from this codebase

These were observed, not reasoned:

1. **The macOS sandbox profiles allowed reading files.** They denied network and file writes from "allow everything". Closed on 21 October 2026: every profile now starts from "deny everything" (section 13).
5. **A process denied Win32k cannot start if it imports `shell32.dll`.** The trusted core under its Windows sandbox ended before its first line (`STATUS_DLL_INIT_FAILED`) until `shell32.dll`, needed only by the broker, was delay-loaded (`sandboxing.md` section 7.4); a test now starts the core confined on every platform.
2. **macOS reports a slow mouse-wheel notch as a tenth of a line,** and speeds up quick ones (measured 16 October 2026). Not a security finding; recorded because the interface's handling of input is our own code now, and each such assumption must be measured.
3. **The engine's user agent fetches a channel in its contents file's order at its own fixed rate,** whatever is on screen: measured, 218 of 800 pages in 45 seconds. The fetcher takes no input from the trusted core, so the interface has no path to influence fetching. *Since decisions 44 and 62* the fetcher also takes input messages for its push WebSocket and cookie answers; neither names an address, page or connection (requirement 4.7).
4. **The interface's protocol refuses every kind it may not receive,** and messages with hidden characters, oversized lengths or trailing bytes (tests in `protocol`).
6. **A sandbox profile can hide the window without an error.** On macOS 15.6 the interface's profile lacked the window manager's service, `com.apple.windowmanager.server`: the window was made and drawn, never shown, and nothing failed (23 October 2026, `sandboxing.md` section 6). A denial shows only in the system's log; a missing service is found by allowing all lookups once and reading the denials.

---

## 16. Remaining risks, ranked

| Rank | Risk | Likelihood | Impact | Main controls |
|---|---|---|---|---|
| 1 | Spoofing of the trusted strip by page images, especially in animated layouts and popups | Medium | The user is misled about where they are | Strip outside the page area; page images kept below it; recorded layout reviews (requirements 11.1 to 11.4, 14.5) |
| 2 | A fault in the interface process, which draws the strip | Low | A false strip and forged navigation; the history and bookmarks the panels show, read through them and carried out by a navigation (decision 57) | Our own code, no `unsafe`, inputs limited to our own images and cleaned text; views of at most 100 rows asked for by number; fuzzing and the hostile harness (requirements 3.9, 4.6, 10.12, 20.5); open decision 3 |
| 3 | **As built:** the broker and the launcher are not confined | Low (needs a fault in the trusted core and then in the broker's parser, or in the interface) | The user's rights | Every other process confined on macOS, Windows and Linux (decisions 52, 55, 56); the broker parses a handful of bounded, fuzzed messages and decides every role itself (section 5.8) |
| 4 | A fault in the trusted core, the process holding the browser's state | Low | The settings and every channel's content, and a false strip; not the user's files or the network, which its sandbox denies (decision 56). What remains through the broker: files elsewhere that look like teletext, a recording or page file anywhere but the refused places, which the broker reads for it (decision 88) | Small, no format readers, bounded protocol, no `unsafe` (requirements 3.2, 4.2, 14.2); fuzzing (requirement 4.6); confined behind the broker, which reads only teletext outside the dot-folders, applications' data and the browser's own folders, never through a link, and opens a web page at most once every two seconds |
| 5 | A plain `http` channel | Depends on the channel | That channel readable and alterable on the network | Transport Layer Security with `rustls` and Mozilla's roots for `https` channels (decision 41); a plain channel shown as "Not secure" |
| 6 | Theft of the update root keys at threshold | Low | Malicious update | Threshold, offline hardware keys, separate holders (requirements 13.8, 13.9) |
| 7 | A supply chain compromise of a Rust dependency | Low | Depends on the process it sits in | Pinned versions, permissive licences, review; `cargo-deny` and `cargo audit` (requirement 14.3) |
| 8 | A decoder or fetcher fault, with the sandboxes complete | Low | Contained to one channel | Separate processes per channel, no rights (requirements 3.3, 3.4, 3.12) |
| 9 | Exhaustion of resources | Low | An unresponsive tab or application | Visibility culling, bounds, on-demand drawing (section 11) |

---

## 17. Checks that keep these properties

Each check below belongs in the project's check script, run by git hooks (requirement 14.1). **Built** marks those that exist.

1. **Strict compilation:** `unsafe` forbidden, Clippy `pedantic`, warnings as errors (requirement 14.2). **Built.**
2. **Message bounds:** oversized, malformed and wrongly-kinded messages refused (requirement 4.2). **Built** in part, as unit tests.
3. **Text cleaning:** hidden characters removed and refused (requirements 4.8 and 11.5). **Built.**
4. **Frame checks:** wrong sizes, palettes and indices refused (requirement 9.2). **Built.**
5. **Image reader:** anything but our encoder's shape refused (requirement 9.3). **Built.**
6. **Trusted core dependencies:** no web engine and no format reader (requirement 3.2).
7. **Engine dependencies:** no interface toolkit; one pinned commit (requirements 2.2 and 2.5).
8. **Process rights:** network and file access fail where denied, on each platform (requirements 3.3 to 3.7). **Built** in part: on Linux `confine/tests/probe.rs` runs every role confined and checks what each is refused: files, programs, the session's sockets, the x32 numbers, the terminal and other processes' signals; on macOS the profiles' contents are tested (`confine/src/macos.rs`), and a confined fetcher's reach of this computer (`confine/src/spawn/tests.rs`); on Windows a confined interface's network and file reads (`confine/src/spawn/tests.rs`, run in the virtual machine on 24 October 2026), and the containers' making and environment.
9. **Fuzzing** of every reader in the trusted core and the interface (requirement 4.6). **Built:** `native/fuzz`, one target for each message reader, the decoder's engine, the interface's image reader and the address parser; the settings, history, bookmarks, session, cookie and Netscape bookmark files; input records, phone keys and relay messages; the launcher's state, the release manifest, the gzip reader, the version mark and the updater's messages; and in `native/tuf/fuzz` The Update Framework's metadata and the updater's HTTP responses.
10. **The hostile interface harness** (requirements 3.9, 4.5 and 4.7). **Built** in part: `native/core/src/trusted_core/hostile_tests.rs` against the trusted core, on macOS and Linux with a stand-in broker whose requests and helpers' input it checks (finding F-98: no helper, file or hand-off from a page intent, helpers only of the roles, every message a helper's reader accepts, no channel's file given to another's decoder); requirement 4.7's request comparison waits on the real helpers.
11. **Channel isolation:** cookies and pages never cross channels (requirements 7.3 and 7.8).
12. **Dependencies:** `cargo-deny`, `cargo audit`, lockfile review (requirement 14.3). **Built** in part: `cargo deny check`, with RustSec advisories, in both workspaces; `cargo audit` itself is not run.
13. **Updates:** the full set of update tests (requirement 13.12).
14. **Spoofing:** recorded review of every layout (requirement 14.5).
