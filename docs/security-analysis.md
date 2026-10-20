# NeoTeletext: security analysis

| | |
|---|---|
| **Status** | Analysis of the native architecture (decision 33), 16 October 2026. Where a point was observed on this codebase rather than reasoned, it says so. Where the design and what is built differ, both are given. |
| **Companion documents** | [`architecture.md`](architecture.md) describes the design. [`requirements.md`](requirements.md) states each requirement this analysis relies on, numbered, with whether it is met; this document cites them as "Requirement N.N". The earlier design's analysis, with its web engine, is in [`../prototypes/docs/security-analysis.md`](../prototypes/docs/security-analysis.md). |

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

- **Reading untrusted bytes only in processes with no rights:** the fetcher (one per web channel, network only) and the decoder (one per channel, no network).
- **Keeping a trusted core that contains no web engine,** reads no network or teletext format, draws nothing, and owns every secret and every decision.
- **Having no web engine at all.** The interface is NeoTeletext's own Rust code. It runs no script, parses no markup, and reads only images our own encoder made and text the trusted core cleaned. The whole class of web-engine risk that the earlier design spent decisions 27 to 32 containing is gone.
- **Keeping the address and every security decision in the trusted strip,** outside the page area, where no page image is ever drawn.

**What decision 33 accepted.** The trusted strip is drawn by the interface process. A fault in the interface process could therefore draw a false strip and send strip input (section 5.6). In the earlier design that needed a fault in a web engine plus an escape from its sandbox; here it needs a fault in our own interface code or its drawing libraries, reached only through our own images and cleaned text. Open decision 3 asks whether to move the strip into a smaller process.

**The main remaining risks,** detailed in section 16:
1. A fault in the trusted core, which is why it is kept small.
2. Spoofing of the trusted strip by page images, in an interface built for animation.
3. A fault in the interface process, which draws the strip.
4. Sandboxes that are not yet complete: on macOS they still allow reading files, and elsewhere there are none.
5. The update signing keys, once updates exist.

---

## 2. What is protected

| Asset | Where it lives | Why it matters |
|---|---|---|
| Which channel each tab shows, and the address the user sees | Trusted core; shown in the trusted strip | The user's decisions rest on it |
| Cookies, and anything a channel's login sets | Today: each fetcher, for its own channel. Required: the trusted core (requirement 7.5) | Access to the user's accounts on teletext services |
| History, bookmarks, settings, sessions | Trusted core (in memory today) | Private; settings control security |
| The user's own files | The file system; opened by the trusted core when the user names one, and decoded by a decoder | Private |
| The integrity of the application's code | Installed versions; in time the launcher and update keys | Everything else depends on it |
| Availability | Every process | The application must stay usable |

---

## 3. Who attacks, and how

| Adversary | Capabilities |
|---|---|
| **A hostile teletext service** | Sends any bytes in any role: malformed or enormous pages, crafted enhancement data, misleading page text and titles, a hostile contents file, a push feed that never stops, cookies |
| **A hostile recording or page file** | The same, through a file the user opens |
| **A network attacker** | Reads and alters unencrypted traffic, which is allowed (decision 10) and which is today all traffic, because Transport Layer Security is not built |
| **A compromised update server, a thief of a signing key** | Once updates exist (section 8) |
| **A compromised dependency** | Code inside a crate built into a process |
| **A local process running as the user** | Reads the user's files; sends input to windows under X11 |
| **A compromised component** | Any one of our own processes, assumed compromised to test that the others hold |

Out of scope: an attacker who already controls the user's account or has administrator rights, and physical access to the machine.

---

## 4. Trust boundaries

```
network ──[1]──► fetcher (one per web channel) ──[2]──► trusted core ──[3]──► decoder (one per channel)
files the user opens ──────────────────────────────────► trusted core ──[3]──► decoder
trusted core ──[5]──► interface process (images, cleaned text, state)
interface process ──[6]──► trusted core (intents, and strip input)
update subdomain ──[8]──► updater ──► versions directory ──► launcher        (not built)
```

| Boundary | What crosses | The receiving side treats it as |
|---|---|---|
| 1. Network to fetcher | Raw protocol bytes | Untrusted |
| 2. Fetcher to trusted core | The contents file as typed entries; page bytes, forwarded unread; notices | Untrusted, checked as data |
| 3. Trusted core and decoder | Bytes, shown pages and ticks in; arrivals, updates and frames out | Decoder output untrusted |
| 5. Trusted core to interface | Images our encoder made, cleaned text, tab, strip and channel state | The interface's input; contains nothing secret |
| 6. Interface to trusted core | Intents and strip input | Untrusted intents; strip input trusted only as far as the interface process is (section 5.6) |
| 8. Update subdomain to updater | Metadata and release files | Untrusted until verified (not built) |

Boundary 4 of the earlier design, the font helper, returns when fonts are fetched (requirement 3.5). Boundary 7, the strip in a separate host, is folded into boundary 6.

---

## 5. Analysis of each boundary

### 5.1 Boundary 1: the network into the fetcher

**Threats.** Faults in the protocol readers (the HyperText Transfer Protocol, Server-Sent Events, the descriptor and contents readers); decompression bombs; slow or endless responses; redirect loops; requests aimed at the local network.

**Mitigations.**
- The engine's strict, bounded readers, through its user agent; the fetcher only moves bytes between the agent and its sockets (requirement 2.1).
- The engine's limits on size, time and redirects, and on decompressed size (requirements 5.11 and 5.12).
- The local network refused unless the user names an address (requirement 5.10).
- One fetcher per channel (requirement 3.12).

**If a fetcher is compromised.**
- **Has:** the network; its own channel's connections and cookies.
- **Does not have:** other channels' cookies, which sit in their own fetchers; the trusted core's state; the screen, except through messages the trusted core checks.
- **Can:** send false pages and contents for its own channel, which that channel's server, or a network attacker on a plain connection, could do anyway.
- **Today, on macOS, can also:** read the user's files, because its sandbox profile denies only file writes (requirement 3.3), and send them over the network. This is the most serious gap in what is built (section 16, rank 3).

**Remaining risk:** low by design; **medium as built** until the fetcher's sandbox denies file reads.

### 5.2 Boundary 2: the fetcher into the trusted core

**Threats.** False results: oversized contents, hostile titles and section names, bytes in place of pages.

**Mitigations.**
- The protocol's bounds: at most 1,000 contents entries; every title and name cleaned and cut on arrival (requirements 4.2 and 4.8).
- Page bytes are cut into pieces and passed to the channel's decoder unread (requirement 5.3).
- A malformed message stops the fetcher (requirement 4.3).
- Only the trusted core decides which decoder a fetcher's bytes reach: its own channel's.

**Remaining risk:** low. It depends on the protocol's readers being correct, which is why they must be fuzzed (requirement 4.6, not met).

### 5.3 Boundary 3: the trusted core and the decoders

**Threats.** Faults in content detection, the format readers, the decoder or the renderer, triggered by crafted teletext; a compromised decoder sending false frames or floods of them.

**Mitigations.**
- Every channel has its own decoder process (requirement 7.2), with no network (requirement 3.4).
- Every frame is checked on arrival: the exact raster size, at most 32 colours, every index within the palette, a valid page number (requirement 9.2).
- Frames are passed on only for pages the interface shows, and only when changed (requirements 9.4 and 9.5).
- Titles are cleaned and bounded (requirement 11.5).

**If a decoder is compromised.**
- **Can:** draw any picture into its own channel's page images and give its pages any title, which its server could do anyway.
- **Cannot:** reach the network, other channels or the trusted core's state.
- **Today, on macOS, can also:** read the user's files. It cannot send them anywhere, because it has no network, and it cannot encode them into frames the user would not see as pictures; but a later fault elsewhere could combine with it. Its sandbox must deny file reads (requirement 3.4).

**Sound (requirements 16.1 to 16.4).** A decoder also reads its channel's audio packets, with the engine's bounded, fuzzed audio receiver, and sends voice operations and 16-bit samples. The trusted core passes on only the active tab's, re-checks each against the sounds that decoder defined, and holds the interface to 32 MB of its samples. A compromised decoder **can** play any sound in its own channel's four voices while its tab is active, as its server could; it **cannot** sound from a background tab, reset another channel's sounds, exceed the bounds, or play when the user has no output device.

**Remaining risk:** low.

### 5.4 Boundary 4: fonts

Not present yet: no font is fetched. When fonts are, they are read by a confined font helper and checked twice, as the earlier design set out (requirement 3.5).

### 5.5 Boundary 5: the trusted core into the interface process

**Threats.** Teletext data reaching the interface's readers; deceptive characters in page text; secrets leaking into the interface.

**Mitigations.**
- Pages cross only as indexed Portable Network Graphics images our encoder produced from checked palette indices. The interface's reader accepts only that shape: indexed, 8 bits a dot, exactly 480 by 500, at most 32 colours, with a bound on its memory (requirement 9.3). An attacker controls pixel values within a fixed raster and palette, never file structure.
- Text crosses only cleaned and bounded, and the interface refuses text that still holds a hidden character (requirements 4.8 and 11.5).
- Text is only ever drawn as text runs: there is no markup to inject into (requirement 10.2).
- No cookie, credential, setting or history beyond the strip's own state is ever sent (requirement 3.6).
- Sound crosses only as voice operations and 16-bit samples, each bounded by the protocol and checked by the trusted core (requirement 16.4). The interface parses no audio format; it copies samples into one engine mixer per channel, which has no `unsafe` code. Each channel's sound is tagged by the trusted core, not the decoder, so a decoder cannot reach another channel's sound; only the trusted core can reset or mute a channel.
- The Dock menu on macOS (decision 39) is the one place with `unsafe` code: Objective-C runtime calls in the interface process that add a method to winit's application delegate and build the menu from tab titles the trusted core cleaned. Its input is our own tab list; its output is a tab index or mute-all, sent to the trusted core as ordinary intents. Playing needs the operating system's audio output, reached through `cpal` on macOS; the interface's sandbox profile already allowed it (verified 19 October 2026).

**Remaining risk:** low. The image reader is a third-party crate (`png`), reading only our own encoder's output; it must still be fuzzed (requirement 4.6).

### 5.6 Boundary 6: intents and strip input from the interface process

**Threats.** A compromised interface process sends messages to navigate, confirm a channel change, change security settings, read secrets, influence fetching, or exhaust the trusted core.

**Mitigations.**
- **Page intents carry no address.** They can report visible pages, click a cell, and select, close or open tabs (requirement 4.5).
- **Visible pages affect only rendering,** never fetching: the fetcher takes no input from the trusted core at all, so nothing the interface does can change any request (requirement 4.7). This closes the covert channel between channels that the earlier design found (`../prototypes/docs/security-analysis.md` section 5.6) for every channel fetched from a contents file or a bundle, and for files.
- **Visible pages of another channel are filtered out** by the trusted core (requirement 4.4).
- Every message is bounded and checked; a malformed one ends the application (requirements 4.2 and 4.3).

**What a compromised interface process can do.** The trusted strip is drawn by the interface process, so its integrity rests on that process (decision 33; requirement 3.9):
- **Can:** draw a false strip; send strip input, and so navigate any tab to any address the trusted core's rules accept, go back and forward, and set bookmarks; see every page on screen.
- **Cannot:** read cookies, history beyond what is shown, or files (once its sandbox denies reads); reach the network; change what any fetcher requests; change a security setting, because none is reachable from strip input yet. When the site-change screen and security settings are built (requirements 8.4 and 11.2), a compromised interface could confirm them too; that is the cost of option 1 that open decision 3 weighs.
- **What reaching it takes:** a fault in NeoTeletext's own interface code (no `unsafe`, requirement 10.12) or in its drawing libraries (`vello_cpu`, Parley, `png`, winit, softbuffer), reached only through our own images, cleaned text and the user's own input.

**Remaining risk:** low likelihood; high impact within its reach (navigation under the user's eyes, without secrets). Controls: small code, no `unsafe`, fuzzing of the interface's readers and the hostile interface harness (requirements 3.9 and 4.6, not met), and open decision 3.

---

## 6. What leaving the web engine changed

The earlier design's main concession was the system webview: many millions of lines of C++ and Objective-C with a steady flow of security fixes, which it contained with a window host, a Content Security Policy, the removal of WebRTC, and on Linux an outside confinement, a nested display and a display host of its own (`../prototypes/docs/security-analysis.md` section 6).

**With no web engine:**
- **The three levels of compromise collapse to one:** a fault in the interface process (section 5.6).
- **No script runs,** so no package supply chain runs in the interface, and no script can reach the network.
- **The interface process can be denied the network on every platform,** including macOS, where proof A showed a webview cannot (`../prototypes/docs/proofs.md` section 2). Built on macOS.
- **On Linux,** the interface needs the display as any application does, and nothing more; no nested display or display host is needed, because nothing in it is a web engine to contain.
- **The cost:** NeoTeletext owns its interface code, including what browsers gave for free: text editing, input methods, accessibility. Each is our code and must be tested as such.

---

## 7. Spoofing and the line of death

**The threat.** A teletext page can draw anything at teletext resolution: a fake address bar, a fake site-change screen, a fake button, a fake system dialog.

**Mitigations.**
- The address, identity, encryption state and, when built, the site-change screen are shown only in the trusted strip, which is drawn outside the page area and which no page image is ever drawn over (requirements 11.1 and 11.2).
- **Every page image is kept below the strip,** including the zoomed page and the overview's popup of a page at its native size, which shrinks on a short window rather than cover the strip (observed in this codebase's tests).
- A click on a page means only "this cell" (requirement 12.3).
- The strip stays visible in every layout (requirement 11.4).
- Every new layout is reviewed for spoofing (requirement 14.5, no review recorded yet).

**Remaining risk:** medium. This is where safety depends on discipline in visual design: an interface built for animation is exactly where a security indicator could be hidden or imitated.

---

## 8. The application update mechanism

Unchanged from the earlier design (`../prototypes/docs/security-analysis.md` section 8), and not built: The Update Framework with offline threshold keys, side-by-side installation, an atomic switch, automatic rollback, profiles never migrated in place, and one launcher scheme on every desktop. Its remaining risks are key custody and faults in the launcher, both low once requirements 13.1 to 13.13 are met.

---

## 9. Supply chain

**Threat.** Malicious or vulnerable code in a dependency.

**Mitigations.**
- **No run-time JavaScript and no package manager for the interface.** The earlier design's medium-likelihood risk from React and its packages is gone.
- **Rust dependencies pinned to exact versions** in `native/Cargo.toml`, every one permissively licensed (decision 34); lockfile committed. `cargo-deny` and `cargo audit` are required and not yet in the check script (requirement 14.3).
- **Where a dependency sits limits what it can do.** The trusted core depends on two engine crates and the protocol only (requirement 3.2). The drawing libraries sit in the interface process, the engine's readers in the decoder and fetcher.
- **The engine** is pinned to one commit in its own repository and never patched here (requirement 2.5).

**Remaining risk:** low for the trusted core. Low to medium for the interface process's drawing libraries, which are young (`vello_cpu` 0.3, Parley 0.12) and run in the process that draws the strip.

---

## 10. Local attackers

- **No network ports are opened** by the application. Messages travel over pipes between our own processes.
- **No debugging interface** in any build (requirement 10.8).
- **Under X11** any local process can send input to the window, as to every X11 application; the trusted core still applies its rules to everything it receives.
- **History and bookmarks are not saved yet;** when they are, the data directory must have user-only permissions and, on macOS, sit inside the App Sandbox container.

**Remaining risk:** low. A process running as the user can read the user's own files; that is outside this analysis's scope.

---

## 11. Availability

**Threats.** Floods of pages or frames, channels with thousands of pages, endless push feeds, decompression bombs, decoders that stop answering, many tabs.

**Mitigations.**
- Frames only for visible pages, and only when changed (requirements 9.4 and 9.5).
- Every message bounded; at most 1,000 contents entries; files at most 64 megabytes or 4,096 page files.
- The engine's response and decompression limits (requirements 5.11 and 5.12).
- A decoder or fetcher that breaks the protocol is stopped; its channel's pages stay.
- The window redraws only on input, change or animation; a folded overview draws nothing and starts no animation (observed in this codebase's tests).

**Not yet:** restarting a decoder that stops answering; the live-feed threshold (requirement 6.5); a bound on how many tabs, decoders and fetchers may run.

**Remaining risk:** low.

---

## 12. Privacy

- **Fetching never depends on viewing** (decision 22): a channel's server cannot see which of its pages a viewer reads (requirements 4.7 and 5.15).
- **Cookies are host-only** and stay within their channel's fetcher; there is no third-party context (requirements 7.6 and 7.7).
- **No `Referer` header, and a minimal user agent** (requirement 5.14).
- **The interface process stores nothing** (requirement 7.9).
- **No telemetry.**
- **Every channel is unencrypted today,** because Transport Layer Security is not built; the strip says "Not secure" (requirement 5.9). A network observer can read what is fetched.
- **Closing a tab** stops its fetcher at once, which the channel's server can see; the grace period is required (requirement 5.17).

---

## 13. Platform notes

| Platform | Notes |
|---|---|
| macOS | The decoder, fetcher and interface run under `sandbox-exec` profiles: no network for the decoder and interface, no file writes for any. The profiles start from "allow everything", so file reads are still allowed, and `sandbox-exec` is deprecated by Apple; the release needs the App Sandbox with each process as its own helper, starting from "deny everything" (requirements 3.7 and 3.8). The trusted core is not sandboxed. |
| Windows | Not built. AppContainers for the fetcher, decoder and interface (`architecture.md` section 6.5). |
| Linux | Not built. Landlock and seccomp filters for every untrusted process, and an empty network namespace for the decoder and interface. The interface needs the display like any application. |
| Android, iOS | Later. On iOS, with one process, the decoder's only wall is whatever open decision 2 chooses. |
| Microcontrollers | A single-purpose device with one program; it needs its own analysis once its features are decided (open decision 5). |

---

## 14. Comparison with the alternatives

| Design | Web engine | Script at run time | What compromising the interface gains | Interface richness |
|---|---|---|---|---|
| The earlier design (React in a webview, window host, native strip; `../prototypes/docs/`) | Yes, contained | Yes, as page code | Level 1 (page code): wrong pixels and the use of open channels; level 3 (an escape into the window host): a false strip and forged strip input, no secrets | Full |
| **This design, adopted** (our own native interface in its own process, drawing the strip) | **No** | **None** | A false strip and forged strip input, no secrets; reaching it needs a fault in our own code or its drawing libraries | Full, by our own drawing |
| The same, with the strip in a smaller separate process (open decision 3) | No | None | Wrong pixels in the page area only | Full |
| egui (the engine's own design) | No | None | Everything, in one process | Lowest |

---

## 15. Findings from this codebase

These were observed, not reasoned:

1. **The macOS sandbox profiles allow reading files.** They deny network and file writes from "allow everything" (`core/src/main.rs`). Recorded as a gap in requirements 3.3, 3.4 and 3.6.
2. **macOS reports a slow mouse-wheel notch as a tenth of a line,** and speeds up quick ones (measured 16 October 2026). Not a security finding; recorded because the interface's handling of input is our own code now, and each such assumption must be measured.
3. **The engine's user agent fetches a channel in its contents file's order at its own fixed rate,** whatever is on screen: measured, 218 of 800 pages in 45 seconds. The fetcher takes no input from the trusted core, so the interface has no path to influence fetching.
4. **The interface's protocol refuses every kind it may not receive,** and messages with hidden characters, oversized lengths or trailing bytes (tests in `protocol`).

---

## 16. Remaining risks, ranked

| Rank | Risk | Likelihood | Impact | Main controls |
|---|---|---|---|---|
| 1 | Spoofing of the trusted strip by page images, especially in animated layouts and popups | Medium | The user is misled about where they are | Strip outside the page area; page images kept below it; recorded layout reviews (requirements 11.1 to 11.4, 14.5) |
| 2 | A fault in the interface process, which draws the strip | Low | A false strip and forged navigation; no secrets | Our own code, no `unsafe`, inputs limited to our own images and cleaned text; fuzzing and the hostile harness (requirements 3.9, 4.6, 10.12); open decision 3 |
| 3 | **As built:** sandboxes that allow file reads on macOS, and none elsewhere | Low (needs a fault in a reader first) | A compromised fetcher could send the user's files to a server | Profiles that start from "deny everything"; the App Sandbox; Windows and Linux sandboxes (requirements 3.3, 3.4, 3.6 to 3.8) |
| 4 | A fault in the trusted core, the one process holding everything | Low | Full compromise | Small, no format readers, bounded protocol, no `unsafe` (requirements 3.2, 4.2, 14.2); fuzzing (requirement 4.6) |
| 5 | **As built:** no Transport Layer Security | Certain, for now | Every channel readable and alterable on the network | Shown as "Not secure"; open decision 1 |
| 6 | Theft of the update root keys at threshold (once updates exist) | Low | Malicious update | Threshold, offline hardware keys, separate holders (requirements 13.8, 13.9) |
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
8. **Process rights:** network and file access fail where denied, on each platform (requirements 3.3 to 3.7).
9. **Fuzzing** of every reader in the trusted core and the interface (requirement 4.6). **Built:** `native/fuzz`, one target for each message reader, the decoder's engine, the interface's image reader and the address parser.
10. **The hostile interface harness** (requirements 3.9, 4.5 and 4.7).
11. **Channel isolation:** cookies and pages never cross channels (requirements 7.3 and 7.8).
12. **Dependencies:** `cargo-deny`, `cargo audit`, lockfile review (requirement 14.3). **Built** in part: `cargo-deny`.
13. **Updates:** the full set of update tests (requirement 13.12).
14. **Spoofing:** recorded review of every layout (requirement 14.5).
