# NeoTeletext: security analysis

| | |
|---|---|
| **Status** | Analysis of the proposed architecture, 15 October 2026. Where a point was observed on this codebase rather than reasoned, it says so. |
| **Companion documents** | [`architecture.md`](architecture.md) describes the design. [`requirements.md`](requirements.md) states each requirement this analysis relies on, numbered; this document cites them as "Requirement N.N". |

**Abbreviations.** None are used. Technical terms are written out in full; `architecture.md` section 18 is the glossary.

## Contents

1. Summary
2. What is protected
3. Who attacks, and how
4. Trust boundaries
5. Analysis of each boundary
6. The system webview: the main concession
7. Spoofing and the line of death
8. The application update mechanism
9. Supply chain
10. Local attackers
11. Availability
12. Privacy
13. Platform notes
14. Comparison with the alternatives
15. Findings from the prototype
16. Remaining risks, ranked
17. Checks that keep these properties

---

## 1. Summary

The owner's requirement is that being hacked is not acceptable under any circumstances (decision 4). No design can promise that no component will ever have a fault. This design therefore works to a different, achievable standard: **compromising any single component must gain an attacker nothing of value.**

The design meets that standard by:

- **Reading untrusted bytes only in processes with no rights.** These are the fetcher (network only), the decoder (no network, no files, inside the WebAssembly interpreter `wasmi`) and the font helper (no rights).
- **Keeping a trusted core that contains no web engine,** reads no network format, and owns every secret and every decision.
- **Treating the interface drawn with web technology as untrusted.** Its page code runs in the system webview's own sandboxed content process, which on macOS has no window server and no network sockets. The window host that holds the webview has no secrets and makes no decision (decision 27). On Linux, where the content process has no sandbox of its own under Tauri, the window host and every process it starts are confined from outside: no network, no files and no local services (decision 28). The interface receives only images our own encoder made and cleaned plain text.
- **Placing every security indicator and decision in a native trusted strip,** in the same window as the webview but outside page code's reach (`docs/proofs.md` section 3).
- **Making updates verifiable and recoverable:** The Update Framework metadata, side-by-side installation, atomic switching and automatic rollback.

**The main remaining risks,** detailed in section 16:
1. A fault in the trusted core itself, which is why it is kept small.
2. Spoofing of the trusted strip by page images or by the interface, in an interface built for animation.
3. The update signing keys.
4. On Linux, the display. A fault in the web engine needs one, and under X11 a display lets it type into the user's other programs. The window host therefore gets a nested display of its own (decision 29). NeoTeletext's own display host (decisions 30 and 31) draws the strip outside the nested display; its smallest form stopped every attempt to type into or cover the strip from inside (`docs/proofs.md` section 6.14).
5. Slow security updates of the Linux webview, confined from outside to a process with no rights (decision 28).

---

## 2. What is protected

| Asset | Where it lives | Why it matters |
|---|---|---|
| Which channel each tab shows, and the address the user sees | Trusted core; shown in the trusted strip | The user's decisions rest on it |
| Cookies, and anything a channel's login sets | Trusted core's cookie store | Access to the user's accounts on teletext services |
| History, bookmarks, presets, settings, sessions | Trusted core's data directory | Private; settings control security |
| The user's own files | The file system; opened by the trusted core and read by a decoder | Private |
| The integrity of the application's code | Installed versions, the launcher, the update keys | Everything else depends on it |
| Availability | Every process | The application must stay usable |

---

## 3. Who attacks, and how

| Adversary | Capabilities |
|---|---|
| **A hostile teletext service** | Sends any bytes in any role: malformed or enormous packets, crafted enhancement data, misleading page text, links and redirects to other channels, cookies, hostile descriptors, contents files, fonts and discovery markup, and live feeds that never stop |
| **A hostile broadcast capture or file** | The same, through files the user opens |
| **A network attacker** | Reads and alters unencrypted traffic, which is allowed (decision 10); tries to redirect, downgrade or replay |
| **A compromised update server** | Serves any files from the update subdomain |
| **A thief of a signing key** | Holds one update signing key |
| **A compromised dependency** | Code inside a crate or package that is built into the application |
| **A local process running as the user** | Connects to local ports; reads the user's files |
| **A compromised component** | Any one of our own processes, assumed compromised to test that the others hold |

Out of scope: an attacker who already controls the user's account or has administrator rights, and physical access to the machine.

---

## 4. Trust boundaries

```
network ──[1]──► fetcher ──[2]──► trusted core ──[3]──► decoder (one per channel)
files ─────────────────────────► trusted core ──[3]──► decoder
network ──[1]──► fetcher ──[2]──► trusted core ──[4]──► font helper
trusted core ──[5]──► window host (images, plain text, view state)
window host ──[6]──► trusted core (intents)
trusted core ◄──[7]──► trusted strip (native, drawn by the window host; strip state out, strip input in)
update subdomain ──[8]──► updater ──► versions directory ──► launcher
```

| Boundary | What crosses | The receiving side treats it as |
|---|---|---|
| 1. Network to fetcher | Raw protocol bytes | Untrusted |
| 2. Fetcher to trusted core | Teletext bytes (forwarded unread), typed descriptor, contents and discovery results, `Set-Cookie` headers, status | Untrusted, checked as data |
| 3. Trusted core and decoder | Bytes and commands in; frames, links, wants, forms and status out | Decoder output untrusted |
| 4. Trusted core and font helper | Font bytes in; a glyph table and hash out | Untrusted, checked twice |
| 5. Trusted core to window host | Images our encoder made, cleaned plain text, view state | The window host's input; contains nothing secret |
| 6. Window host to trusted core | Intents | Untrusted |
| 7. Trusted strip | Strip state to the window host; typed addresses, confirmations and security settings back as strip input | Trusted user input, provided the window host is not compromised (section 5.6) |
| 8. Update subdomain to updater | Metadata and release files | Untrusted until verified by The Update Framework and the platform signature |

---

## 5. Analysis of each boundary

### 5.1 Boundary 1: the network into the fetcher

**Threats.**
- Faults in the protocol readers: the HyperText Transfer Protocol, WebSocket, Server-Sent Events, the zxnet feed, the descriptor and contents readers, and the discovery reader.
- Decompression bombs.
- Slow or endless responses.
- Redirect loops.
- Requests aimed at the user's local network.

**Mitigations.**
- `neoteletext-web`'s strict, bounded, fuzzed readers (requirement 2.1).
- Limits per role on size, time and redirects (requirement 5.12).
- Bounded decompression (requirement 5.11).
- Local-network addresses refused by default (requirement 5.10).
- Discovery limited to link pointers in 8,192 bytes, analysed in `neoteletext/docs/web-discovery-security.md` (requirement 5.5).

**If the fetcher is compromised.**
- **What the attacker has:** the network and the cookies of requests currently in flight (requirement 7.5).
- **What the attacker does not have:** no files, no history, no other channels' cookies, no decoder, no way to show anything on screen except through messages the trusted core checks.
- **What the attacker can do:** send false responses for channels the user is visiting. That is no more than a network attacker can do to unencrypted traffic, which the user is already shown.

**Remaining risk:** low.

### 5.2 Boundary 2: the fetcher into the trusted core

**Threats.**
- False results: oversized descriptors, contents lists naming other channels, discovery results pointing at hostile addresses, cookies for another channel, bytes delivered for the wrong request.

**Mitigations.**
- The trusted core re-checks every typed result: counts, lengths and address syntax (requirement 5.4).
- Teletext bytes are forwarded unread to the decoder that asked for them (requirement 5.3).
- `Set-Cookie` is applied only for the request's own host (requirements 7.5 and 7.6).
- Discovery results become navigations through the site-change screen (requirement 5.6).
- Every message is bounded and a malformed one closes the fetcher (requirements 4.2 and 4.3).

**Remaining risk:** low. It depends on the trusted core's message readers being correct, which is why they are fuzzed (requirement 4.6).

### 5.3 Boundary 3: the trusted core and the decoders

**Threats.**
- Faults in content detection, the format readers, the decoder or the renderer, triggered by crafted teletext.
- A compromised decoder sending false frames.
- Requests for pages on other channels.
- Floods of frames.

**Mitigations.**
- The decoder has no rights and runs inside `wasmi` with memory, table and fuel limits (requirement 3.4).
- Operating-system sandboxes add a second wall on four platforms (requirement 3.7).
- Every frame is checked: known tab, dimensions, palette size, index range (requirement 9.2).
- A cross-channel request is a navigation through the site-change screen (requirement 8.4).
- Frames are sent only for visible pages and only when changed (requirements 9.4 and 9.5).

**If a decoder is compromised.**
- **What the attacker has:** one channel's public pages, held in the decoder's own memory.
- **What the attacker can do:** draw any picture into that channel's page images. That is no more than the channel's server can do.
- **What the attacker cannot do:** reach the network, files, cookies or other channels.

**Remaining risk:** low.

### 5.4 Boundary 4: fonts

**Threats.**
- A crafted font file attacking the font reader.
- A font table that misleads the renderer.

**Mitigations.**
- A confined font helper with no rights (requirement 3.5).
- The trusted core checks the font's hash and table shape, and the decoder checks the table again before use.
- Teletext fonts never reach the webview (requirement 5.7).
- `neoteletext/docs/font-security.md` covers the reader itself.

**Remaining risk:** low.

### 5.5 Boundary 5: the trusted core into the window host

**Threats.**
- Teletext data reaching the webview's parsers.
- Page text interpreted as markup.
- Deceptive characters in page text.
- Secrets leaking into the untrusted process.

**Mitigations.**
- Pages cross only as Portable Network Graphics images our encoder produced from checked palette indices. The webview's image decoder sees only well-formed files from one encoder. An attacker controls pixel values, within a fixed raster and a 32-colour palette, never file structure (requirements 9.3 and 10.1).
- Page text crosses only as cleaned, bounded plain text (requirement 11.5).
- Page text is rendered by React only as text nodes, which escape every value; nothing builds markup from strings (requirement 10.2).
- The script cannot build markup or code from strings (requirement 10.4).
- No cookie, credential, security setting or unneeded history ever crosses (requirement 3.6).

**Remaining risk:** low.

### 5.6 Boundary 6: intents from the interface

**Threats.** A compromised interface sends intents to:
- navigate to another channel;
- confirm the site-change screen;
- change security settings;
- read cookies or history;
- exhaust the trusted core.

**Mitigations.**
- Intents can only show pages on the current channel, change layout, switch or close tabs, report visibility, and pass keys and cell clicks (requirement 4.5).
- Changing channel, confirming the site-change screen and changing security settings are possible only in the trusted strip (requirements 8.5 and 11.2).
- A cell click means only "this cell"; the trusted core and decoder decide whether it is a link, and a link to another channel goes through the site-change screen (requirement 12.3).
- Every intent is bounded and checked (requirements 4.2 and 4.4).

**Three levels of compromise.** Measured on macOS in `docs/proofs.md`.

1. **Page code compromised** (a malicious or vulnerable JavaScript package, or any other way to run script in the page). The most likely case.
   - **Can:** show wrong pixels inside the webview's area; use the channels already open, switching tabs, showing other pages of the same channels and closing tabs; see every page on screen.
   - **Cannot:** draw over or read the trusted strip (Proof B2); produce strip input (requirement 11.1); reach the network, which is stopped by the Content Security Policy and WebRTC removal, with nothing reaching a test server in Proof A; read a secret; change channel; change a security setting.
2. **The web content process compromised** by a memory-corruption fault in the web engine.
   - **Can:** everything in level 1, and ask the webview's networking process to load any address, bypassing the Content Security Policy, which is enforced inside the content process. That could send what is on screen to a server the attacker chooses.
   - **Cannot, on macOS:** open a window, so it cannot cover or imitate the trusted strip; open network sockets of its own; read files. Its sandbox denies all three by default (`com.apple.WebProcess.sb`, `docs/proofs.md` section 3.2).
   - **On Linux** (decision 28, `docs/proofs.md` section 6.8):
     - **Cannot:** it inherits the window host's confinement, so it cannot reach the network, read the user's files, or reach any local service.
     - **Can:** reach the display it is given.
       - **On the user's display:** it could cover the strip, and under X11 type into the user's other programs (`docs/proofs.md` section 6.8). Decision 29 therefore gives the window host a nested display of its own.
       - **On a nested display:** it reaches nothing of the user's (section 6.11). With a stock compositor it can still type into the window host's own strip (section 6.12), so the strip moves to NeoTeletext's own display host, outside the nested display (requirement 3.11, decision 30).
3. **The window host compromised**, which requires a second fault: an escape from the web content process's sandbox.
   - **Can:** draw anything in its windows, including a false trusted strip; forge strip input, and so confirm navigations and change security settings; use the network entitlement the webview requires.
   - **Cannot:** read cookies, credentials, history or bookmarks, which stay in the trusted core (requirement 3.6); read the fetcher's connections or the decoders' memory.

**The covert channel between channels (found 15 October 2026).**
- **What a hostile interface can see:** every page on screen, including pages of every open channel.
- **What it can cause:** requests for other pages of channels already open. If a channel serves one file per page, each such request reaches that channel's server.
- **The attack:** a hostile interface reads a page shown from channel A, then encodes its contents in the pattern of page numbers it asks for from channel B. If channel B's server belongs to the attacker, data leaks from A to B. Each request carries about eleven bits (one of 2,048 page numbers), and its timing carries more.
- **Why it matters:** it breaks the isolation between channels (requirement 7.3), and it contradicts the owner's principle that the interface's code must make no difference to security (decision 20, requirement 3.9).
- **Options considered:** rate-limiting requests (shrinks the leak, does not close it); accepting page requests only from trusted input (closes it, but stops page keys working in the tab, which the owner requires); one interface process per channel (closes it, at the highest cost).
- **Decided:** fetching never depends on viewing (decision 22). Pages are fetched whole and in an order fixed by the channel's data, refreshed on a fixed schedule or by push, and a closed tab's channel keeps refreshing for a grace period. Keys and interface requests only select among pages already held. Nothing the interface does, and no key, can influence any request, so there is nothing to encode a message in (requirements 4.7 and 5.15 to 5.17).
- **Accepted exception (decision 23):** channels that publish only separate page files, with no bundle and no contents file, cannot be fetched in advance, because nothing says which pages exist. Their pages are fetched when selected, as any browser does. For those channels alone, a compromised interface could forge key presses whose page requests carry information to that channel's server, and the server can see which pages are read. The owner chose this over walking the channel's links in advance, which would load small servers and need `neoteletext`'s rule against crawling changed.
- **Side benefit:** a channel's server cannot see which of its pages a viewer reads.

**Remaining risk:** low for bundled channels and channels with a contents file, provided requirements 4.5, 4.7 and 5.15 to 5.17 are enforced by the hostile interface harness. **Accepted** for channels with only separate page files: the leak exists only if page code is compromised and the user also has a channel run by the attacker open as one of those channels. **Level 2 and level 3 compromises** can reach the network directly and so can leak what is on screen; they need a fault in the web engine itself, and level 3 a second fault escaping its sandbox (section 16).

### 5.7 Boundary 7: the trusted strip

The trusted strip is drawn natively by the window host, in the webview's window, from state the trusted core sends. It takes only the user's own input, which reaches the trusted core as strip input (decision 27).

**Threats.**
- Pasted or programmatic input confirming a navigation.
- Deceptive addresses.

**Mitigations.**
- Confirmations are not accepted from pasted input or from the page area (requirement 8.5).
- The address display rules emphasise the host and show internationalised names safely (requirement 11.3).

**Remaining risk:** low against page code. On Linux, a compromised web content process can forge or cover the strip while the strip is drawn inside a display the content process can reach. Requirement 3.11 moves it out, into NeoTeletext's own display host (section 5.6, level 2; decision 30).

### 5.8 Boundary 8: updates

Analysed in section 8.

---

## 6. The system webview: the main concession

The system webview is many millions of lines of C++, Objective-C or Java with a steady flow of security fixes. The engine's own design rejected Tauri for exactly this reason: it put the webview inside the trusted process.

**What changes in this design.**
- **The webview is not in the trusted process.** It is hosted by the window host, which holds no secrets and makes no decision (requirement 3.6), and its page code runs in the platform's sandboxed web content process.
- **The webview sees no attacker-chosen bytes:** only our own React interface and its approved packages, images from our own encoder and cleaned text (requirement 10.1).
- **Its most exposed parts have nothing to attack:** the markup parser, the JavaScript engine and the image and font decoders.
- **If it were compromised,** section 5.6 sets out what each level of compromise gains.
- **Its configuration closes the remaining doors:**
  - a Content Security Policy with no inline script, no evaluation and no remote sources (requirement 10.5);
  - navigation and new windows refused (requirement 10.6);
  - capabilities naming webview labels only, with no plugins that touch files, processes or the network (requirement 10.7);
  - no inspector, remote debugging or global Tauri object in release builds (requirement 10.8);
  - dropped files routed to the trusted core (requirement 10.9);
  - no network for page code: the Content Security Policy, WebRTC removed in every frame, the webview's own feature switches, and the web content process's lack of sockets (requirement 10.11). On macOS the operating system cannot also deny the window host the network, because the webview then renders nothing (`docs/proofs.md` section 2).

**Remaining risk.** A fault in the web engine (level 2 in section 5.6) could send what is on screen to an attacker's server, because the networking process serves the content process's requests and the operating system cannot remove the window host's network entitlement on macOS. It could not imitate the strip or read a secret. An escape into the window host (level 3) could also imitate the strip and forge confirmations, but still read no secret. Both need faults in the web engine itself. **Low likelihood; medium impact for level 2 (confidentiality of what is on screen), high for level 3 (navigation and settings, without secrets).**

**On Linux** the network is closed to level 2 as well: the window host and its children run in an empty network namespace (decision 28). Level 2 still needs a display. On the user's display it could imitate the strip and, under X11, type into the user's other programs, so the window host gets a nested display of its own (decision 29). **Low likelihood; high impact (strip input forged) until NeoTeletext's own display host (decision 30) is built in full. In its smallest form it already stopped every attempt from inside to type into the strip or cover it (`docs/proofs.md` section 6.14), so on Linux even level 3 can neither draw over the strip nor type into it.**

---

## 7. Spoofing and the line of death

**The threat.** A teletext page can draw anything at teletext resolution: a fake address bar, a fake site-change screen, a fake "confirm" button, a fake system dialog. A compromised interface can draw anything in its own area.

**Mitigations.**
- The address, channel identity, encryption state and the site-change screen are shown only in the native trusted strip, which neither pages nor the interface can draw over (requirements 11.1 and 11.2).
- Confirmations are accepted only in the trusted strip; a click on a fake button in a page is only a cell click (requirements 8.5 and 12.3).
- Row 0, columns 0 to 7, belong to the receiver, as in the engine design.
- The trusted strip stays visible in every layout, or reappears on any interaction (requirement 11.4).
- Every new layout is reviewed for spoofing (requirement 14.5).

**Remaining risk:** medium. This is the one area where safety depends on discipline in visual design: an interface built for animation is exactly where a security indicator could be hidden or imitated. The recorded review in requirement 14.5 is what keeps it in check.

---

## 8. The application update mechanism

The updater can replace the application's code, so it is the most powerful component in the system.

### 8.1 Threats

1. A compromised update server serving a malicious build.
2. A compromised update server serving old, vulnerable builds (rollback) or withholding new ones (freeze).
3. A stolen signing key.
4. A network attacker altering downloads.
5. An interrupted or failed update leaving the application unusable.
6. A new version that runs but corrupts the user's data.
7. A fault in the launcher, the component that recovers from bad updates.

### 8.2 Mitigations

| Threat | Mitigation | Requirement |
|---|---|---|
| 1 | The Update Framework metadata signed by offline keys decides what is accepted. A server cannot produce valid signatures. The platform code signature is checked as well. | 13.3, 13.8 |
| 2 | Version numbers only increase, so older builds are refused. Metadata expires, so a frozen server shows "updates unavailable" instead of silently withholding fixes. | 13.8 |
| 3 | Root keys need a threshold. Root and targets keys are offline. A single stolen key is not enough, and keys can be replaced by a new root signed with the old threshold. | 13.8, 13.9 |
| 4 | Every file's hash and size are in signed metadata. | 13.8 |
| 5 | Side-by-side install, atomic pointer switch, previous version kept. | 13.2, 13.4, 13.5 |
| 6 | Profiles are copied and migrated, never changed in place. Rollback finds its own profile. | 13.6 |
| 7 | The launcher is small, fully reviewed, rarely changed, and itself updated only by atomic directory exchange or through a fixed stub. | 13.7 |

### 8.3 Why one launcher scheme instead of the operating systems' package managers

- MSIX on Windows and Flatpak on Linux download and install by themselves. They trust only the package's code signing certificate or the repository's signature.
- Using them as the update path would add a second, weaker way in: one stolen code signing key would be enough to ship an update.
- One launcher scheme keeps The Update Framework on every path (requirement 13.10).
- The cost is that the launcher is our own code, which is why it must be small and fully reviewed, and why the tests of requirement 13.12 must pass.

### 8.4 Remaining risk

- **Key custody:** the strength of The Update Framework rests on the offline keys. If the threshold of root keys were stolen together, an attacker could ship an update. Three hardware keys held by three separate people, one kept in a safe, with a threshold of two (decision 24), make that very unlikely. **Low.**
- **A fault in the launcher:** **low.** It is small and its behaviour is proved by the tests of requirement 13.12.

---

## 9. Supply chain

**Threat.** Malicious or vulnerable code in a dependency, built into a process.

**Mitigations.**
- **Run-time JavaScript is a fixed, approved list:** React, React DOM and the Tauri interface package, bundled at build time from the committed lockfile, with installation scripts blocked and `npm audit` in the check script (requirement 10.3). Keeping React was the owner's decision (decision 19).
- **That code runs only as page code in the web content process** (level 1 of section 5.6). A compromised package there gains what compromised page code gains: wrong pixels in the interface area and the use of channels already open. It cannot change channel, change a security setting or read a secret.
- **Rust dependencies:** lockfiles committed and reviewed, `cargo-deny` and `cargo audit` in the check script, package installation scripts blocked, and every new library approved by the owner (requirement 14.3).
- **Where a dependency sits limits what it can do.** Most dependencies sit in processes with no rights. The trusted core's dependency tree is kept small and contains no web engine (requirement 3.2).

**Remaining risk:** medium likelihood for the JavaScript packages, because supply chain attacks on such packages are common; low impact, because of where they run. Low for the Rust dependencies of the trusted core, which are few and reviewed.

---

## 10. Local attackers

- **No local network ports are opened.** Frames and intents travel over pipes between our own processes.
- **No debugging endpoints in release builds** (requirement 10.8). The Chromium Embedded Framework trial showed how a debugging port gives any local process control of the interface.
- **The data directory has user-only permissions.** On macOS it sits inside the App Sandbox container.

- **On Linux, accessibility requests and input method text reach the display host** (decision 32). Any process of the user's on the accessibility bus can send a screen reader's requests, as it can to every desktop application. The display host must act on them only as on the user's own input in the strip, and nothing inside the window host's confinement can reach that bus (`docs/proofs.md` section 6.16).

**Remaining risk:** low. A process running as the user can read the user's own files; that is outside this analysis's scope.

---

## 11. Availability

**Threats.**
- Floods of pages or frames.
- Channels with thousands of pages.
- Endless live feeds.
- Decompression bombs.
- Decoders that stop answering.
- Many tabs.

**Mitigations.**
- Frames only for visible pages, and only when changed (requirements 9.4 and 9.5).
- Bounded caches with least-recently-used eviction.
- `wasmi` fuel limits.
- Restart of a decoder that stops answering.
- Response limits and bounded decompression (requirements 5.11 and 5.12).
- Closing of misbehaving live feeds (requirement 6.5).

**Remaining risk:** low, provided every limit is a constant defined once in neoteletext, with a test (requirement 2.4).

---

## 12. Privacy

- **Cookies are host-only and stay within their channel;** there is no third-party context (requirements 7.6 and 7.7).
- **No `Referer` header, and a minimal user agent** (requirement 5.14).
- **The webview stores nothing of any channel** (requirement 7.9).
- **No telemetry.** The updater contacts only the update subdomain (requirement 13.11).
- **Unencrypted channels are allowed** (decision 10). The user sees which channels these are (requirement 5.9). A network observer can read what is fetched from them.

---

## 13. Platform notes

| Platform | Notes |
|---|---|
| macOS | Trusted core, fetcher and decoder as sandboxed services. The window host must hold the outgoing-network entitlement, because WKWebView renders nothing without it; page code is kept off the network by the Content Security Policy, WebRTC removal and the web content process's sandbox instead (decision 27, `docs/proofs.md`). The fetcher and decoder sandboxing gap in `neoteletext` must be closed (requirement 3.8). |
| Windows | Full separation with AppContainers. WebView2 is updated independently by Microsoft. |
| Linux | Full separation with Landlock and seccomp filters. WebKitGTK may lag upstream security fixes; it is confined to the window host, and the application warns about old versions (requirement 10.10). Under Tauri, WebKitGTK's content process has no sandbox of its own. Instead the window host, and with it every WebKitGTK process, is confined from outside (decision 28, requirement 3.10): an empty network namespace, private `/run` and `/tmp` holding only the display's socket, and Landlock. Measured: no network, no home directory, no local service (`docs/proofs.md` section 6.8). The window host draws on a nested display of its own, never the user's (decision 29, requirement 3.11), on which it reached nothing of the user's (section 6.11). The strip is drawn outside it, by NeoTeletext's own display host (decision 30). The window host has no system call filter yet (open decision 22). |
| Android | The decoder runs in an isolated service process inside `wasmi`. The webview runs within the application's process space, so the separation of section 5.6 is weaker; the webview still receives no teletext data and holds no secrets. |
| iOS | One process. The decoder's only wall is `wasmi`. WebKit's own content process still separates script from the application. This is the weakest platform, as in the engine design. |

---

## 14. Comparison with the alternatives

| Design | Web engine in the trusted process | Third-party script at run time | What compromising the interface gains | Interface richness |
|---|---|---|---|---|
| First proposal (webview as trusted shell, React) | Yes | Yes | Everything the user can do | Full |
| **This design, adopted** (React interface as sandboxed page code, a window host with no secrets, native trusted strip in the webview's window) | **No** | Yes, as page code in the web content process | Wrong pixels inside the webview's area, and use of channels already open | Full |
| Blitz (Rust markup and style renderer, no JavaScript engine) | No | None exists | Nothing script-related exists | Rejected by decision 19: it cannot run the React interface |
| egui (the engine's own design) | No | None | Not applicable | Lowest |

This design keeps the full richness of the first proposal, including React, while matching the egui design's core property: there is no web engine in the trusted process. The owner adopted it on 15 October 2026 (decisions 18 and 19).

**How the Linux confinement compares** with ten other ways of organising security in a Linux application (`architecture.md` section 6.7):
- **What it matches:** Chromium's first sandbox layer (namespaces), Firefox's in-process WebAssembly sandboxing (the decoder in wasmi), and the virtual-machine systems' trusted display (the display host).
- **What it lacks:** Chromium's second layer, a system call filter, for the window host (open decision 22). Until that is added, a fault in the web engine faces the whole kernel interface, though with no network, no user files, no local services and no display but its own.
- **Not adopted:**
  - *Flatpak* would take over updates, against decision 24, and has had sandbox escapes.
  - *The Wayland security-context protocol* depends on the user's compositor; it is proposed as an extra layer (open decision 23).
  - *A virtual machine per component* would be stronger, but far heavier; it is the upgrade path if process isolation proves insufficient.

---

## 15. Findings from the prototype

These were observed on this codebase, not reasoned:

1. **Capabilities and window labels** (Tauri 2.12.3): a capability that names a window label covers every webview in that window, because Tauri grants a command when either label matches. Capabilities must name webview labels, and commands must check their caller (requirement 10.7).
2. **Caller attribution in `tauri-runtime-cef` 3.0.0-alpha.5** (branch `tauri3`):
   - With several webviews in one window, the shell's calls were attributed to a tab's webview.
   - The runtime falls back to an arbitrary registry entry when it cannot identify the caller.
   - In the test it failed closed, but the reverse attribution would grant a tab the shell's permissions.
   - Any runtime change requires the attribution test (requirement 14.4).
3. **The tab sandbox probe** (Tauri 2 with WKWebView): a tab calling a command reserved for the shell was refused by the permission check ("not allowed by ACL", the literal error text). A start-up self-test of this kind remains useful in debug builds.
4. **Cookies on the `tauri://` scheme are dropped.** In this design cookies are held in Rust, so this has no effect.

---

## 16. Remaining risks, ranked

| Rank | Risk | Likelihood | Impact | Main controls |
|---|---|---|---|---|
| 1 | The covert channel between channels through the pattern of requests: closed for bundled channels and channels with a contents file once requirements 4.7 and 5.15 to 5.17 are implemented; **accepted** for channels with only separate page files (decision 23) | Low; needs both a compromised interface and an attacker's per-page channel open | One channel's pages leak to another channel's server | Fetching never depends on viewing (decision 22); per-page channels' requests confined to their own channel and bounded (requirement 4.7) |
| 2 | Spoofing of the trusted strip by page images or the interface, especially in animated or full-screen layouts | Medium | The user is misled into confirming a navigation | Native trusted strip; confirmations only there; recorded layout reviews (requirements 8.5, 11.1 to 11.4, 14.5) |
| 3 | A web engine fault taking over the web content process (level 2 of section 5.6) | Low | What is on screen can be sent to an attacker's server through the webview's networking process; no secrets. No strip imitation on macOS; **on Linux the strip can be covered**, because the content process is given the display (`docs/proofs.md` section 6) | Platform updates of the web engine; no teletext bytes reach it; on macOS its sandbox denies windows and sockets (`docs/proofs.md` section 3.2); on Linux the window host's confinement, which its children inherit: no network, files or local services (decision 28) |
| 4 | On Linux, the web content process using the display. On the user's display: typing into the user's other programs under X11 (measured). On a stock nested compositor: typing into the window host's strip (measured) | Low; needs a fault in the web engine | The user's account on the user's display; forged strip input on a stock nested compositor | A nested display of the window host's own (decision 29); the strip drawn outside it by the display host, NeoTeletext's own, which offers no input injection (requirement 3.11, decisions 30 and 31; proved in its smallest form, `docs/proofs.md` section 6.14) |
| 5 | An escape from the web content process into the window host (level 3 of section 5.6) | Very low; needs a second fault | A false trusted strip and forged confirmations and settings; no secrets | Secrets kept in the trusted core (requirement 3.6); the window host makes no decision |
| 6 | A fault in the trusted core, the one process holding everything | Low | Full compromise | Small, no web engine, no network format readers, fuzzed message readers, `unsafe` forbidden (requirements 3.2, 4.6, 14.2) |
| 7 | Theft of the update root keys at threshold | Low | Malicious update | Threshold, offline hardware keys, separate holders (requirements 13.8, 13.9) |
| 8 | A supply chain compromise of a Rust dependency in the trusted core | Low | Full compromise | Small dependency tree, review, auditing (requirements 3.2, 14.3) |
| 9 | A supply chain compromise of React or another run-time JavaScript package, running as page code | Medium | Compromise of a process with nothing of value; it can switch tabs and show other pages of channels already open | Fixed approved list, lockfile review, `npm audit`, process isolation, intent restrictions (requirements 3.6, 4.5, 10.3) |
| 10 | A Linux webview fault reached through our own content | Low | As the web engine faults above, for longer, because WebKitGTK updates may lag | No teletext data reaches it; confined from outside (decision 28); version warning (requirements 3.6, 3.10, 10.1, 10.10) |
| 11 | A decoder or fetcher fault | Low | Contained | No rights, `wasmi`, sandboxes (requirements 3.3, 3.4, 3.7) |
| 12 | Exhaustion of resources | Low | Unresponsive tab or application | Visibility culling, limits, fuel (section 11) |
| 13 | Runtime caller misattribution, as found with the Chromium Embedded Framework | Low on Tauri 2 | Permission bypass | One webview per window; attribution test on every runtime change (requirement 14.4) |

---

## 17. Checks that keep these properties

Every check below is in the project's check script, run by git hooks (requirement 14.1):

1. **Engine dependencies:** no interface toolkit in any `neoteletext` crate used (requirement 2.2).
2. **Trusted core dependencies:** no web engine, JavaScript engine or network format reader (requirement 3.2).
3. **Process rights:** network and file access attempts fail in the fetcher, decoder and font helper, and file access outside the bundle fails in the window host, on each platform; page code reaches no network; on Linux, only the nested display's socket is reachable from the window host and its web content process, no request to the user's systemd succeeds, and nothing typed or pointed from inside the nested display reaches the trusted strip (requirements 3.3 to 3.7, 3.10, 3.11 and 10.11).
4. **Message bounds and fuzzing:** every message kind, and fuzzing of every reader in the trusted core (requirements 4.2 and 4.6).
5. **Intent restrictions:** every intent kind fails to change channel, confirm the site-change screen or change a setting (requirement 4.5).
6. **Role crossing:** responses are refused in any role but their own (requirement 5.2).
7. **Detection and refusal:** non-teletext content is refused and never shown (requirement 6.3).
8. **Channel isolation:** cookies, caches and pages never cross channels (requirements 7.3 and 7.8).
9. **Webview configuration:** the Content Security Policy is no looser than requirement 10.5; capabilities name webview labels only; navigation is locked; release builds have no inspector or debugging (requirements 10.5 to 10.8).
10. **Forbidden script interfaces:** a lint rule (requirement 10.4).
11. **Text cleaning:** control, bidirectional and zero-width characters are removed (requirement 11.5).
12. **Updates:** the full set of update tests (requirement 13.12).
13. **Attribution:** before any runtime change (requirement 14.4).
14. **Dependencies:** `cargo-deny`, `cargo audit`, lockfile review (requirement 14.3).
15. **Spoofing:** recorded review of every layout (requirement 14.5).
