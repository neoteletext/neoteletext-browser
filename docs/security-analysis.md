# NeoTeletext: security analysis

| | |
|---|---|
| **Status** | Analysis of the native architecture (decision 33), 16 October 2026. Where a point was observed on this codebase rather than reasoned, it says so. Where the design and what is built differ, both are given. |
| **Companion documents** | [`architecture.md`](architecture.md) describes the design. the requirements in rdm states each requirement this analysis relies on, numbered, with whether it is met; this document cites them as "Requirement N.N". The earlier design's analysis, with its web engine, is in `prototypes/docs/security-analysis.md`. |
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
| Viewer keys, which replaced cookies (DEC-6471) | The trusted core, one jar for ordinary tabs and one for private windows, one Ed25519 key a channel identity; a visit key in memory only, a remembered one kept between runs in `viewer-keys.bin`, sealed under a key in the platform's secret store (REQ-715, REQ-716, REQ-881, REQ-882) | Being known to a channel as the viewer it already knows, and the user's accounts on teletext services that rely on it |
| History, bookmarks, settings, sessions | Trusted core: history kept 90 days in `history.txt`, bookmarks in `bookmarks.txt`, settings in `settings.txt`, the session (the window and every tab but private ones, with their lists) in `session.txt`, each in the settings folder and readable by the user alone (decisions 57 and 59) | Private, and history most of all: which pages the user read and when; settings control security |
| The user's own files | The file system; opened by the trusted core when the user names one, and decoded by a decoder | Private |
| The integrity of the application's code | Installed versions, the launcher and its state file, and the update keys (section 8) | Everything else depends on it |
| Availability | Every process | The application must stay usable |

---

## 3. Who attacks, and how

| Adversary | Capabilities |
|---|---|
| **A hostile teletext service** | Sends any bytes in any role: malformed or enormous pages, crafted enhancement data, misleading page text and titles, a hostile contents file, a push feed that never stops, viewer-state challenges and ends |
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
a picture the user chose ──► broker ──[10]──► trusted core ──[3]──► decoder --picture ──[3]──► trusted core ──► broker ──► the place the user chose to save
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
| 2. Fetcher to trusted core | The contents file as typed entries; page bytes, forwarded unread; notices, exchanges for the console, push and input state, the descriptor's links, viewer-state challenges, ends and offers to remember, each of fixed length with enumerated values, and the kind of a refused certificate, one byte (decision 83, `certificate-errors-security.md`); the push WebSocket's round trip, eight bytes (decision 125); to the fetcher, input for its push WebSocket, each message with whether more of its moment follow and a move's contact, the round trip's switch, and viewer-state answers (a signature over bytes the trusted core built for the fetcher's own tab's channel, or none) | Untrusted, checked as data |
| 3. Trusted core and decoder | Bytes and whole files, shown pages, the open page and its view in; arrivals, updates, frames, links, sub-pages, voice operations, cursor and input-control records, channel log lines and, for an import, bookmarks out; for the Tools menu's picture converter, the panel's choices and a picture in, and a TTI page, its T42 packets and a summary out (`picture-tools-security.md`) | Decoder output untrusted |
| 5. Trusted core to interface | Images our encoder made, cleaned text, tab, strip and channel state | The interface's input; contains nothing secret |
| 6. Interface to trusted core | Intents and strip input | Untrusted intents; strip input trusted only as far as the interface process is (section 5.6) |
| 8. Update subdomain to updater | Metadata and release files | Untrusted until `tough` verifies it against The Update Framework's metadata (`installation-and-updates-security.md` section 4a) |
| 9. Trusted core and sound helper | Checked voice operations and samples, and the sound settings, in; which channels sound, out | The sound helper's report untrusted |
| 10. Trusted core to broker | Requests to start one of the browser's helpers, end or wait for one, read a file channel, open a web address, give the local time, give the viewer keys' sealing key, report the version healthy, read the launcher's state, read the picture the user chose to convert, and save a converted page where the user chose (`picture-tools-security.md` section 4.4) | Untrusted: the broker parses them within bounds and decides every role itself |
| 11. Trusted core and updater | A check or a download in; the newest version, a staged version or a failure out (`update::wire`) | The updater's messages untrusted: at most 512 bytes, read strictly, fuzzed (`updater_messages`) |
| 12. Launcher's inputs | The state file, a staged version's manifest and files, the broker's version mark and its `healthy` line | Untrusted: bounded strict readers, fuzzed (`launcher_state`, `release_manifest`, `version_mark`), then the pinned signer (`installation-and-updates-security.md` section 4a) |
| 13. Relay to phone keyboard to trusted core | The relay's messages; the phone's keys and text | Untrusted: the keyboard process's readers fuzzed (`relay_messages`, `phone_keys`); a compromised keyboard is a compromised local keyboard (`interaction-security.md` sections E and F) |

Boundary 4 of the earlier design, the font helper, is gone: no font is fetched from a channel (decision 66). Boundary 7, the strip in a separate host, is folded into boundary 6.

---

## 5. Analysis of each boundary

### 5.1 Boundary 1: the network into the fetcher

**Threats.** Faults in the protocol readers (the HyperText Transfer Protocol, the descriptor and contents readers); decompression bombs; slow or endless responses; redirect loops; requests aimed at the local network.

**Mitigations.**
- The engine's strict, bounded readers, through its user agent; the fetcher only moves bytes between the agent and its sockets (requirement 2.1).
- The engine's limits on size, time and redirects, and on decompressed size (requirements 5.11 and 5.12).
- The local network refused unless the user names an address (requirement 5.10). Without a proxy, the fetcher's transport also refuses, before connecting, a name that resolves to this computer or the local network, by the engine's rule and the channel's allowed addresses (finding F-62; `a_name_resolving_to_this_computer_opens_no_connection`), and tells the console why.
- **Through a proxy the rule is by name** (finding F-17): the proxy resolves, so the browser sees only the name it asks for. Addresses written as local IP addresses, `localhost` and names under `.localhost`, `.local` and `.internal` are refused before the proxy is asked (requirement 19.11); the engine's decision E.113 adds names of one label and names under `.home.arpa`, `.lan`, `.corp`, `.intranet`, `.private` and `.localdomain`, which reach the browser with the re-pin of 24 October 2026 at `5311f98`. A public name that resolves to a local address, such as `10.0.0.1.nip.io`, stays a residual through a proxy; resolving here and asking the proxy for the address would tell the local resolver every name, against the privacy a proxy is set for.
- One fetcher per channel (requirement 3.12).
- **Writes on a thread of their own** (decision 125, requirement 18.31): each open connection's writes go through a writer thread, so a server that stops reading holds up neither the fetcher's loop nor its other connection; what waits is counted, and bounded at 256 KiB (`MAX_UNSENT`), past which the connection is closed, as a write that times out is (`a_stalled_socket_is_told_behind_then_drained_and_holds_nothing_up`). The operating system's own socket buffer is not seen, as reading it needs unsafe code.
- **TLS resumed, never early data** (decision 125, requirement 18.33): every connection of a fetcher shares one rustls configuration with its in-memory store of sessions, so a reconnection resumes where the server allows, and early data (TLS 0-RTT), which an attacker on the path could replay with an input message in it, is never sent; TCP Fast Open is never used (`a_reconnection_resumes_the_tls_session_and_sends_no_early_data`). Sessions live in the fetcher's memory and end with it; a compromised fetcher already holds its own connections' keys.
- A certificate is checked against Mozilla's roots, compiled in (`webpki-roots`), never the system's store, and revocation is not checked: a certificate revoked but not yet expired is accepted (finding F-95; `certificate-errors.md` section 3).
- A connection, its proxy tunnel and its TLS handshake are opened on a thread of their own and the engine's connect limit bounds the whole, one deadline for resolution, the connects and the handshake (finding F-63; `a_proxy_answering_a_byte_at_a_time_fails_within_the_deadline`), so a slow or silent server or proxy holds up nothing else the fetcher does (decision 53); a connection closed while it opens ends its thread (`closing_a_connection_while_it_opens_ends_its_thread`). The system's resolver cannot be interrupted, so it runs on a thread of its own, which outlives the deadline until the resolver gives up by its own limits; since 3 November 2026 (TODO-1277) a fetcher runs at most three such threads at once, one for each of its connections, and a resolution beyond them fails at once as out of time (`resolver_threads_that_never_return_are_bounded`), so a name whose servers never answer, retried at the agent's rate, holds at most three. An opening that runs out of its deadline is said as the channel not answering in time, never as a refused certificate (`a_handshake_that_runs_out_of_the_deadline_is_a_timeout`). What remains: a close during resolution or the TCP connect itself ends the thread only when that step returns, within the deadline.

**If a fetcher is compromised.**
- **Has:** the network; its own channel's connections; the signatures the trusted core makes for its own tab's channel when that channel challenges, one at a time and bounded a minute (REQ-879, REQ-888).
- **Does not have:** a viewer key or any part of one: it cannot sign for its channel itself, nor have a signature made for another channel, since the trusted core takes the channel from the fetcher's own tab and builds the signed bytes itself (REQ-879); no cookie, since none is kept (REQ-889); the trusted core's state; the screen, except through messages the trusted core checks.
- **Can:** send false pages and contents for its own channel, which that channel's server, or a network attacker on a plain connection, could do anyway.
- **Cannot, on every platform** (decisions 52 and 58): read any file but the system's name-resolution files, start a program, or reach any service but the resolver's: on macOS its profile starts from "deny everything" (section 13); on Linux, Landlock with the resolver's directories alone and a system-call filter allowing only the internet families' sockets and routing netlink, no local socket (decision 86); on Windows an AppContainer with the internet client capability, in a job (`sandboxing.md`).

- **Local network (decision 89):** the refusal of this computer and the local network (requirement 5.10) is the engine's check, inside the fetcher. A compromised fetcher or phone keyboard is kept off this computer's addresses by its macOS profile, which refuses `localhost` unless the user allowed the local network, and on Windows by its AppContainer; it is not kept off the local network beyond this computer on macOS (the profile language cannot name private ranges), nor off either on Linux. Connecting in the broker and passing the socket to the fetcher is the planned design; it needs its own design and security analysis.

**Remaining risk:** low.

#### 5.1.1 A channel's tabs on one connection (neoteletext-common's DEC-6979)

Written 21 November 2026, after the build, since the feature was adopted by the owner's instruction before its analysis (DEVELOPMENT.md section 14). Every tab of one jar (the ordinary windows', or one private window's) that shows one live channel is served by one fetcher process (REQ-6980, DEC-6990, kept by DEC-6995), and each tab's push WebSocket is a stream, opened by extended CONNECT (RFC 9220), of one HTTP/3 connection over QUIC to the channel's server (REQ-6989). Fetches keep their own HTTP/1.1 connections.

**What changes for a compromised fetcher.** It now holds every tab of its channel and jar, where it held one. It can read the viewer's input in each of them, forge their pages and ask the trusted core for that channel's signatures (REQ-879): all things that channel's own server, which every one of those tabs shows, can already do or already receives. It holds nothing of another channel or of another jar: a private window's tabs of the same channel have a process of their own, two tabs whose addresses differ in any character get two processes, and the messages that add and take away a tab (`ToFetcher::TabOpens`, `ToFetcher::TabLeaves`; REQ-2946 item 5) carry nothing but the tab's number, so a fetcher cannot be pointed at another channel (REQ-2948). A process holds at most `protocol::fetcher::MAX_FETCHER_TABS` tabs; a second "tab opens" for a number it holds, or one past that bound, ends it. The trusted core takes a message only for a tab it added to that process and has not taken away. DEC-6995 weighed a fetcher per tab with a connection process per channel and jar, and found its protection is against the same channel's server, at the price of a new path between processes on three platforms.

**New readers of untrusted bytes, all in the fetcher.**
- **HTTP/3 and QPACK**, the engine's `neoteletext-http3` (REQ-6986): `no_std`, no unsafe code, the no-panic lint list, every bound a constant of its `limits` (a field section at most 16 KiB, SETTINGS at most 512 bytes, at most 8 interim responses); QPACK with the static table alone, this side offering a dynamic table of no capacity, so a reference to one is a connection error. Fuzzed as `http3_reader` and `http3_client` in the engine.
- **Alt-Svc**, the engine's `neoteletext-web::alt_svc` (REQ-6983): at most 512 bytes and 8 entries are read; only an `h3` entry for the origin's own host is taken, and a stream is asked for only on the push address's own port while the entry is fresh (REQ-6984). A server can make a tab try HTTP/3 only toward itself. Fuzzed as `alt_svc`.
- **QUIC**, through `quinn-proto` 0.11.19 with `rustls` and its `ring` provider, in the engine's `neoteletext-transport::quic` (REQ-6985): its packet and frame readers are `quinn-proto`'s, fuzzed by that project, not here; around them the engine bounds what a peer can make this side hold: no bidirectional stream and at most 8 unidirectional streams of the peer's, a receive window of 1 MiB a stream and 8 MiB the connection, at most 256 KiB unsent a stream, at most 64 streams, datagrams of at most 1,472 bytes, a 5-second handshake and a 15-second idle timeout with a keep-alive every 5. No early data (0-RTT) is ever sent, since it could replay a WebSocket's first input message. The certificate is checked against Mozilla's roots, as for TCP. Each address is checked against the local-network rule before the first datagram is sent to it (finding F-62; `the_shared_connection_reaches_no_address_the_channel_may_not`), and a fetcher with a proxy has no shared connection at all, since nothing may go direct (requirement 19.10; `a_transport_with_a_proxy_shares_no_connection`). TODO-6993 item 5 owes a test of the driver against a hostile QUIC peer.

**The UDP rule on each platform** (REQ-6981). The trusted core asks the broker for a fetcher with its channel's port only when sharing is on, no proxy is set and the channel is served over TLS; the broker refuses a port for any other program.
- **macOS:** the fetcher's profile gains two rules: `(allow network-outbound (remote udp "*:<port>"))`, before the refusal of `localhost`, which therefore holds for UDP as for TCP; and `(allow network-bind (local udp "*:*"))`, since the socket is bound before it sends and the profile language takes no port 0 for the ephemeral port it binds (without it the QUIC connection could not start, "Operation not permitted", found on the Mac on 21 November 2026). No `network-inbound` rule is given: a port bound under these rules received nothing a local program sent to it, and only the connected server's answers arrive. A compromised fetcher can send datagrams to that one port of any host, as it could already open TCP connections to any host, and bind UDP ports that receive nothing; it gains no other port.
- **Linux:** nothing is added. The system-call filter already allowed the internet families' sockets of every type, since the resolver asks over UDP, and Landlock's network rules cover TCP alone and are not applied to the fetcher's role, so a compromised fetcher could send UDP anywhere before this change and still can.
- **Windows:** nothing is added. The AppContainer's internet client capability covers outbound UDP as TCP and names no port.

**The remembered failure.** A shared connection that cannot be made (no answer within the handshake bound, an ICMP refusal during the handshake, or a server without extended CONNECT) is remembered in the fetcher's memory for five minutes (`HTTP3_RETRY_AFTER`, REQ-6989 item 5; `a_remembered_failure_ends_five_minutes_after_it`), during which each tab opens its own HTTP/1.1 connection at once; it is forgotten when a tab's TCP connection fails too, since the server was then not reached at all (`a_failure_is_forgotten_when_the_tabs_tcp_connection_fails_too`). An attacker on the path who forges ICMP refusals or drops UDP can hold the channel's tabs on HTTP/1.1 for as long as they keep it up: the fallback is TLS over TCP as before, so this costs the server connections, not the viewer security. ICMP refusals are ignored once a connection is up, so a forged one cannot end it.

**When the shared connection is lost** every tab on it resumes by itself (DEC-6979 item 7), so a server, or an attacker on the path, that ends the one connection ends every tab's push stream at once; each then resumes as it would after its own connection's loss.

**Verified, 21 November 2026.** A confined fetcher carried two tabs of `https://app.neottx.com/hackaday/` as two streams of one QUIC connection, holding one UDP socket and the two tabs' fetch connections, while the server held those two TCP connections alone; with sharing off it held no UDP socket and two TCP connections a tab. On Linux (chunko, Landlock and seccomp, release build), on Windows 11 (the virtual machine, AppContainer and job object, release build) and on macOS (the Mac, `sandbox-exec`, debug build started beside the owner's work, after the bind rule above).

**Remaining risk:** low. The new readers are bounded and fuzzed or, for QUIC's own, an approved library's; the gain for a compromised fetcher is the other tabs of the same channel and jar, which their server already controls.

### 5.2 Boundary 2: the fetcher into the trusted core

**Threats.** False results: oversized contents, hostile titles and section names, bytes in place of pages.

**Mitigations.**
- The protocol's bounds: at most 1,000 contents entries; every title and name cleaned and cut on arrival (requirements 4.2 and 4.8).
- Page bytes are cut into pieces and passed to the channel's decoder unread (requirement 5.3).
- A malformed message stops the fetcher (requirement 4.3).
- Only the trusted core decides which decoder a fetcher's bytes reach: its own channel's.
- **A page's list of sub-pages** (decision 121; neoteletext-common's REQ-1888 and DEC-2116, the parts `NNN/P.sub` in the page's folder) is read in the fetcher, a part at a time, by the engine's bounded and fuzzed reader (at most 4,119 bytes a part), never in the trusted core, and crosses as one message a part (`FromFetcher::SubPages`): the page's version of at most 16 bytes, its count within the standard's 8,190, and at most 64 rows from a part's first place, each a sub-code of the thirteen bits and a title of at most a row. A live channel's list, from its SUBPAGES records, crosses in the same parts once its whole sending has ascended over every part and numbered its count. The trusted core refuses a part that is not at a part's first place, holds more than 64 rows, runs past the count or does not ascend; a part of another version replaces what it held; it cleans every title, asks only for the parts the open page and the list's rows want, and holds at most 16 lists a channel, the least recently used forgotten first, so a fetcher cannot make it hold more than about 21 MB of lists a channel (16 of 8,190 rows, each title at most 160 bytes). A compromised fetcher **can** list any sub-codes and titles for its own channel's pages, which its server could; it **cannot** make the trusted core fetch anything (the trusted core asks, section 5.6), or reach another channel's sub-pages.

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

**Pictures to convert (the Tools menu, `picture-tools.md`, DEC-2390).** A decoder started with `--picture` takes the panel's choices and one picture the user chose, decodes it with the engine's bounded, fuzzed picture converter (the `image` crate's memory-safe decoders, the engine's REQ-2330), and answers once with a TTI page, its T42 packets and a summary, each bounded, or is killed after 30 seconds. The trusted core never decodes the picture or parses the page: the page goes unread to a channel's decoder at `neoteletext:picture`, and the summary is cleaned. A compromised picture decoder **can** draw any picture in the converted page's tab, as a channel can; it **cannot** reach anything a channel's decoder cannot. A channel's decoder's picture answers are ignored (`picture-tools-security.md` sections 4.1 and 4.2).

**Animation (REQ-6536; neoteletext-common's `docs/animation.md`).** The decoder reads five more records, each with the engine's bounded, fuzzed reader (`ANIMATE`, `SPRITES`, `NAPLPS_OBJECTS`, `SHEET`, `FRAME`), and keeps animation state, frame cycles, sheets and atomic frames. All of it is inside the decoder, so a channel gains nothing beyond what it had: drawing any picture into its own pages. What a channel's bytes can make the decoder hold is bounded by named constants: a frame holds at most 262,144 bytes for at most 120 ticks, for at most 4 pages (the engine's `MAX_FRAME_BYTES`, `MAX_FRAME_TICKS`, `MAX_OPEN_FRAMES`), and whatever ends it, what it held is applied, never kept; a sheet is at most 65,536 bytes a page, for at most 4 pages a renderer; 64 objects a sub-page hold state and 16 cycle; the teletext state is kept for at most 16 sub-pages; NAPLPS bytes held behind a WAIT are at most 262,144, and waits play for at most a minute a generation. What a channel can make the decoder do each tick is bounded too: a ReGIS repaint by the repaint budget (REQ-6380), a NAPLPS repaint for animation by 153,600 pixels a tick, a cycle by its period of at least one tick; the loop wakes at most once a tick for animation, and only while something is due. The animation state is the receiver's alone: no byte of a stream can read it (ANIM-6), so a page learns nothing of what the viewer's pointer or the page's rules did from it. The trusted core's checks of a picture are unchanged, and hold for every rectangle the decoder sends: a known page, a rectangle inside the raster, every index inside the palette. A NAPLPS picture now carries a pick record, checked as a ReGIS picture's is. A compromised decoder **can** animate its own channel's pages falsely, and nothing more.

**Painting as a show (neoteletext-common's DEC-7031; REQ-7024, REQ-7025, REQ-7026; recorded 21 November 2026).** One more record, `PAINT` (type 34), read by the engine's bounded, fuzzed reader, which refuses any length but 14 bytes, a line speed outside 300 to 115,200 bits a second, a default paint rate outside 1 to 32,767 or stated for NAPLPS, and a record that states neither. It adds no output and no new reach: it changes only when the bytes of a channel's own picture are drawn. What it can make the decoder hold is bounded: one record waiting for each sub-page and language, four a language, the oldest key dropped first; the bytes a paced stream waits in, at most 262,144 a sub-page in either renderer (the figure a NAPLPS WAIT already held), after which pacing ends and all is drawn; and one record kept with each kept picture's source, counted in that store's budget. What it can make the decoder do is bounded: a paced stream wakes the loop at most once a tick, and only for a sub-page on screen; the slowest line empties the largest hold in 524,288 ticks, and the viewer's setting, or reduced motion, ends every pace at once. A paced generation is presented before it is whole, which the page swap otherwise forbids; what is shown is still only the channel's own stream, drawn as far as the line has delivered it. The `painted` events are the receiver's own, given to the open page's rule table alone, at most 32 watched drawings a table and the whole picture at most once a tick, under the evaluator's existing budget of events a tick; a rule that answers one by starting another painting is paced by the tick, never by a loop within one. Nothing of them leaves the decoder unless a rule of the table reports it, through the trusted core's existing gate (section 5.10).

**Remaining risk:** low.

**Renderer parity (neoteletext-common's `docs/renderer-parity.md`, section 19; REQ-6770, REQ-6771; reviewed 19 November 2026).** What this work added that reads outside input or holds memory for it, and what bounds it, by the specification's numbering:
- *Threat 8, a picture's links mislead a viewer.* Links on NAPLPS objects (PAR-28) are the `DRAWING_LINKS` record already read for ReGIS drawings, by the same bounded reader, and are followed by the trusted core under the rules of every link: the destination shown in the strip while the pointer is on the object, the dwell time, and the question before leaving the channel (REQ-6376). The decoder only gathers the record; it decides nothing about following.
- *Threat 9, activation where the viewer did not point.* Small-target picking (PAR-29) is decided in the trusted core from the pick record it holds, never from the interface: the interface says the pixel and how many picture pixels its screen pixel covers, the reader refuses a span of 0 and a pixel no picture has, and the trusted core looks at no more than `MAX_PICK_SPAN` (8) across and down, so an interface that lies about the span can only move the pointer to a pixel within that block of what it already could name. The keyboard's reach through a picture's targets (PAR-30) is held the same way: the trusted core works out each target's rectangle from the pick record it holds and sends the interface only those rectangles and which are links (`ToInterface::PictureTargets`, at most `MAX_PICK_ORIGINS`, each a rectangle of at least a pixel inside the largest picture); the interface's focus (`ToCore::PictureFocus`) is taken only for a number it was sent, and Enter (`ToCore::PictureFollow`) follows only the link the focus is on, by the same checks as a click (`Links::click_drawing`: the destination shown in the strip, the dwell time, the question before leaving). Enter or Space on a target that is not a link is a press and a release at the centre of its rectangle sent as a pointer's would be, the picture pixel first and then the main button, so it can do nothing a pointer at that place could not.
- *Threat 11, static channel files with oversized pictures.* A fetched sub-page file's picture records (PAR-36 item 1) are read by a record reader of their own, kept from one message to the next and apart from the live connection's, with the same bounded readers and stores as a live channel's; a page the channel pushes is never replaced by an older file. Since the engine's DEC-6810 a page file with a picture may reach 2,224,090 bytes, far past one message to the trusted core, so the fetcher takes a page body up to that, the engine's largest pictured page file (`MAX_PICTURED_PAGE_FILE`, derived from the engine's constants and asserted to equal the stated figure), in a body buffer of that size, and passes a body larger than one message only when it is a page's packet file whose teletext packets fit one: those first, then its record packets in pieces of whole packets of one message each (the browser's DEC-6817). Anything else past one message is dropped with a notice. No message bound is raised, so finding F-74's fix stands: no message the fetcher writes passes `protocol::MAX_PAYLOAD`, and the trusted core and the decoder allocate no more for one than before.
- *Picture sources kept for back and forward (PAR-34).* The decoder keeps each picture sub-page's records, keyed on page and sub-code together, within `MAX_KEPT_PICTURE_SOURCE_BYTES` (33,554,432) a tab and the page cache's lifetime; past the budget the source drawn longest ago is dropped, and one larger than the budget is not kept. It is page content the channel sent the viewer; no local state, such as animation, is kept.
- *Reduced motion (PAR-33).* The interface tells the trusted core the platform's preference, which tells the decoders; the interface's word is taken as it stands, since at worst it stops or starts motion only the receiver makes.
- *Threat 7, user fonts for pictures (PAR-35).* A font file is chosen only in the file dialog (`regis-font` or `naplps-font`), and the broker reads only the file last chosen for that kind (`Request::ReadFont`), as it reads a picture to convert: canonical path, no link followed, nothing in a refused place, and its length checked against its language's bound before any of it is read (`protocol::regis::MAX_REGIS_FONT_BYTES`, 1,048,576, and `MAX_NAPLPS_FONT_BYTES`, 262,144, which the decoder asserts equal `regis::limits::MAX_FONT_BYTES` and `naplps::limits::MAX_STROKE_FONT_BYTES`). Neither the broker nor the trusted core parses it: a decoder started for it alone (`--font`) parses it with the renderer's own bounded parser, and only a font that parsed is kept, for the run, and given to every channel's decoder in parts of at most a chunk (`ToDecoder::PictureFont`, whose reader refuses a font over its bound and a part past its end). A font refused leaves the one in force; the parser's message reaches the panel cleaned. The provided sets are not replaceable (DEC-6669).

### 5.4 Boundary 4: fonts

Removed by decision 66: no font is fetched from a channel, so there is no font file to read and no font helper. Pages are drawn in the engine's own fonts, which ship with the browser.

### 5.5 Boundary 5: the trusted core into the interface process

**Threats.** Teletext data reaching the interface's readers; deceptive characters in page text; secrets leaking into the interface.

**Mitigations.**
- Pages cross only as page images our encoder produced from checked palette indices (`protocol::frame`, decision 61). The interface's reader accepts only that shape: at most 32 colours, the screen colour and every dot within them, the dots filling 480 by 500 exactly, changes only against the frame it holds (requirement 9.3). An attacker controls dot values within a fixed raster and palette, never file structure.
- Text crosses only cleaned and bounded, and the interface refuses text that still holds a hidden character (requirements 4.8 and 11.5).
- Text is only ever drawn as text runs: there is no markup to inject into (requirement 10.2).
- No viewer key, credential, setting or history beyond the strip's own state is ever sent (requirement 3.6).
- No sound crosses: the interface is told only which channels are sounding, for the tabs' speakers (decision 55; section 5.7).
- The Dock menu on macOS (decision 39) is the interface's one crate of its own with `unsafe` code, beside `confine`, which confines it at start (requirement 10.12): Objective-C runtime calls in the interface process that add a method to winit's application delegate and build the menu from tab titles the trusted core cleaned. Its input is our own tab list; its output is a tab index or mute-all, sent to the trusted core as ordinary intents.

**Remaining risk:** low. The image reader is our own, about a hundred lines with no third-party decoder, reading only our own encoder's output, and fuzzed (`page_image`, requirement 4.6).

### 5.6 Boundary 6: intents and strip input from the interface process

**Threats.** A compromised interface process sends messages to navigate, confirm a channel change, change security settings, read secrets, influence fetching, or exhaust the trusted core.

**Mitigations.**
- **Page intents carry no address.** They can report visible pages, click a cell, select, close, open or move tabs and windows, and send input to the open page's channel when it takes input (requirement 4.5, decision 48).
- **Visible pages affect only rendering,** never fetching: the fetcher takes from the trusted core only input messages for the channel's own push WebSocket, with whether more of their moment follow and a move's contact, the switch of that socket's round-trip ping (decision 125), and the answers to its channel's viewer-state challenges, none of which names an address, page or connection, so nothing the interface does can cause, delay or reorder a request (requirement 4.7). This closes the covert channel between channels that the earlier design found (`prototypes/docs/security-analysis.md` section 5.6) for every channel fetched from a contents file or a bundle, and for files.
- **Sub-pages are fetched only for the page open** (decision 121, requirement 4.7 as amended): while a page of the active tab's channel is open and is one of a set, the trusted core asks that channel's fetcher for the page's catalogue once each time it opens, and for the sub-page shown and the one either side of it, each once while the page is open; it names the page and a sub-code it checked, never an address, and nothing reaches another channel's fetcher. A compromised interface that opens pages and chooses sub-pages so decides some of the active channel's requests, and could signal through them to that channel's server; it can already do so more widely by navigating the tab, which it may, and the agent spaces the requests at its own rate. The sub-page list's rows are answered from what the trusted core or the decoder holds, never fetched.
- **Visible pages of another channel are filtered out** by the trusted core (requirement 4.4).
- Every message is bounded and checked; a malformed one ends the application (requirements 4.2 and 4.3).

**What a compromised interface process can do.** The trusted strip is drawn by the interface process, so its integrity rests on that process (decision 33; requirement 3.9):
- **Can:** draw a false strip; send strip input, and so navigate any tab to any address the trusted core's rules accept, go back and forward, reload (decision 63: the active tab's channel opened again at the same address, which the trusted core holds; the message carries nothing), and set bookmarks; change any setting the settings panel offers, the proxy and the local addresses allowed among them; follow any link on a page it shows; confirm the web-browser prompt; install a version the trusted core offers, which runs only if both keys signed it (section 8); type into the open page's channel when it takes input (decision 48); see every page on screen; open the history panel and the bookmarks manager and page through them, reading every row 100 at a time, and remove history or bookmarks by the rows' numbers (decision 57; `history-and-bookmarks-security.md` section 4, its first risk).
- **Cannot:** read viewer keys, history beyond the panels' views, or the user's files (its sandbox denies them on every platform: on Windows its AppContainer since decision 90, verified in the virtual machine on 24 October 2026); reach the network; change what any fetcher requests. That it can confirm the web-browser prompt and change a security setting is the cost of option 1 that open decision 3 weighed (decision 41 kept the strip in the interface process); each change is checked by the trusted core's rules and said in the strip (`settings-home-console-proxy-security.md`).
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
- A file channel is read by the browser's own channel rules and bounds (64 MB, 4,096 files), and only when the engine's `detect` recognises it as teletext in a format the decoder is built with, or an import's bookmarks file by its first line; never beneath an entry of the home folder beginning with a dot, the platform's folders of applications' data, or the browser's own folders, the only ones the confined core writes, so no link or hard link it makes there is read; and never through a link, the file being opened without following one and checked by its descriptor. Every refusal is one sentence, so none tells whether a file exists (decision 88). Since 31 October 2026 the broker reads a file for the core only when it is granted, named to the broker itself, or declared by the interface on a channel the core never sees (REQ-1698, `sandboxing.md` section 7.6), which closes this boundary against a compromised core alone; the residual is a compromised core and interface together. A grant names its file by device, inode and when the file was made, so a file made where a granted one was removed is never read through the grant, even when the file system gives it the same inode number (section 15, finding 7).
- A web address is checked by `web-opener`, as for a link (requirement 17.9), and the broker opens one at most once every two seconds, the core's own interval between hand-offs and one constant for both (decision 88, finding F-61). Its address is not checked further: any data fits in a valid address, so a compromised core can still send a little through the system's web browser, as fast as a link could; the fetcher's start arguments are not checked, for the same reason.
- A picture is read for the picture converter only when it is the one the user last chose in the picture dialog, by the same rules (refused places, no links, at most 64 MB), and only when its first bytes are a picture's; that choice grants nothing else. A converted page is written only to the place the system's save dialog last returned, of its kind, once, never through a link, at most 256 KB; the core never names the path (`picture-tools-security.md` section 4.4).
- Every request is a kind, a length and a payload read within its bound; at most 512 helpers alive and 64 arguments of 8,192 bytes; a request the broker cannot parse ends the core. The broker's request and reply readers are fuzzed (`broker_requests`, `broker_replies`). The broker reads nothing else from outside.

**Remaining risk:** low. The broker is small and unconfined: a fault in its parser would give a compromised core the user's rights. It parses only a handful of fixed messages.

### 5.9 A channel's program moving a tab (the REDIRECT record)

Reviewed on 3 November 2026 at browser `aed1829` (TODO-1691), for REQ-1688 and DEC-1683: a live channel's program may move the tab showing it to another of its pages, or to another channel, with no act of the viewer's, by a REDIRECT record on the push connection (DOC-1960). Section 5.1's redirects are a server's answers to a request; this one is a channel's own decision, carried in its data, and acted on by the trusted core.

**Threats.** A hostile channel sends the viewer somewhere they did not choose: to a lookalike channel, to this computer or the local network, to a `file:` address or a built-in page, or in a loop that never lets the tab rest; it uses the moves to fill the tab's history; it sends a page or query no address could hold; a compromised fetcher invents the record for a channel that never sent it.

**Mitigations** (`native/core/src/trusted_core/redirects.rs`, `native/core/src/session/navigation.rs`).
- **The record is the fetcher's word, read as hostile text.** The fetcher reads it with the engine's bounded, fuzzed record reader and passes it on as `FromFetcher::PageRedirect` (a page and sub-code, checked by `redirect_in_bounds`, and a query) or `FromFetcher::ProgramRedirect` (an address of at most `MAX_ADDRESS` bytes); the fetcher protocol's reader is fuzzed (`fetcher_messages`).
- **Only a live channel's tab, not already leaving, can be moved** (`may_redirect`): a static channel has no program, and a second record while the first's notice shows is ignored.
- **A page of the channel** is followed only when the channel lists it, and with a query only when a SELECT could carry that query for that page (`is_page_parameters`); otherwise the record is dropped and logged, and the server is told again what the tab selects. The page opens as the tab's selection, with no notice, as a link in the channel would.
- **Another channel** goes by the rules of a server's redirect (`redirect_destination`, REQ-701 item 2): `https` or `http` written out, read by the URL map's address rules, never a file, a built-in page or a host with its scheme left off. One refused is dropped and logged and the tab stays where it is. The engine's fetcher then refuses this computer and the local network as for any channel (requirement 5.10).
- **The viewer sees where the tab is going.** For one second (`REDIRECT_NOTICE`) the page area shows "Redirecting to:" and the destination, in words and a style the channel does not choose (REQ-1686; section 7.1), and only then does the tab follow; a tab behind others runs its own second.
- **Loops end.** The tab keeps the chain of channels it was redirected through; past `MAX_REDIRECTS`, 20, with no input from the viewer the redirect is refused (REQ-1688 item 6). The viewer's own input on the tab, or a page the viewer chooses, starts the count again. A program's loop is not refused at its first repeat, as a server's is, since a program may rightly send a viewer back; the bound of 20 at one second each ends it within about twenty seconds.
- **History.** A program's redirect adds a history entry by DEC-1684's rules (`Tab::redirected`), marking the page left as passed over when the viewer did nothing there, so Back does not return into the redirect. Entries are held to `MAX_TAB_ENTRIES`, 50, the oldest passed-over entry going first.
- **The destination is a channel like any other**, with its own fetcher, decoder and viewer key (REQ-714, REQ-716), and nothing of the channel that sent the viewer.

**If the sending channel is hostile.** It **can** move its own tab to any channel the address rules accept, as a web page can, up to 20 times in a row, each shown for a second; it **can** add history entries, each bounded as above. It **cannot** move another tab, open a file, a built-in page or this computer's addresses, carry anything of its own into the destination, or move the tab without the second's notice when it leaves the channel.

**If the fetcher is compromised.** It can invent the record for its own channel, which is what that channel's server could send, and no more: the trusted core applies every check above to it.

**If the interface is compromised** (section 5.6). An `Opened` intent for a page of the tab's own channel adds a history entry and starts the redirect count again (`Session::page_chosen`). That is within what a compromised interface can already do by navigating the tab, and the entries are bounded by `MAX_TAB_ENTRIES`; it cannot reach another channel's tab or name an address through it.

**Remaining risk:** low; ranked 8 in `hyperlinks-security.md`'s table of risks. What is not verified: no channel deployed on 3 November 2026 sends a redirect naming another channel, so the notice, the move and the loop's bound have been seen only in the tests of `native/core/src/trusted_core/redirects/tests.rs`, not live (TODO-1691, TODO-2064).

### 5.10 A page's rule table (push protocol 10, the second form)

A page may carry a rule table: a bounded, declarative list that binds the viewer's input, timers, cues and conditions to groups of protocol commands, each event handled at once and only time run on a tick of sixty a second (DEC-6587, DEC-6599, DEC-6635) (neoteletext-common's `docs/page-rules.md`, DEC-6522; requirement 1.3 as RULE-82 amends it). It is the one way a page's channel is sent the viewer's input (REQ-6523). This is the analysis of that document's section 12, for the browser as built (`architecture.md` section 6.4.1).

**What is protected.** The viewer's machine (processor, memory, sound output, screen); the viewer's input, on this page and on every other; the viewer's settings and other tabs (volume, mute, which other channels are open and what they play); and the separation of channels.

**Who attacks.** A hostile channel that sends any bytes as a table, as sounds and as pages; one that has also taken over its own decoder or the sound helper through some other fault; someone on the network of an unencrypted connection, who can do what a hostile channel can.

| # | Threat | Answer, as built | Remains |
|---|---|---|---|
| 1 | A table that is a program: loops, recursion, unbounded work | A rule runs one group of protocol commands and nothing else (DEC-6587): a group is at most 32 commands, nested at most 4 deep, a run at most 128 commands; no command makes a rule fire while it is being applied, and an event's firings are all decided before its commands run (RULE-1, RULE-6, RULE-118, RULE-120, DEC-6635). There is no cap on the work of a tick by the owner's decision (DEC-6587 item 2, "not work cap"): what bounds it is the table's own limits, the per-tick budgets of events, commands and reports for the events of a tick (MAX_EVENTS_PER_TICK 16, MAX_EVENT_COMMANDS_PER_TICK 2,048, MAX_EVENT_REPORTS_PER_TICK 32), and at most six ticks a wake (RULE-4) | A hostile table can keep its own confined decoder's core busy while its page is open (page-rules.md section 12.4). Ticks that run late slow page time and are reported as fault 9; the developer console's "Page cost" section shows the page's work, the sound helper's time for it, and the decoder's own processor time, which the broker reads once a second from the operating system's account of the process it started (on Linux `/proc/<pid>/schedstat`), never from the decoder; a tab whose decoder has used more than half a core over ten seconds shows a mark the interface draws outside the page's picture, which no page can draw or hide (REQ-6626) |
| 2 | A table built to exhaust memory or to crash the decoder | The table is decoded and evaluated by `neoteletext-rules` alone, which has no allocation, no dependency and no panic on outside input; a table is accepted whole or refused whole and reported as fault 1 (RULE-56, RULE-70, RULE-71). The decoder keeps the bodies of at most 64 pages' tables, each at most 8,184 bytes, and one evaluator | A fault in the crate would be confined to that channel's decoder, which holds only that channel's pages |
| 3 | A page that takes input the viewer did not mean for it | Only the open page of the active tab is given input, and only the classes its table asks for; the trusted core names that page to the decoder and takes a table only for it (RULE-58). The browser's own keys, a link's press, the strip and the wheel outside a `wheel` rule's rectangle are never given (RULE-51). The insecure badge is unchanged | A page the viewer has open and is typing at can ask for every key typed at it, as every page could before; it can no longer have them without a table saying so |
| 4 | Input leaving for somewhere else | The trusted core builds each `RULE_EVENT` from its own copy of the event, on the session of that page's own channel (RULE-52 items 3 and 4). The protocol crate refuses to write a viewer's input for a fetcher outside a `RULE_EVENT` | Nothing |
| 5 | A decoder taken over, asking the core to send what the viewer never did | A report carries no event bytes: it names a sequence number, and one the decoder was never given, one of another page or another sub-code of it (RULE-112), one given more than 4 ticks ago, or one already reported for that rule and row is refused and counted (`typing::tests::gate`, the fuzz target `core_report_gate`) | It can cause real input on its own page to reach its own channel, which the channel's table could have asked for |
| 6 | A covert channel out through values: a `send` of a value, or the state answer, carrying data chosen by a compromised decoder | 20 reports a second with no input event, of 120 in all; one state answer a query, only for a query the fetcher received, beginning with that query's identifier (RULE-52 items 5 and 6) | A compromised decoder holds nothing its channel does not already hold (its own pages and its own page's input); the budget bounds the rate |
| 7 | Learning the viewer's settings or other tabs through metered values | The level is the voice's peak before the master volume and the mute, of that view's own mixer; the helper reports a view's values tagged with that view's channel, and the trusted core gives them only to that view's decoder (RULE-32, RULE-33) | Nothing in the design. See 8 |
| 8 | A sound helper taken over through an instrument (a hostile bank image, setup block or MIDI stream), reporting another view's sound as this view's levels, which a colluding table then sends home | The instruments and the mixer are `no_std`, never allocate and forbid `unsafe`; every family's bank and message readers are fuzzed in the engine (`local_midi`); the trusted core bounds every field of a local sound's operations and accepts a metered report only at the length of the watch list it passed for that view, at most 32 bytes a tick; the helper stays confined to the sound output alone | A fault in the instruments that gave control of the helper would let a page learn which other tabs sound, at up to 32 bytes a tick. The helper already held every tab's sound; what is new is a path back, and that it now reads bank images and MIDI from a channel. Recorded for the owner (DEC-6525 item 12) |
| 9 | Sound as a nuisance: a table that plays loud notes for ever | The viewer's volume and mute are outside a page's reach; a local sound ends when its page is closed or its connection ends, and is silent, its notes released, while its page is paused (RULE-60, RULE-111); at most 2 local sounds a view; at most 512 sound commands a tick from one view's decoder pass the trusted core, however they arose, the rest dropped and told as fault 10 (RULE-52 item 5) | A page can make sound while open, as it could with the audio profile |
| 10 | Processor exhaustion through local instruments | Two local sounds a view, only for the open page; a view whose local sounds overrun the buffers they fill for 8 buffers running has them stopped (RULE-62). Measured: at most 0.8 ms of a 10 ms buffer for eight held notes | A page can cost up to the overrun bound of the audio thread for 8 buffers before it is stopped; the audio thread renders every view's local sounds under one lock |
| 11 | Processor exhaustion through animation actions | A rule's animation actions write the same bounded state an `ANIMATE` record writes, at most 64 objects a sub-page of which 16 cycle (`docs/animation.md` ANIM-6, ANIM-14); a ReGIS repaint is held to its repaint budget (REQ-6368) and a NAPLPS repaint for animation to 153,600 pixels a tick (ANIM-16), a repaint held back being made later of the latest state and reported as fault 8; the state itself changes at once, so rules and input are not delayed. A teletext page's repaint and a ReGIS repaint have no pixel maximum of their own yet (TODO-6563 item 8) | A page can cost its repaint budget every tick while it is open; a teletext page with 32 sprites is drawn whole rows at a time |
| 12 | Timing: a table as a clock for measuring the machine | A page tick is all a table can read of time; ticks slow under load (RULE-4) | A channel can learn that the machine was late: about one bit a second |
| 13 | Fingerprinting the keyboard layout through physical and character keys | The KEY event already carries both | Unchanged |
| 14 | A table that survives the page | Everything local of a page is dropped when it is closed or its connection ends, and nothing is carried over or rebuilt (RULE-111): the decoder holds one table, of the selected page and sub-code; a tab that is not the active one is paused, its state kept but no tick run, no input given and no report made. For 4 ticks after a close the trusted core still passes that page's reports, so that a `close` rule's report goes; on a disconnect nothing more goes, and a `RULE_EVENT` not acknowledged is never sent again: a session that gave a table input is not resumed (RULE-46) | Up to 4 ticks of reports for a page just left |
| 15 | The default table (RULE-53) sending more than a page needs | It is the behaviour before push protocol 9 made explicit, chosen for each channel by its operator | Until a channel moves to tables of its own, its pages ask for everything, as they did |
| 16 | A page reaching another sub-page's state | A sub-page is a page (DEC-6599, RULE-112): its table, variables, sheet and animation state are its own, keyed on page number and sub-code in the decoder and the trusted core; another sub-code of the same number is another page, opened only by closing this one | Nothing |

**New in the sound helper.** Before this, the sound helper parsed nothing: it copied checked samples. It now holds instruments and gives them bank images, a setup block and MIDI messages that come from a channel. The trusted core bounds each field's length and range and reads none of their content; the instruments' own readers bound what they do with it, in code that cannot allocate or index out of range without a checked fallback. The helper's confinement is unchanged. That is why threat 8 is recorded as a residual risk and not as closed.

**Checks.** `core/src/typing/tests/gate.rs` (the hostile-decoder harness and REQ-6523's no input of any kind without a table), `core/src/trusted_core/rules/tests.rs` (the same through the whole core's pipes), `decoder/src/engine/rules/tests.rs`, `sound/src/audio/local_tests.rs`, the protocol crate's bounds tests of every new message, and the fuzz targets `core_report_gate` and `protocol_audio_local`. Reviewed against this section on 18 November 2026, and again that day after the rule table was given the animation's picture, the receiver's tracked values and the setting "Paint drawings gradually" (a message to a decoder of one flag, which a decoder cannot send; it changes only how fast that decoder paints).

**Remaining risk:** low for the gate, whose property does not depend on the decoder; medium-low for the sound helper's new reading of channel data (threat 8), which rests on the instruments' code.

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

**The threat.** A teletext page can draw anything at teletext resolution: a fake address bar, a fake web-browser prompt, a fake button, a fake system dialog.

**Mitigations.**
- The address, identity, encryption state and the web-browser prompt are shown only in the trusted strip, which is drawn outside the page area and which no page image is ever drawn over (requirements 11.1 and 11.2).
- **Every page image is kept below the strip,** including the zoomed page and the overview's popup of a page at its native size, which shrinks on a short window rather than cover the strip (observed in this codebase's tests).
- **A page's screen colour stops at the strip.** With a page alone the colour is the background of the window below the strip (REQ-740), so a channel chooses one of 4,096 colours for that area: never a word, a shape or a pixel of the strip, which is drawn after the page area, over anything that reached it, and the same on every page (`the_strip_never_takes_the_colour`, `a_pages_colour_reaches_every_pixel_below_the_strip_and_no_further`). The panels on the colour draw the channel's magazine names and page titles in the browser's ink, fitted to their places as on black.
- A click on a page means only "this cell" (requirement 12.3).
- The strip stays visible in every layout but the page-only full screen, which ends before anything leaves the channel and brings the strip back at the top edge (requirement 11.4, `full-screen-security.md`).
- Every new layout is reviewed for spoofing (requirement 14.5); the review of every layout, 3 November 2026, follows.

**Remaining risk:** medium. This is where safety depends on discipline in visual design: an interface built for animation is exactly where a security indicator could be hidden or imitated.

### 7.1 The review of every layout, 3 November 2026

Read at browser commit `c69b96a` and its two fixes `9ddd23a` and `158a6f2` (REQ-785, TODO-2086, DEC-2387), in `native/interface-core/src/browser` and `wall`, with the desktop back end `native/draw-vello`, at a wide window (1440 by 900 points) and at the smallest the interface allows (1000 by 640, `MINIMUM_WIDTH` and `MINIMUM_HEIGHT` in `native/interface/src/window.rs`). Two questions for each layout: does the trusted strip stay visible, with nothing drawn over it; and can anything a channel controls look like it? What a channel controls: every pixel of a page image, the screen colour (REQ-740), and text the trusted core passes on cleaned (page titles, magazine names, sub-page titles, a link's address, a redirect's target, a host).

**The strip and the order of drawing.** The strip is the tab row (0 to 44 points), the toolbar with the address field and the padlock (44 to 98, black, ending in a one-point rule) and the bookmarks bar when shown (98 to 132). The page area starts 8 points below it (`BODY_TOP`, `Browser::strip_end`). `Browser::draw` (`browser/window.rs`) draws the page area first (the screen colour, the banks, the lists, the wall or the page, the overview and the console) and the strip after it, opaque, so nothing of the page area can show above the strip's end whatever its rectangle. Only what is drawn after the strip can cover it: the overview's native-size preview, the remote, the phone keyboard's panel, the settings, history and bookmarks panels, the star, the tab overflow list, popups, tips and the link popover, and a dragged tab. The desktop back end clips each area it is told to (`draw-vello/src/record.rs`). The interface core's test canvas records no clip, so the clipping below is tested in the back end, which clips (`what_moves_in_the_page_area_never_leaves_its_clip` in `draw-vello/src/tests.rs`), and the order of drawing by the test canvas, which keeps page images and fills in the order drawn.

**The wall, at every page size** (decision 35). The pages, the hover pop-out and the zoom are drawn inside a clip of the wall's area, which starts below the size controls, at least 48 points below the strip's end. The screen colour is off on the wall. Strip visible; nothing like it. Tested at 1000 by 640 with the bookmarks bar hidden and shown: part of the way through the zoom and the slide between zoomed pages, no pixel above the wall's area differs from the window at rest (`what_moves_in_the_page_area_never_leaves_its_clip`, which fails with the clip taken out, a sliding neighbour reaching the row the strip ends on). The zoom runs between two places inside the area and needs no clip.

**The page alone** (DEC-1018, REQ-772). The page is centred in the page area, drawn in a clip of it, on the channel's screen colour, which starts at the strip's end (REQ-740). Strip visible; the slide between pages alone is tested as the wall's is (`what_moves_in_the_page_area_never_leaves_its_clip`, which fails with the clip taken out). Look-alike: a channel can choose a colour near the strip's black or its rule and draw, in its top rows, something like a second toolbar about 8 to 10 points under the real one. That is the line of death every browser has; the strip above it holds the real address and padlock, a click on the page is only a cell (requirement 12.3), and nothing a page draws can be typed into. Residual, low.

**Full screen** (DEC-978, `full-screen-security.md`). The strip is gone by design while it is up; nothing leaves the channel without full screen ending first. When it comes down at the top edge it is drawn after the page, opaque, over it (`the_strip_in_full_screen_is_drawn_over_the_page` in `browser/full_screen/tests.rs`: the page's screen colour and image come before the strip's fills, which cover every row to the strip's end, at 1000 by 640 and the tests' window, with the bar hidden and shown; it fails with the strip drawn first). The leaving notice and the typing badge are drawn in the margin after the page; during a slide between pages a neighbouring page passes through that margin under them, never over them. Strip visible when down; a page drawing a false strip while it is up is `full-screen-security.md`'s first threat, answered there.

**The console docked at the foot** (decision 120). Drawn in the page area, its log clipped to its list; at the smallest window its top stays below the page area's. Nothing of it reaches the strip.

**The overview and its native-size preview.** The preview is drawn after the strip and not clipped, but its scale keeps its top at or below the page area's top, the bookmarks bar included. Strip visible; tested with the bar hidden and shown, the whole preview (the screen colour, the image, the outline and the shadow) in the tests' window, at 1000 by 640 and in a short window (`the_whole_preview_stays_below_the_strip_with_the_bar_shown_or_hidden`, which fails with the scale ignored, and alone fails with a scale that leaves out the bar; `a_page_in_the_overview_pops_up_at_native_size_under_a_hand`).

**The bookmarks bar shown or hidden.** The page area and the screen colour start below the bar when it shows (`the_strip_never_takes_the_colour`, both states). The bar's chips are bookmark names the user kept; its folder list stays within the page area.

**The sub-pages list and its divider.** The divider is the browser's colour and stays inside the list; the channel's sub-page titles are clipped to their rows (`the_sub_pages_stand_on_the_colour`).

**The whole-window screen colour** (REQ-740). Filled from the strip's end down, the strip drawn after it (`the_strip_never_takes_the_colour`, `the_pages_colour_is_behind_everything_below_the_strip`, `a_pages_colour_reaches_every_pixel_below_the_strip_and_no_further`). The last measures in pixels that the colour starts on exactly the row the strip ends on (`Browser::strip_end` at the drawing's scale), at 1000 by 640 and the tests' window, with the bar hidden and shown, at one and two device pixels to the point; it fails on a colour started at the page area's top, which before it passed. Look-alike: as for the page alone.

**A redirect's notice, a refused certificate, a denied selection** (REQ-1686, `certificate-errors.md`). Each is drawn in the page area with the screen colour off. The redirect's notice shows the channel's target in the address field's typeface, white on black under the words "Redirecting to:", not the field's dark ink on grey; the certificate's panel shows the host; the denied notice has fixed words. None reaches the strip. Residual, low.

**The link popover** (REQ-1660). **Fault, fixed.** It holds the address a channel chose and was drawn after the strip, clamped only to the window's top: at 1000 by 640 a long address put it over the address field and the padlock, and the longest over the whole strip. Since `9ddd23a` a tip of anything below the strip stays below the strip's end, and the rest scrolls with the wheel (`a_links_popover_never_covers_the_strip`, which failed on the code before). In full screen its box is drawn as the leaving notice is; it shows an address, the notice fixed words. Residual, low.

**The pointed link in the address field** (requirement 17.6, REQ-806). While the pointer or the focus is on a link the field shows where it leads, behind "→ " for a page in the channel and "↗ " for a link that leaves it. When reviewed it was drawn in the field's own ink with the current channel's padlock still beside it, so a viewer could take a link's address for the channel's own. **Fault, fixed** (DEC-2496, TODO-2398): the destination is now drawn in the link ink (`POINTED_INK`), which the real address, drawn only in `FIELD_INK` and `DIM`, never takes, as the theme asserts when it compiles; and while it shows, the channel's identity (the padlock, "Not secure", "Certificate refused", "NeoTeletext" or "File") is greyed in `DIM`, kept in place so a channel cannot hide its own warning by covering its page with links, but no longer vouching for the destination (`a_pointed_destination_is_never_drawn_as_the_channels_own_address`, which failed on the code before).

**Multiple windows and a dragged tab** (`multiple-windows.md`). Each window draws its own strip in the same way. A dragged tab is the browser's tab, titled with the host the trusted core gives, and may pass over the page area; the tab overflow list holds the browser's tabs. Nothing of a channel's reaches a strip.

**Zoom and animation.** Zooming in and out, the slide between pages and the hover pop-out all run inside the wall's or the page area's clip; no animated page passes the strip's end. The zoom and the slides are tested in the back end, which clips (`what_moves_in_the_page_area_never_leaves_its_clip`); the hover pop-out is held by the same clip, by reading.

**The panels.** Settings, history, bookmarks and the remote are placed within the page area (`widgets::panel_rect`, `remote/draw.rs`). History and bookmarks show titles and addresses a channel gave, in their rows, below the strip. **The phone keyboard's panel: fault, fixed.** It was centred on the page area at its full 560 points with no limit and reached above the strip at 1000 by 640; its content is the browser's, but it hid the strip and took its presses. Since `158a6f2` it stays below the strip, shorter on a short window with its code smaller by as much (`the_panel_never_covers_the_strip`, which failed on the code before).

**Tests added for what the review held by reading** (TODO-2399): the clip, in the back end (`what_moves_in_the_page_area_never_leaves_its_clip`); the screen colour's end measured against the strip's height in device pixels (`a_pages_colour_reaches_every_pixel_below_the_strip_and_no_further`); the strip drawn over the page in full screen (`the_strip_in_full_screen_is_drawn_over_the_page`); the whole overview preview with the bookmarks bar shown and hidden (`the_whole_preview_stays_below_the_strip_with_the_bar_shown_or_hidden`). Each was shown to fail with the property it checks broken on purpose. The smallest window is written in each test as 1000 by 640 with a comment naming `MINIMUM_WIDTH` and `MINIMUM_HEIGHT` in `native/interface/src/window.rs`. What stays by reading: the clips of the hover pop-out and the console's log, and the look-alikes above, which are a matter of design and not of drawing.

**For each new layout:** a commit that adds a layout, or anything drawn after the strip, names its answer to the two questions in this section.

---

## 8. The application update mechanism

Built 23 October 2026 (decisions 54 and 73 to 82): `installation-and-updates.md`, with its own analysis in `installation-and-updates-security.md`, whose section 4a reviews the build. The Update Framework with offline threshold keys and a second, platform signature by a key held apart; side-by-side installation, an atomic switch, rollback after two failed starts; a confined updater that only stages; updates suggested and installed only when the user accepts. Until the key ceremony the production identity accepts nothing (decision 76). Its remaining risks are key custody, a fault in the launcher, which is not confined, and on Linux the AppArmor profile of decision 77.

---

## 9. Supply chain

**Threat.** Malicious or vulnerable code in a dependency.

**Mitigations.**
- **No run-time JavaScript and no package manager for the interface.** The earlier design's medium-likelihood risk from React and its packages is gone.
- **Rust dependencies pinned to exact versions** in `native/Cargo.toml`, every one permissively licensed (decision 34); lockfile committed. `cargo deny check advisories` runs at every push and `cargo deny check` in the full gate, for both workspaces, `native` and `native/tuf`: licences, sources and the RustSec advisories, which `cargo audit` would read too and which DEC-2354 makes the check (requirement 14.3). Three advisories are ignored with their reason, crates merged into `zbus` 6 under two clients still on `zbus` 5 (TODO-2366).
- **Where a dependency sits limits what it can do.** The trusted core depends on six of the engine's crates and no other, each for a bounded use that DEC-2355 lists and `check_dependency_trees` holds: `neoteletext-core` (the standard's numbers and the console's packet addresses), `neoteletext-input`, `neoteletext-audio` and the `neoteletext-opus` it brings (the console's recognition of sound), `neoteletext-formats` (the broker's `detect`), and, of `neoteletext-web`, viewer state's channel identity and the bytes a key signs, its proxy-address parser, its address type and the channel URL map; and on the protocol, `confine`, `update`, `release-identity`, `web-opener` (for the broker), `ring`, `keyring` and `unicode-normalization` (requirement 3.2). The trusted core reads no header: a fetcher reports a viewer-state challenge as fixed fields, and the core builds the bytes it signs (REQ-879); there is no cookie jar since DEC-6471. `protocol` also takes `unicode-normalization`, for its table of combining marks (finding F-36). The drawing libraries sit in the interface process, the engine's readers in the decoder and fetcher.
- **The engine** is pinned to one commit in its own repository and never patched here (requirement 2.5).

**Remaining risk:** low for the trusted core. Low to medium for the interface process's drawing libraries, which are young (`vello_cpu` 0.3, Parley 0.12) and run in the process that draws the strip.

---

## 10. Local attackers

- **No network ports are opened** by the application. Messages travel over pipes between our own processes.
- **No debugging interface** in any build (requirement 10.8).
- **Under X11** any local process can send input to the window, as to every X11 application; the trusted core still applies its rules to everything it receives.
- **History, bookmarks, settings, the session and the sealed file of remembered viewer keys are kept in the settings folder,** readable by the user alone (decisions 57 and 59, REQ-882); the trusted core is confined to that folder (decision 56), not to an App Sandbox container.

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

- **Fetching never depends on viewing** (decision 22): a channel's server cannot see which of its pages a viewer reads (requirements 4.7 and 5.15). *Rescinded 25 October 2026 by decision 104:* pages are fetched while visible or focused, so a channel's server learns which of its pages are on screen. *Built 3 November 2026 (DEC-2356):* the trusted core asks a static channel's fetcher for the pages the active tab's address names and the pages the interface says it shows of that channel, each a page number the protocol bounds, each once until it arrives or is refused, at the engine's rate. A compromised interface can therefore choose which pages of the active tab's channel are fetched, and so signal to that channel's server through its requests; it could already do as much by navigating the tab (section 5.6). It can reach no other tab's channel: the trusted core drops a page report of any channel but the active one.
- **A channel served as files learns which sub-page is read** (decision 112 item 5, built by decision 121): the sub-page shown and the one either side of it are fetched from their own files when the page is open, as the page's own file is; a channel that publishes no catalogue learns nothing more.
- **There are no cookies** (REQ-889, DEC-6471 of 18 November 2026). **A viewer key belongs to one exact channel,** held by the trusted core and proven only on that channel's own push connection; there is no third-party context (REQ-716, REQ-717). A key is made only when the channel asks after the viewer acted; it is kept between runs only when the viewer chose to be remembered, and only sealed, never a private window's; "Channels that know you" forgets one channel's or all, stops remembering or blocks, and history's clearing takes keys with it (REQ-882, REQ-884, REQ-886).
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
| Linux | Built 22 October 2026 (`sandboxing.md`, decisions 52 and 58): each helper confines itself first thing in `main`, before any thread: no new privileges; for every role without the network an empty network namespace; Landlock 5, enforced whole, with only the directories the role may read (none for a decoder; the resolver's for a fetcher; fonts, preferences and the system's for the interface) and no TCP for the roles without the network; and a seccomp deny-list of system calls: no programs, no tracing, no mounting, no kernel interfaces a helper has no use for, and sockets only of the families the role needs (none for a decoder or the trusted core; the internet's and routing netlink for the fetcher, keyboard and updater; `AF_UNIX` for the interface and the sound helper, which since decision 86 have their own `/run`, `/tmp` and runtime folder holding only their display's or sound server's sockets, so no helper reaches the session bus; since DEC-6659 item 24, 19 November 2026, the interface also writes an empty `/dev/shm` of its own, a private `tmpfs` that no other process sees, for the image it shares with the X server: it reaches nothing it could not before, since the X server is handed the image's descriptor over the socket the interface already holds, and the memory it holds there is memory it could take as its own). The trusted core confines itself the same way, with its settings folder writable, before it reads its broker's channel (decision 56). Every layer is required (decision 58): a machine without Landlock 5, without unprivileged user namespaces (a default Ubuntu from 24.04 needs an AppArmor profile for the browser, which every Linux release carries as `neoteletext.apparmor` for an administrator to install, decision 77) or without a filter for its architecture does not run the browser, and says why. Measured on Ubuntu 26.04 (kernel 7.0): the crate's probe tests pass and the browser runs under Xvfb with every helper reporting its layers. |
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
3. **The engine's user agent fetches a channel in its contents file's order at its own fixed rate,** whatever is on screen: measured, 218 of 800 pages in 45 seconds. The fetcher takes no input from the trusted core, so the interface has no path to influence fetching. *Since decisions 44 and 62* the fetcher also takes input messages for its push WebSocket and, since 18 November 2026, viewer-state answers in place of cookie answers (DEC-6471); neither names an address, page or connection (requirement 4.7). *Since 3 November 2026 (DEC-997, DEC-2356)* it fetches a static channel's pages and listing only as the trusted core asks for them, by page number, never by address.
4. **The interface's protocol refuses every kind it may not receive,** and messages with hidden characters, oversized lengths or trailing bytes (tests in `protocol`).
6. **A sandbox profile can hide the window without an error.** On macOS 15.6 the interface's profile lacked the window manager's service, `com.apple.windowmanager.server`: the window was made and drawn, never shown, and nothing failed (23 October 2026, `sandboxing.md` section 6). A denial shows only in the system's log; a missing service is found by allowing all lookups once and reading the denials.
7. **A file grant named its file by device and inode alone, and ext4 gives those to the next file made.** Found on 18 November 2026 when the push gate ran with `TMPDIR` on the ext4 disk (TODO-6629): a recording granted, removed, and another file written at the same path took the freed inode number, and the broker read the new file through the old grant (REQ-1698, finding F-03). Anyone who can replace a granted file could have had a different file read in its place. Closed the same day: a grant's identity (`confine::FileIdentity`) also holds when the file was made, its birth time where the file system records one and its inode's change time where it records none, so a file made at a freed inode is another file; grants written before are dropped when read, and asked for again. The broker's test now makes its files on the build's disk, not under `TMPDIR`, and `confine`'s own test makes a file at a freed inode and holds that it is another; both fail on the code before.

---

## 16. Remaining risks, ranked

| Rank | Risk | Likelihood | Impact | Main controls |
|---|---|---|---|---|
| 1 | Spoofing of the trusted strip by page images, especially in animated layouts and popups | Medium | The user is misled about where they are | Strip outside the page area; page images kept below it; recorded layout reviews (requirements 11.1 to 11.4, 14.5) |
| 2 | A fault in the interface process, which draws the strip | Low | A false strip and forged navigation; the history and bookmarks the panels show, read through them and carried out by a navigation (decision 57) | Our own code, no `unsafe`, inputs limited to our own images and cleaned text; views of at most 100 rows asked for by number; fuzzing and the hostile harness (requirements 3.9, 4.6, 10.12, 20.5); open decision 3 |
| 3 | **As built:** the broker and the launcher are not confined | Low (needs a fault in the trusted core and then in the broker's parser, or in the interface) | The user's rights | Every other process confined on macOS, Windows and Linux (decisions 52, 55, 56); the broker parses a handful of bounded, fuzzed messages and decides every role itself (section 5.8) |
| 4 | A fault in the trusted core, the process holding the browser's state | Low | The settings and every channel's content, and a false strip; not the user's files or the network, which its sandbox denies (decision 56). What remains through the broker: only files the user gave, each by a grant, a command line or the interface's declaration (REQ-1698) | Small, no format readers, bounded protocol, no `unsafe` (requirements 3.2, 4.2, 14.2); fuzzing (requirement 4.6); confined behind the broker, which reads only what it was given, and of that only teletext outside the dot-folders, applications' data and the browser's own folders, never through a link, and opens a web page at most once every two seconds |
| 5 | A plain `http` channel | Depends on the channel | That channel readable and alterable on the network | Transport Layer Security with `rustls` and Mozilla's roots for `https` channels (decision 41); a plain channel shown as "Not secure" |
| 6 | Theft of the update root keys at threshold | Low | Malicious update | Threshold, offline hardware keys, separate holders (requirements 13.8, 13.9) |
| 7 | A supply chain compromise of a Rust dependency | Low | Depends on the process it sits in | Pinned versions, permissive licences, review; `cargo-deny`'s advisory check at every push (requirement 14.3, DEC-2354) |
| 8 | A decoder or fetcher fault, with the sandboxes complete | Low | Contained to one channel | Separate processes per channel, no rights (requirements 3.3, 3.4, 3.12) |
| 9 | Exhaustion of resources | Low | An unresponsive tab or application | Visibility culling, bounds, on-demand drawing (section 11) |

**Smaller residuals of the review of 24 October 2026, settled on 3 November 2026 (TODO-1278, DEC-2415, awaiting the owner's confirmation).** Closed: a junction swapped in for a folder of a file channel's path on Windows is refused by the path the open handle was resolved by (`a_junction_inside_the_path_refuses_the_open`, to be run in the Windows virtual machine); a Linux helper whose broker ended before it first read its parent refuses to run, by the parent's number given at its start (finding F-34, `a_parent_that_ended_before_the_first_look_is_seen`); a text field that arrives with a run of combining marks longer than `MAX_COMBINING_RUN` has the run cut, not the pipe refused (finding F-36, `a_run_of_combining_marks_that_arrives_is_cut`). Accepted, each with its reason in DEC-2415:
- **F-26:** the phone keyboard closed in the instant between its screen being made and its waker stored exits without closing the screen, whose relay instance then lapses by the relay's own expiry; no phone can have joined it. Closing it needs the engine's qrkeyboard library to take a waker before its screen exists.
- **F-54:** `arboard` reads the clipboard whole before the paste is cut; the clipboard is the user's own or a local program's, and a bounded read needs another library.
- **F-09:** on macOS a trial that never says `started` ends that start with an error, and is rolled back after three such starts (`NOT_STARTED_TO_ROLL_BACK`).
- **F-80:** after a lost state on macOS the installed bundle, from which the launcher itself runs and which the system checks at each start, is not accepted again; a replaced bundle would carry a replaced launcher.

---

## 17. Checks that keep these properties

Each check below belongs in the project's check script, run by git hooks (requirement 14.1). **Built** marks those that exist.

1. **Strict compilation:** `unsafe` forbidden, Clippy `pedantic`, warnings as errors (requirement 14.2). **Built.**
2. **Message bounds:** oversized, malformed and wrongly-kinded messages refused (requirement 4.2). **Built** in part, as unit tests.
3. **Text cleaning:** hidden characters removed and refused (requirements 4.8 and 11.5). **Built.**
4. **Frame checks:** wrong sizes, palettes and indices refused (requirement 9.2). **Built.**
5. **Image reader:** anything but our encoder's shape refused (requirement 9.3). **Built.**
6. **Trusted core dependencies:** no web engine, no script engine or interpreter, and of the engine's crates only the six DEC-2355 names, each for a bounded use (requirements 1.3 and 3.2). **Built:** `check_dependency_trees` in the push gate.
7. **Engine dependencies:** no interface toolkit; one pinned commit, through every manifest, path and lockfile entry; the engine's own full gate at that commit (requirements 2.2, 2.3 and 2.5). **Built:** `check_dependency_trees` (push gate), `check_engine_pin` (commit gate), `check_engine_gate` (full gate, DEC-2360).
8. **Process rights:** network and file access fail where denied, on each platform (requirements 3.3 to 3.7). **Built** in part: on Linux `confine/tests/probe.rs` runs every role confined and checks what each is refused: files, programs, the session's sockets, the x32 numbers, the terminal and other processes' signals; on macOS the profiles' contents are tested (`confine/src/macos.rs`), and a confined fetcher's reach of this computer (`confine/src/spawn/tests.rs`); on Windows a confined interface's network and file reads (`confine/src/spawn/tests.rs`, run in the virtual machine on 24 October 2026), and the containers' making and environment.
9. **Fuzzing** of every reader in the trusted core and the interface (requirement 4.6). **Built:** `native/fuzz`, one target for each message reader, the decoder's engine, the interface's image reader and the address parser; the settings, history, bookmarks, session, remembered viewer keys and Netscape bookmark files; input records, phone keys and relay messages; the launcher's state, the release manifest, the gzip reader, the version mark and the updater's messages; and in `native/tuf/fuzz` The Update Framework's metadata and the updater's HTTP responses.
10. **The hostile interface harness** (requirements 3.9, 4.5 and 4.7). **Built** in part: `native/core/src/trusted_core/hostile_tests.rs` against the trusted core, on macOS and Linux with a stand-in broker whose requests and helpers' input it checks (finding F-98: no helper, file or hand-off from a page intent, helpers only of the roles, every message a helper's reader accepts, no channel's file given to another's decoder); and with the real decoder and fetcher, confined as the broker confines them, against two reference channels that log every request (`hostile_tests/real_helpers.rs`, TODO-1254): with every intent but strip input sent while the first channel is in front, the channel behind hears exactly a quiet run's requests (requirements 4.7 and 7.3), the channel in front only those and SELECTs of its own pages with no parameters (requirement 4.7's fourth exception), no other address is connected to, no file is read and no address handed on through the broker, and every helper says it is confined. Not on Windows, where `confine` has no stand-in broker.
11. **Channel isolation:** viewer keys and pages never cross channels (requirements 7.3 and 7.8, REQ-716).
12. **Dependencies:** `cargo-deny` and lockfile review (requirement 14.3). **Built:** `cargo deny check advisories` in both workspaces at every push, and `cargo deny check` whole in the full gate; `cargo deny`'s advisory check is the RustSec check in place of `cargo audit` (DEC-2354).
13. **Updates:** the full set of update tests (requirement 13.12).
14. **Spoofing:** recorded review of every layout (requirement 14.5). **Built:** section 7.1, 3 November 2026, with `a_links_popover_never_covers_the_strip` and `the_panel_never_covers_the_strip`; the tests it found missing are TODO-2399.
