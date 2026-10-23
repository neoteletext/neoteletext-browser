# NeoTeletext: security analysis of hyperlinks

| | |
|---|---|
| **Status** | Written 20 October 2026 against [`hyperlinks.md`](hyperlinks.md) as proposed. The spec was corrected from this analysis (its section 18 lists the corrections), and built the same day; its section 19 lists what building changed, none of which weakens a property analysed here. Section 15's checks are in the gates. |
| **Method** | The method of [`security-analysis.md`](security-analysis.md): what is protected, who attacks, each trust boundary the feature crosses, the mitigations, and the remaining risk ranked. |
| **Scope** | Links found in page text, links a page declares, links a descriptor declares, following links within and between channels, and handing a link to the system's web browser. Fastext shares the link table and is covered where it differs. |

---

## Contents

1. Summary
2. What is protected
3. Who attacks, and how
4. The boundaries a link crosses
5. Boundary A: the page into the decoder
6. Boundary B: the decoder into the trusted core
7. Boundary C: the trusted core into the interface
8. Boundary D: the interface back into the trusted core
9. Boundary E: the trusted core into the operating system
10. Spoofing
11. Privacy
12. Availability
13. Findings against the proposed spec
14. Remaining risks, ranked
15. Checks that keep these properties

---

## 1. Summary

Hyperlinks add two things the browser has never had: **navigation that a page starts,** and **a way out of the browser** into the system's web browser. Both are new attack surface in the trusted core, which until now navigated only on what the user typed.

The design holds up because of three properties, and the analysis shows they are sufficient when the corrections of section 13 are made:

1. **A page supplies positions and targets, never pixels or prompts.** Everything the user sees that says "link" or "this leads to X" is drawn by the browser from data the trusted core checked.
2. **Nothing leaves without the user acting in the strip.** Within a channel, a link is as safe as typing a page number. Leaving a channel, and leaving teletext, each need a confirmation drawn where no page can reach and accepted from no input a page can produce.
3. **The hand-off to the operating system is a single, parsed, re-serialised `https` or `http` address, passed as one argument by the trusted core.** The operating system is never asked to open anything else.

The largest remaining risk is one the architecture already carries: a **compromised interface process** can forge strip input. With hyperlinks that forgery can open an address in the system's web browser, which makes NeoTeletext a possible delivery route for an exploit against *that* browser. Section 14 ranks it and section 9 bounds it.

## 2. What is protected

In addition to `security-analysis.md` section 2:

- **The user's understanding of where a click leads.** A link must never take the user somewhere other than where the browser said it would.
- **The boundary of the channel.** No page may move a tab to another channel, or another origin, without the site-change screen.
- **The boundary of the browser.** No page may cause anything to run or open outside NeoTeletext without the user's confirmation in the strip.
- **The trusted core's integrity.** Link data is page data; parsing it in the core must not add a way to crash or confuse the core.
- **The user's attention.** Prompts must not be forced, flooded or timed to catch a click meant for something else.

## 3. Who attacks, and how

- **A page author** (the network attacker of `security-analysis.md` section 3), who controls every byte of a channel: its pages, its TTI link lines, its descriptor, its sub-page carousel and its timing.
- **A compromised decoder,** which can send any `Links` message, for any page of its channel, at any rate.
- **A compromised interface process,** which can send any intent or strip input and draw anything in the window.
- **A compromised fetcher,** which can alter a descriptor's services and aliases before the core sees them; it is equivalent to a page author for this feature.
- **A local attacker** with a user account, who can replace the system's web browser or the `$BROWSER` variable; out of scope, as `security-analysis.md` section 10 says.

What each wants:
- to make the user visit a channel or a web address they did not choose (phishing, drive-by delivery of a web-browser exploit, tracking);
- to make the user open a local file or run a program;
- to learn something: that the user looked at a link, or what else they have open;
- to crash or wedge the browser.

## 4. The boundaries a link crosses

```
A           B              C               D               E
page ─────▶ decoder ─────▶ trusted core ─▶ interface ────▶ trusted core ─▶ operating system
 text,       finds           checks,         draws,          cell, point,     opener program,
 LT/LK/LN,   candidates      classifies,     shows strip     strip input      default browser
 descriptor                  keeps per frame
```

Boundaries B, C and D exist today for frames and clicks; links add new message kinds across each. Boundary E is entirely new.

## 5. Boundary A: the page into the decoder

**What crosses.** TTI `LT`, `LK` and `LN` lines (parsed by the engine in `neoteletext-formats`), X/27/0 Fastext packets, the decoded grid (for page numbers found in text), and the descriptor's `services` and `aliases` (parsed by the engine in `neoteletext-web`, in the fetcher).

**Threats and mitigations.**

| Threat | Mitigation |
|---|---|
| Malformed link lines crash the parser | The engine's parsers are bounded, panic-free and fuzzed (its REQ-SEC-016); the decoder is a process with no rights (requirement 3.4), so a crash loses one channel's pictures and nothing else |
| More links than the engine holds | The engine keeps at most eight `LT` and eight `LK` per page and reports the rest |
| A target over 256 bytes | Refused by the engine |
| A page remaps a visible number (`LK,140,…`) to another page | Within the channel, harmless: the same author controls both pages. To another channel: the site-change screen (section 8) |
| **A descriptor's `aliases` remap a typed number to another service or address** | Up to 16; these are links and must go through the same table and the same screens as `LK`. The proposed spec had missed them (finding 2) |
| **A descriptor's named `services` lead to other descriptors, at other origins** | A `name:mpp` target is *another channel*, not "this channel, another service" as the spec said (finding 1); it needs the site-change screen |

**Remaining risk:** low. The decoder cannot act on a link; it can only describe one.

## 6. Boundary B: the decoder into the trusted core

**What crosses.** `Links { key, version, links }`: up to 32 runs per page, each a position and a target string of at most 256 bytes.

**Threats and mitigations.**

| Threat | Mitigation |
|---|---|
| Links for a page or channel this decoder does not own | The core checks the key against the decoder's channel, as it checks frames (requirement 4.4) |
| Links for a version the core never sent, or an old one | Ignored; the core keeps the last four versions of each page on screen |
| Runs outside rows 1 to 23, beyond 40 columns, overlapping, zero or over length | Dropped, and the message counted against the decoder |
| More than 32 runs | The page has no links at all, and the core logs it: a flood shows as nothing, never as real links hidden among false ones |
| **The target grammar reaches the core from a page.** Today `core/src/address.rs` parses only what the user typed. A link target is attacker input parsed in the most trusted process | The target grammar is a separate, smaller parser than the address field's: page numbers, `name:mpp`, and an address which is then passed to the existing address rules. Both are fuzzed (`fuzz/` has an `address` target today; a `link_target` target is added), and neither may panic (`#![deny(clippy::panic, …)]` as the readers already are). This is the one place this feature adds parsing to the core, within requirement 3.2's spirit: a grammar of a few tokens, not a format reader |
| A decoder sends `Links` every frame to burn the core's time | Bounded by the frame rate the core already accepts; each check is a few hundred comparisons |
| Memory | Four versions × 32 runs × 260 bytes × pages on screen: under 1 MB for a full wall |

**Remaining risk:** low, provided the target parser is fuzzed and panic-free.

## 7. Boundary C: the trusted core into the interface

**What crosses.** `LinkRuns { key, version, runs }` with positions and a kind (this channel, another channel, the web): no targets, no addresses, no labels. The strip text for the pointed link's destination. The site-change screen and the web-browser prompt, as strip state.

**Threats and mitigations.**

| Threat | Mitigation |
|---|---|
| The interface learns channel data it does not need | It receives only positions and a kind (requirement 7.9) |
| Destination text makes the strip misleading | Cleaned as all untrusted text is (requirement 11.5); hosts shown by the address rules with the host emphasised; internationalised hosts refused until shown safely (requirement 11.3), which for the web hand-off means **no homograph host can be handed over at all** today |
| A very long address in the strip | Cut to the strip's limit, host first; the full address on request |
| Blue lines where no text is: `LT` runs over blank cells used as decoration or to draw shapes | Runs are trimmed to their shown, non-blank cells before drawing; a run with no shown cell is not drawn (finding 7). The total of underlined cells per page is bounded (eight `LT` × 40 plus 24 numbers × 3, about 400 cells) |

**Remaining risk:** low.

## 8. Boundary D: the interface back into the trusted core

**What crosses.** `Point { key, version, column, row }`, `Cell { key, version, column, row }`, and strip input: Confirm, Cancel and Open.

This is the boundary where **timing** attacks live, and the one where a compromised interface is most dangerous.

### 8.1 Honest interface, hostile page

| Threat | Mitigation |
|---|---|
| **Bait and switch:** the strip shows "Page 302", the page swaps the link's target in the next frame, the user clicks | The click carries its frame's version; the core resolves it against that version's links. And the core accepts a click only when the resolved target **equals the target the strip is showing** (finding 3), and that target has been unchanged for 500 ms |
| **Stale strip:** a new frame arrives, the interface draws it, the strip still names the old frame's destination, the click carries the new version | The core keeps the last `Point` per tab and re-resolves it whenever a `Links` message arrives for that page, updating the strip; so the strip names the destination of the frame on screen within one message round-trip. The equality rule above closes the gap that remains |
| **Sub-page carousel:** a link stays the same while the text under it changes ("Weather" becomes "Cancel") | Harmless: nothing in the page area confirms anything. Within the channel the destination is the same author's; leaving the channel is confirmed in the strip |
| **Clickjacking the prompt:** a game makes the user click repeatedly at one spot, and a prompt's Open button appears there | The prompt is in the strip, outside the page area, in a fixed place; repeated clicks in the page area never land in it. The prompt is also ignored for 500 ms after it appears, and ignores a button that was already down |
| **Keyboard capture:** the user is pressing Enter for something else when the prompt appears | **The prompt never takes keyboard focus by itself** (finding 5). Enter confirms only after the user moved focus into the prompt with Tab or a click, and not within 500 ms of its appearing. A held key is ignored |
| **Flooding prompts:** a page with 32 web links and a carousel | One prompt per window at a time; a new one replaces, never stacks; a prompt is raised only by a click or Enter on a link, never by a frame; at most one hand-off per two seconds per window (finding 8) |
| A background tab's page raises a prompt | Only the selected tab's open page has links that can be followed; a `Cell` or `Point` naming another tab's page is ignored |
| Confirmation from pasted input, drag, or a synthetic event | The strip accepts Confirm and Open only from a click or Enter as interface events of their own kind; paste goes to the address field's editor, never to a button (requirement 8.5) |

### 8.2 Compromised interface

A compromised interface can send `Cell` and `Point` for any cell, and strip input for any button, with any timing. The 500 ms rules, the focus rules and the equality rule do nothing against it, because it is the one reporting the events.

What it can achieve:
- **Follow any link on any page of the selected tab,** including links to other channels and to the web, by sending the clicks and confirmations itself. It cannot invent links: the targets come from the core's table, from pages the user chose to open. But any page the user visits can carry a web link, so in effect **a compromised interface plus any channel of the attacker's choosing can open any `https` address in the system's web browser,** silently.
- It cannot open a `file:` address, a custom scheme, a path or an unparsable address: boundary E refuses them regardless of how the request arrived.
- It cannot change what is fetched (requirement 4.7) or read a cookie (requirement 4.5).

This is the existing residual of `security-analysis.md` section 5.6 and open decision 3, with a higher consequence than before: previously a forged strip input could at most change channel; now it can hand an address to another program. Mitigations that reduce the consequence without a second process:
- the two-second limit, so a forgery cannot spray addresses;
- a hand-off only while the window is focused and the tab selected, which a forgery can satisfy but which stops a hidden background spray;
- **every hand-off is logged** with the page key, the target and the time, so an attack is visible afterwards;
- the trusted core's sandbox, when built, allows it to start the opener program and nothing else.

What would close it: strip input signed by something the interface does not hold, which no single-process interface can provide; or the strip drawn by a second, smaller process (open decision 3). This feature does not decide that; it raises the stakes of deciding it.

**Remaining risk:** medium, entirely from the compromised-interface case.

## 9. Boundary E: the trusted core into the operating system

**What crosses.** One address, to the platform's opener.

### 9.1 What may be handed over

- Only an address that **parsed** as `https` or `http` by the core's address rules, with a host of letters, digits, hyphens and dots, no user name, no hidden characters, a numeric port if any, and at most 2,048 bytes.
- **Re-serialised** by the core from its parsed parts: scheme, host, port, path, query. The original string is never passed on. Anything that does not round-trip (`https://a\@b`, a percent-encoded host, a host with a trailing dot that changed meaning, backslashes) is refused. This removes the **parser differential** between the core's rules and the operating system's, which is where open-redirect and host-confusion bugs in "open URL" code have historically lived.
- The fragment of section 7.3 of the spec (`#302`) is a teletext page address; it is never part of a web hand-off.
- `http` is allowed and marked not secure in the prompt. (The owner may prefer to refuse it; section 13.)

Everything else is refused in the core before the opener is considered: `file:`, `data:`, `javascript:`, `ms-settings:`, `steam:`, `zoom:`, `vscode:`, mail and telephone schemes, UNC paths, bare paths, and anything beginning with `-`. Because the serialised form always begins `https://` or `http://`, no address can be read as an option by an opener program.

### 9.2 How it is handed over

| Platform | Opener | Notes |
|---|---|---|
| macOS | `/usr/bin/open -u <address>` | `-u` opens it as an address even if it matches a file name. Spawned directly, no shell. `open` asks Launch Services for the default handler of the scheme |
| Windows | `ShellExecuteW(NULL, L"open", address, NULL, NULL, SW_SHOWNORMAL)` | For an `https` or `http` address Windows resolves the user's default browser. Needs `unsafe` or a wrapper crate; as decision 39 confined the Dock menu's `unsafe` to one crate, this is confined to an `opener` crate of its own |
| Linux | **The XDG Desktop Portal first:** `org.freedesktop.portal.OpenURI.OpenURI` over the session bus, which is the interface made for sandboxed applications to open addresses, and which Flatpak and Snap already require (finding 6). Otherwise `xdg-open <address>` | `xdg-open` is a shell script that honours `$BROWSER` and dispatches to desktop-specific openers; it has had argument-handling faults over the years. The portal takes the address as a D-Bus string with no shell anywhere |

On every platform the address is one argument; no command line is assembled from text.

### 9.3 What the opener does with it is out of our hands

The default browser may itself be vulnerable, or the user may have set an unusual handler for `https`. NeoTeletext cannot defend that, and does not try: the hand-off is only ever a consequence of the user's confirmation of a visible address, which is the same assurance a link in an email client gives.

### 9.4 Never automatic

A hand-off happens only in the handling of an Open strip input, which exists only while a prompt is shown, which is raised only by a `Cell` or an Enter on a web link of the selected tab's open page. No frame, redirect, refresh, discovery result, descriptor, alias or file drop reaches that code. This is checked by a test that drives every other message kind and asserts the opener was never started.

**Remaining risk:** low for the hand-off itself. The risk of the whole path is the compromised-interface case of section 8.2.

## 10. Spoofing

Everything in `security-analysis.md` section 7 applies. What links add:

- **A page can draw a convincing underline** with mosaics and Level 2.5 colour (spec section 4); DRCS glyphs could too until the engine stopped decoding them (its decision E.95, pinned on 21 October 2026). The design does not try to stop this; it makes the real underline *finer* than any teletext dot and makes the strip the proof. A fake underline leads nowhere: a click on it is a cell click on a non-link, and the strip shows no destination when the pointer is over it. **The user's rule is the web's: trust the strip, not the text.**
- **A page can draw a fake prompt** ("Open in your browser? [Open]") in the page area. A click on its button is a cell click; nothing opens. The real prompt appears only in the strip.
- **A page can describe one destination in its text and link another.** The strip shows the real one before the click. Within the channel this is harmless; beyond it the site-change screen names the channel, and the web prompt names the host.
- **Below two device pixels per teletext dot,** no underline is drawn at all (spec section 9.2), because nothing drawn at that size could be told from the page's own pixels. Links still work by keyboard, and the strip still names them.
- **The cursor** becomes a hand or an arrow-out-of-a-box over a link. A page cannot change the cursor. This is a small, true signal.

**Remaining risk:** medium, as before, for the same reason: visual discipline. Links give a page a new thing to imitate, and a browser a new fixed style to keep sacred.

## 11. Privacy

- **No request before consent.** Classification is from the target alone. Pointing at a link fetches nothing, resolves no name, and pre-connects to nothing.
- **No referrer** when a link is followed to another channel (requirement 5.14), and the system browser receives only the address.
- **No visited-link styling.** On the web, visited styling leaks history to scripts; here there are no scripts, so the leak does not exist, but the styling is left out anyway: it is state the interface would have to hold per channel (requirement 7.9), for a small benefit.
- **The log of hand-offs** (section 8.2) is local, bounded, and holds addresses the user chose to open.

**Remaining risk:** low.

## 12. Availability

- **Prompts cannot wedge the window:** one at a time, replaceable, cancelled by Escape, never modal to the strip's other controls.
- **A link flood** (32 runs changing every frame) costs the core bounded work per frame and the interface a few hundred line draws.
- **An opener that hangs** (a broken `xdg-open`) must not hang the core: the opener is spawned and not waited for, with its standard streams closed. On macOS and Linux it is also started in a process group of its own (24 October 2026), so that a web browser it starts, which `xdg-open` may leave as its descendant, is never in the broker's group, which the launcher kills whole after a broker crash (finding F-34); test `the_opener_is_in_a_process_group_of_its_own`.
- **A broken portal** on Linux falls back to `xdg-open`; a missing opener is reported in the strip as "Could not open your web browser".

**Remaining risk:** low.

## 13. Findings against the proposed spec

Each changed the spec; `hyperlinks.md` section 18 records the edits.

1. **Named services are other channels.** REQ-WEB-002's `services` each name "the address of its own descriptor", and the engine makes a descriptor's channel identity its origin and folder. So `name:mpp` leaves the channel and needs the site-change screen. The spec's classification table called it "this channel, another service". **Severity: high** had it been built: a page could have moved a tab to another origin with no confirmation.
2. **Descriptor aliases are links the spec missed.** REQ-WEB-002 allows up to 16 `aliases` mapping a page number to `name:mpp` or an address. They are a third link source (with `LK` and found numbers) and must enter the same table, so that typing an aliased number shows the destination and goes through the screens. A found page number that the descriptor aliases shows the alias's destination, not "Page 140".
3. **The Point-to-Cell race.** The spec resolved a click against its frame, but the strip could still be naming the previous frame's destination. The core must re-resolve the last `Point` on every `Links` for that page, and accept a click only when the resolved target equals the strip's shown target.
4. **The address parser becomes reachable from pages.** The spec passed link targets to the core's address rules without noting that these rules have only ever seen user input. A separate target parser, fuzzed, and the address fuzz target kept.
5. **The prompt must not take focus by itself.** Enter is a confirmation key; a prompt that grabs focus can be confirmed by a key meant for something else.
6. **Prefer the XDG portal over `xdg-open`.** Section 9.2.
7. **Blank runs draw lines.** Trim runs to shown, non-blank cells; bound the total.
8. **Rate-limit and log hand-offs,** and raise them only from the selected tab in a focused window, to bound the compromised-interface case.
9. **The `-` rule in number finding was inconsistent:** "140-145" would have linked 140 and not 145. Dropped: both are links, and negative page numbers do not occur.
10. **Plain `http` web links.** Allowed and marked, as the address field allows them (requirement 5.9). The owner may prefer to refuse them for the hand-off, since a plain-text address can be rewritten on the way to the other browser; the cost is small, as `http` links are rare. Left as a decision (spec section 16).

## 14. Remaining risks, ranked

| Rank | Risk | Likelihood | Consequence | What bounds it |
|---|---|---|---|---|
| 1 | A compromised interface forges clicks and confirmations and opens an `https` address in the system's web browser | Low (the interface runs no scripts and takes no network bytes; it would need a fault in our own drawing or input code) | High: a route to the user's main browser | Only parsed `https`/`http`; one per two seconds; selected tab, focused window; logged; open decision 3 would close it |
| 2 | A page imitates a link or a prompt well enough that the user does not look at the strip | Medium | Low within a channel; medium for a web link, since the real prompt still intervenes | The strip; the cursor; the hairline; user habit |
| 3 | A fault in the core's target parser | Low (small grammar, fuzzed, no panics) | High (the trusted core) | Fuzzing; `deny(panic)`; the parser's size |
| 4 | A parser differential between the core and the system browser | Low (re-serialised from parts; hosts are ASCII only) | Medium | Re-serialisation; refusing what does not round-trip |
| 5 | The default browser or `$BROWSER` is itself hostile or broken | Out of scope (local attacker) | — | — |

## 15. Checks that keep these properties

To add to the gates when the feature is built:

- **Tests of every row** of the classification table, including `name:mpp` reaching the site-change screen and a descriptor alias reaching it.
- **A "nothing else opens" test:** every message kind other than Open strip input is driven with web links present, and the opener is asserted never started.
- **Fuzz targets** `link_target` and the existing `address`, in the full gate.
- **Timing tests** with a fake clock: a click 499 ms after a target change is refused and one at 500 ms accepted; a prompt confirmed 499 ms after appearing is refused.
- **Equality test:** a click whose frame's target differs from the strip's shown target is refused.
- **Refusal tests for the hand-off:** `file:`, `data:`, `javascript:`, six custom schemes, a UNC path, a bare path, `-` prefixed, 2,049 bytes, a user name, a backslash, a percent-encoded host, an internationalised host.
- **Review** of the strip in every layout with a prompt shown, against the spoofing list (requirement 14.5).
- **A log test** that every hand-off is recorded with page key, target and time.
