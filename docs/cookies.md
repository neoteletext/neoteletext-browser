# NeoTeletext: cookies

| | |
|---|---|
| **Status** | Proposed 22 October 2026 on the owner's request for "a specification and requirements for a full cookie implementation, which, to the extent that it makes sense, be a duplicate of the most modern thinking about browser cookies, and to the extent that it makes sense, ditches historical legacy no longer relevant", written while the owner was away, with the decisions it needed made and listed in section 13 (decision 62). Built the same day, on the owner's "when all tasks complete, fully implement cookie specification", with section 13's items as recommended and item 7's libraries chosen; section 15 lists what building settled, for the owner to confirm. |
| **Security** | [`cookies-security.md`](cookies-security.md), whose findings this design takes. |
| **Requirements** | Section 21 of [`requirements.md`](requirements.md), with their states; requirements 7.4 and 7.5 are met by it. Section 12 below is the text as proposed. |
| **Builds on** | The engine's cookie jar (`neoteletext-web` `cookie.rs`; its REQ-WEB-006, REQ-WEB-026, REQ-SEC-013 and decision E.35), channels as the boundary (requirement 7.1), the trusted core behind the broker (decision 56), history's clearing ranges and private tabs (`history-and-bookmarks.md` sections 2.5 and 2.7), the settings panel (`settings-home-console-proxy.md` section 1.3), session restore (decision 59). |

## 1. What is wanted, and what is not

**What a user of a current browser expects:** signing in to a channel and staying signed in across tabs and restarts; seeing which channels keep cookies and how much; deleting them all, or one channel's, or one site's; deleting them with history for a time range; having them deleted when the browser closes; blocking a channel; private tabs that keep nothing. What a channel's author expects: RFC 6265bis behaviour, so a server written for browsers works unchanged.

**What the most modern thinking adds, and this design takes:**

| Practice | Where it is today | Here |
|---|---|---|
| RFC 6265bis instead of RFC 6265: `SameSite` with `Lax` the default, the 400-day cap, size limits, name prefixes, plain HTTP never touching `Secure` cookies | All major browsers; the IETF draft (latest checked 30 September 2026, the engine's decision E.35) | Taken whole, by the engine's jar |
| The `__Http-` and `__Host-Http-` prefixes | Chrome 140, Firefox 143 (2025) | Taken, by the engine's jar |
| Cookies bound to their origin's scheme and port ("origin-bound cookies") | A Chrome feature with an IETF draft of October 2025, reported enabled by default from Chrome 148 (unconfirmed) | Taken, and stricter: a jar belongs to a channel, whose identity includes the scheme, the port and the folder |
| No third-party cookies; storage partitioned by the top-level site | Safari, Firefox and Brave by default; Chrome kept third-party cookies (April 2025) | Taken, absolutely: a NeoTeletext page cannot embed anything, so there is no third party at all |
| Cookies encrypted at rest with a key the operating system guards | Chrome on every platform (Keychain, DPAPI, App-Bound Encryption on Windows since Chrome 127, the Secret Service on Linux) | Taken: a cookie file is written only encrypted, with a key in the platform's secret store (section 7) |
| `Clear-Site-Data: "cookies"` | Chrome, Firefox, Safari | Taken, for the channel's own jar only (section 8) |
| Device Bound Session Credentials: a session bound to a key that cannot leave the device | Chrome 146 on Windows (April 2026); macOS (Secure Enclave) announced | Not in this design; the engine's REQ-WEB-017 is its Phase 2, and the broker is where the key would be kept (section 14) |

**Legacy this design drops:**

| Legacy | Why it existed | Why it goes |
|---|---|---|
| The `Domain` attribute and cookies shared across subdomains | Sites spread across hosts | The engine ignores it (REQ-WEB-006): every cookie is host-only, and then channel-only (REQ-WEB-026) |
| The Public Suffix List | To stop `Domain=.co.uk` | No `Domain`, so no list to ship and keep current (section 1.1) |
| Third-party cookies, `SameSite=None` for embedded content, `Partitioned` (CHIPS) | Embedded content | Nothing is embedded; `Partitioned` is accepted and changes nothing, `None` matters only to a form sent to another channel |
| Cookies shared between `http` and `https`, and between ports | RFC 6265's host-only rule ignores scheme and port | A channel is its scheme, host, port and folder (requirement 7.1) |
| `Set-Cookie2`, `Cookie2`, `$Version`, `$Path` | RFC 2965, obsolete since 2011 | Ignored |
| Session cookies surviving a restart | Browsers that restore a session restore its session cookies | Never written (section 6): a session cookie is the most valuable thing to steal and the least meant to last |
| Script access (`document.cookie`, the Cookie Store API) | Scripts | There is no script engine (requirement 1.3); every cookie is in effect `HttpOnly` |
| Unbounded or per-domain-only limits | History | Bounds per channel and in total, as section 5 says |

### 1.1 The Public Suffix List, and what leaving it out costs

*Written 23 October 2026, at the owner's question, to record decision 62's reasoning in full.*

**What it is.** A list, kept by Mozilla and shipped by every major browser, of the parts of a name under which anyone may register a site: `com`, `org`, `co.uk`, `com.au`, `github.io` and some ten thousand more. Rules cannot work it out: `co.uk` is open to anyone, while `bbc.co.uk` is one organisation's. The part one step below a public suffix is the **registrable domain** (`example.co.uk` for `news.example.co.uk`).

**What browsers use it for.**
- **Cookies, first.** A site may set `Domain=example.co.uk` to share a cookie across its hosts. Without the list it could set `Domain=co.uk`, and the cookie would go to every British site: tracking across all of them, or cookies planted on them. The list is how a browser refuses `co.uk` and allows `example.co.uk`.
- **Grouping by site.** The registrable domain is the "site" of `SameSite`, of storage partitioning, of the part of the address Chrome emphasises, of `Clear-Site-Data`'s reach, and of the chapter stops of Chrome's Back and Forward menus.

**Why this design leaves it out** (decision 62):
1. Every cookie belongs to the one host that set it: the engine ignores `Domain` (REQ-WEB-006) and narrows a cookie further to its channel (REQ-WEB-026). Nothing can widen a cookie, so there is nothing to check against the list.
2. A page cannot embed anything from another channel, so there is no third party for the list to tell apart.
3. A list is data to ship, keep current and check; an old copy gives wrong answers. Leaving out what earns nothing is the point of this design.

**What it costs.**
- Channels on several hosts of one organisation, `news.example.co.uk` and `sport.example.co.uk`, cannot share a sign-in cookie: each signs in alone. Stricter than any browser, and accepted for privacy.
- Where Chrome groups by registrable domain, this browser groups by host or by channel: `Clear-Site-Data` reaches the channel alone (sections 8 and 13 item 8); the address field emphasises the host (requirement 11.3); the chapter stops of the Back and Forward lists change at each host (`history-and-bookmarks.md` section 16, decision 64), so those two hosts are one chapter in Chrome and two here.
- If grouping by organisation is wanted later, the list would come back for that alone, as data the trusted core reads; no cookie rule would change.

## 2. Where cookies live

**The trusted core owns every cookie** (requirement 7.5). It holds one jar per channel, the engine's `CookieJar`, with the engine's rules unchanged. Tabs on the same channel share its jar (requirement 7.4, decision 11), whether or not they share a fetcher or a decoder.

**A fetcher holds no cookie.** For each request it is about to send, it asks the trusted core for that request's `Cookie` header, giving the address and the kind of request (a page or file of its channel, the push connection, a form submission and its target, a top-level navigation into the channel). The trusted core answers with the header the engine's `header_for` builds, or none. Each `Set-Cookie` line a response carries goes back to the trusted core as received, bounded, with the address it answered and whether the connection was encrypted; the trusted core stores it with the engine's `store`, under the engine's rules. So a fetcher compromised by its channel's server sees only the cookies of the requests it is sending at that moment, and can store a cookie only as the server could have.

**An engine change.** The engine's user agent keeps its jar inside itself today. It is to take its cookies through its platform interface instead: a `Cookies` trait with `header_for(request)` and `set(response, line)`, which the engine's own jar implements for ports that want it inside, and the fetcher implements by asking the trusted core. This is the engine's change to make, by its own process, and the pin moved here (`DEVELOPMENT.md` section 11; `architecture.md` section 5.1 already lists it).

**Round trip.** The question costs one message each way on a pipe the fetcher already has: tens of microseconds against a network request of milliseconds. The push connection asks once, when it opens.

**A file channel has no cookies** (requirement 7.8). **The start page** has none.

## 3. Private tabs

Private tabs share **one private cookie session**: a jar per channel, separate from the ordinary ones, kept in memory only, emptied when the last private tab closes, never written, never listed with the ordinary cookies, never cleared into or out of them. A channel open in a private tab and an ordinary tab is signed in separately in each, as in Chrome and Firefox.

## 4. What the user sees and does

### 4.1 The cookies panel

The developer console's Cookies tab (decision 63, 23 October 2026), opened there or by the settings panel's "Cookies: Manage…", which opens the console at it; until then it was a panel over the window, opened from the settings and from the menu's "Cookies and site data", which is removed. The trusted core sends its rows only while the tab is shown with the console open. It lists, for the ordinary session:

- **By site**: each origin with cookies (`https://news.example`), its total count and size, and when it was last used; under it, each channel of that origin (its folder), with the same.
- **Per channel**: Delete; Block.
- **Per site**: Delete all for this site, which deletes every channel's jar on that origin.
- **At the top**: a search field over the identities, the count and size in all with the trusted core's word, and "Delete all cookies".

**The panel never shows a cookie's name or value.** Requirement 3.6 keeps cookies from the interface; names often carry identifiers, and values are credentials. A developer who needs them uses the channel's server, as with the console (requirement 19.8). The panel's rows are views with view numbers, as the history panel's are (`history-and-bookmarks.md` section 4): the interface can ask for a row's deletion by its number and nothing else.

### 4.2 The settings

| Setting | Values | Default | Kept as |
|---|---|---|---|
| Cookies | Keep · Delete when the browser closes · Block all | Keep | `cookies keep|close|block` |
| Blocked channels | A list of channel identities, at most 1 000 | None | `cookie-block <channel>` lines |

- **Keep:** persistent cookies are kept between runs, as section 6 says.
- **Delete when the browser closes:** every jar is emptied at exit and none is written; a crash leaves nothing either, since nothing was written (section 6).
- **Block all:** no cookie is stored or sent. A channel that needs a session will not keep one; the strip says so the first time a channel sets a cookie that is refused ("This channel's cookies are blocked").
- **A blocked channel** is treated as Block all, for that channel alone.

These are privacy settings, not security settings: a change is not said in the strip (as restoring the session, requirement 20.24), and each is strip input checked by the trusted core.

### 4.3 Clearing with history

The history panel's "Clear" (requirement 20.6) gains a choice beside its time range: **History**, **Cookies**, or both, history alone by default. Cookies in a range are those created within it. This is Chrome's and Firefox's "Clear browsing data".

## 5. Bounds

| | Bound | Source |
|---|---|---|
| A cookie's name and value | 4 096 bytes together; an attribute's value 1 024 | RFC 6265bis, the engine's REQ-WEB-006 |
| A cookie's lifetime | 400 days | RFC 6265bis |
| Cookies a channel keeps | 180 | Chrome's per-site limit |
| Cookies in all | 3 000 | Chrome's total |
| Channels with cookies | 3 000 (a jar is made only when its first cookie is stored) | Follows from the total |
| `Set-Cookie` lines from a fetcher | Within the engine's bounds on a response's headers: 16 KiB together (`DEFAULT_HEADER_LIMIT`), 64 headers (`MAX_RESPONSE_HEADERS`) | The engine's HTTP reader (its REQ-SEC-011) |
| The cookies file | 32 MiB | 3 000 cookies at the largest, encoded, with room |

Beyond a bound the least recently used cookie goes first, expired ones before all (the engine's rule). The engine's jar is given its limits with `set_limits`; the browser's constants name the engine's where they are the same (requirement 2.4).

## 6. Kept between runs

- **Only persistent cookies are written:** those with `Max-Age` or `Expires`, unexpired. **Session cookies are never written** (the engine's REQ-SEC-013), even when the session is restored: a restored tab finds its channel's persistent cookies and none of its session cookies.
- **Written whole after each change**, debounced to one write a second at most, as the session is (section 15.2 of `history-and-bookmarks.md`), to `cookies.bin` in the settings folder, readable by the user alone (0600), with a temporary file renamed over it.
- **Encrypted** (section 7). Without the key, or when the platform's secret store refuses, **nothing is written** and the browser keeps cookies for the run only, saying so once in the settings panel. It never writes them in the clear (decision 58: no weaker mode).
- **Read strictly at start:** a version, every field within its bound and rule, each cookie passed through the engine's rules again as if newly set over the connection it was set on; what breaks a rule is dropped and logged; a fuzz target, `cookie_file`. The engine's `export` and `import` are the format inside the encryption.
- **Private cookies, file channels and blocked channels** are never written.

## 7. Encryption at rest

The file is sealed with an authenticated cipher (XChaCha20-Poly1305 or AES-256-GCM, one of them, per the library chosen) under a 256-bit key that the browser makes once and keeps in the platform's secret store, read through the broker (the trusted core is confined and reaches no secret store itself):

| Platform | Secret store |
|---|---|
| macOS | The login Keychain, an item readable by the browser's signed identity alone |
| Windows | DPAPI (`CryptProtectData`), bound to the user; App-Bound Encryption as Chrome's needs an elevated service and is not proposed |
| Linux | The Secret Service (GNOME Keyring, KWallet) over D-Bus; where none runs, the kernel's persistent user keyring |

**What it protects against:** a copy of the file alone (a backup, another user who can read the folder by mistake, a disk read offline). **What it does not:** a program running as the user, which can ask the secret store as the browser does; no browser can stop that without the elevated service Chrome uses on Windows, and even that is reported bypassed. This is what every major browser offers, and why section 6 never writes session cookies.

**The libraries are the owner's to choose** (the baseline instructions: library choice is an architectural decision): the cipher, and a crate for each secret store or our own calls through the `windows` crate already in the tree and `security-framework` on macOS. Section 13 item 7 recommends.

## 8. `Clear-Site-Data`

A response from the channel over `https` with `Clear-Site-Data` naming `"cookies"` or `"*"` empties **that channel's jar**, and nothing else: not the origin's other channels, which are other people on a shared host (REQ-WEB-026), and not over plain HTTP, where a network attacker could send it. The fetcher passes the header to the trusted core as it passes `Set-Cookie`. `"cache"` and `"storage"` have nothing to clear yet beyond what the engine's cache already discards.

## 9. Messages

| From → to | Message | Carries |
|---|---|---|
| Fetcher → core | Cookie question | A request number, the address, the request's kind, whether encrypted |
| Core → fetcher | Cookie answer | The request number, the `Cookie` header or none |
| Fetcher → core | Cookies set | The address, whether encrypted, the `Set-Cookie` lines, within the engine's bounds on a response's headers |
| Fetcher → core | Site data cleared | The address, the directives |
| Core → interface | Cookies view | Rows: view number, identity, count, size, last used, blocked |
| Interface → core | Cookies request | Open, search, delete by view number, delete a site by view number, block or unblock by view number, delete all |

The interface's requests act only by view numbers, refreshed with every view, as history's (requirement 20.5); "delete all" is strip input like any other.

## 10. The trusted core's confinement

Nothing new: the cookie file is in the settings folder, which the trusted core already writes. The key comes from the broker, by a new request, `CookieKey`, answered with the key or a refusal; the broker reads the secret store, which needs its own access the trusted core does not have (on Linux, the session bus; on macOS, the Keychain). The key never leaves the trusted core; it is wiped from memory at exit.

## 11. Tests and checks this design asks for

Every rule of the engine's REQ-WEB-006 already has the engine's tests. This design adds: a fetcher holding no jar (requirement 7.5's test); two tabs sharing a channel's cookies, and a private tab not; each setting; each deletion; clearing with history by range; the file round trip, encrypted, unreadable without the key, never written in the clear when the key is refused, and never holding a session or private cookie; `Clear-Site-Data` from the channel over `https` alone; the fuzz targets `cookie_file`, `cookie_messages` (what a fetcher sends) and `cookie_requests` (what the interface sends); a review that no message to the interface carries a cookie's name or value.

## 12. Proposed requirements

### 21. Cookies

**Requirement 21.1: The trusted core owns the cookies.**
- *Statement:* Every cookie must be kept by the trusted core, one jar per channel under the engine's rules (REQ-WEB-006, REQ-WEB-026); a fetcher must hold none, receiving for each request only that request's `Cookie` header and returning each `Set-Cookie` line to the trusted core.
- *Reason:* A fetcher compromised by its channel's server then sees only the cookies of requests in flight (requirement 7.5).
- *Verification:* Test: the fetcher has no jar; a request carries the trusted core's header; a `Set-Cookie` line reaches the trusted core's jar.

**Requirement 21.2: Tabs on a channel share its cookies.**
- *Statement:* Every ordinary tab showing a channel must use the same jar.
- *Reason:* Decision 11, requirement 7.4.
- *Verification:* Test with two tabs.

**Requirement 21.3: Private tabs have their own cookies.**
- *Statement:* Private tabs must share one jar per channel, separate from the ordinary jars, held in memory only and emptied when the last private tab closes.
- *Reason:* What a private window means in every browser.
- *Verification:* Test.

**Requirement 21.4: Only persistent cookies are kept, and only encrypted.**
- *Statement:* Between runs only unexpired cookies with a lifetime may be kept, never session cookies, private cookies or cookies of a blocked channel, in `cookies.bin`, readable by the user alone, sealed under a key kept in the platform's secret store; when the key cannot be had, nothing is written.
- *Reason:* A cookie file is a set of credentials; decision 58 forbids a weaker mode.
- *Verification:* Tests: round trip; a file unreadable without the key; no file written when the key is refused; no session cookie in the file.

**Requirement 21.5: The cookie file is read strictly.**
- *Statement:* `cookies.bin` must be read with every field within its bound, each cookie passed through the engine's rules again; what breaks a rule is dropped and logged.
- *Reason:* The file is outside input once decrypted, since another program of the user can replace it with the key.
- *Verification:* Test of each rule; the fuzz target `cookie_file`.

**Requirement 21.6: The cookies panel.**
- *Statement:* A panel must list the channels and sites with cookies, with their counts, sizes and last use, never a cookie's name or value, and must delete a channel's cookies, a site's, or all, and block or unblock a channel, by view numbers alone.
- *Reason:* The control every browser gives, without giving the interface any cookie (requirement 3.6).
- *Verification:* Interface test; a test that no view carries a name or value.

**Requirement 21.7: The cookie settings.**
- *Statement:* The settings must offer Keep, Delete when the browser closes, and Block all, Keep by default, and a list of blocked channels, kept in `settings.txt`.
- *Reason:* As in every browser.
- *Verification:* Test of each.

**Requirement 21.8: Cookies are cleared with history.**
- *Statement:* The history panel's clearing must offer cookies, alone or with history, for its time ranges, by the cookies' creation time.
- *Reason:* Browsers' "Clear browsing data".
- *Verification:* Test.

**Requirement 21.9: `Clear-Site-Data` empties the channel's own jar.**
- *Statement:* A response from the channel over `https` naming `"cookies"` or `"*"` in `Clear-Site-Data` must empty that channel's jar and no other; over plain HTTP it must be ignored.
- *Reason:* The current way for a server to sign a user out everywhere; limited to the channel by REQ-WEB-026.
- *Verification:* Test.

**Requirement 21.10: Cookies are bounded.**
- *Statement:* At most 180 cookies a channel and 3 000 in all, each within the engine's sizes and 400 days, the file within 32 MiB, and the `Set-Cookie` lines from a fetcher within the engine's bounds on one response's headers (16 KiB, 64 headers).
- *Reason:* Bounded memory and input (the baseline instructions).
- *Verification:* Test at each bound.

**Requirement 21.11: Legacy cookie mechanisms are not supported.**
- *Statement:* `Set-Cookie2`, `Cookie2` and `$`-prefixed attributes must be ignored, `Domain` must not widen a cookie (the engine's host-only rule), and no cookie may be shared across schemes, ports or channels.
- *Reason:* Section 1's table.
- *Verification:* Test.

### Amendments on adoption

- **7.4 and 7.5:** met by requirements 21.1 and 21.2 once built.
- **19.1:** `settings.txt` adds `cookies keep|close|block` and `cookie-block` lines.
- **20.6:** clearing offers cookies (requirement 21.8).

## 13. For the owner: decision 62

Decided while the owner was away, on the owner's instruction to make the decisions needed; each is the recommendation, to confirm or change.

1. **The trusted core owns the cookies; fetchers ask per request** (section 2), meeting requirements 7.4 and 7.5. The alternative, a jar per fetcher as today, leaves 7.5 unmet.
2. **The engine takes its cookies through its platform interface** (section 2): the engine's change, by its own process.
3. **Session cookies are never kept**, even when the session is restored (section 6). Chrome and Firefox keep them when they restore a session; this design does not, as the engine's REQ-SEC-013 already says.
4. **Persistence only encrypted, never in the clear** (sections 6 and 7), as decision 58 asks.
5. **The panel shows no cookie names or values** (section 4.1), as requirement 3.6 asks.
6. **Chrome's limits**, 180 a channel and 3 000 in all (section 5), not the engine's small-device defaults of 50 and 100.
7. **Libraries for encryption at rest.** The recommendation was RustCrypto's `chacha20poly1305` and a crate for each store. Chosen in building: **`ring`** for the cipher (ChaCha20-Poly1305) and the system's random source, already in the tree under rustls, so no new cryptography; and **`keyring` 4.2.0** (MIT or Apache-2.0) for the stores: the macOS Keychain, the Windows Credential Manager (which DPAPI protects) and the Secret Service over a pure-Rust D-Bus client (`zbus`). The kernel keyring is not used on Linux: it does not survive a restart, so a key kept there would make the file unreadable after one. Where no Secret Service runs, cookies last for the run only, as decision 58 asks. Library choice is the owner's: this one awaits confirmation.
8. **`Clear-Site-Data` clears the channel only** (section 8), not the registrable domain the specification names, since NeoTeletext has no registrable domains.
9. **A blocked channel and "Block all"** are offered; per-channel allow lists under "Block all" are not, to keep one rule a channel.
10. **Clearing with history uses creation time**, as Chrome does.

## 14. Not in this design

- **Device Bound Session Credentials** (the engine's REQ-WEB-017, Phase 2): the key would live in the Secure Enclave, a TPM or the platform's store, reached through the broker. To be designed when the engine has it.
- **Storage other than cookies:** NeoTeletext has none (no script, no local storage, no cache that outlives a run beyond the engine's).
- **Importing cookies from another browser:** never; it would read another program's credentials.

## 15. What building settled

Built on 22 October 2026 (decision 62); each item for the owner to confirm.

1. **One jar for the ordinary tabs and one for the private, each channel under a tag.** The engine's jar keeps cookies by host; each channel's cookies are kept under a tag of its own (`c1`, `c2` …), which the jar's host rules accept, with the trusted core's table of tags to channels written into the file. The jar's per-host limit is then the per-channel limit. One jar of 3 000 cookies and 4 MiB of text is made on first need, on a thread with room for it (it is too large for a default stack while it is boxed).
2. **The engine's change** is its decision E.104 (commit `64c1a97`): `Cookies`, an interface for a store, which the agent takes from a port with `UserAgent::with_cookies`; `Clear-Site-Data`; and each cookie's times in seconds, with listing and removal, and a saved format of version 2. The fetcher gives its agent a store that asks the trusted core, and the agent's own jar holds no cookie (`CookieJar<0, 0>`).
3. **A fetcher is answered only with its own channel's cookies**, for requests on its channel's host and scheme within its folder. A form sent to another origin therefore carries no cookie at all: stricter than REQ-WEB-006's rule, which would send that origin's `SameSite=None` cookies, since that origin's cookies belong to its own channels. A cookie set by a response from another host is refused. A push connection's cookies are kept in the channel's folder.
4. **Whether a host was reached securely** is answered from every `https` channel of that host, the engine's REQ-SEC-007, so a plain-HTTP channel of a host whose `https` channel set a cookie is refused as a downgrade. That is one bit about another channel, given to protect this one.
5. **The key is asked for at start**, when cookies are kept. On macOS a build not signed with a stable identity, as a development build, may be asked by the Keychain whether it may read the key after each rebuild, and the browser waits for the answer; a signed release is not asked (`PENDINGTASKSFORANDREW.md` item 1).
6. **"On close" removes the file at start and at exit; "Block all" leaves it and reads nothing**, so turning Keep on again restores the cookies kept before.
7. **A refused cookie is noted in the developer console without its name or value**; a blocked channel's first refused cookie is said in the strip once.
8. **The panel lists a blocked channel with no cookies**, so it can be unblocked there.
9. **Clearing with history** offers History and Cookies as two switches beside the ranges, History on as before.
10. **The engine's `cookies` feature was off in the browser** (its `neoteletext-web` was built with `push` alone), so the agent had never kept or sent a cookie: a real run found it, as `CookieRejected(NotCompiledIn)`. It is on, and `the_agent_asks_for_each_requests_cookies` fails without it.
11. **On Windows a fetcher cannot reach this computer's own loopback addresses**: its AppContainer forbids it, and Windows grants that only to a package by an exemption the browser does not ask for. A channel served on `localhost` opens on macOS and Linux but not on Windows; one on the local network opens with `--allow-local`.
12. **Fuzzing found a fault before release:** a store that never kept a cookie wrote a file it could not read back (no empty line after its channels). Fixed, with a test and the input kept in `native/fuzz/regressions/cookie_file/`.

