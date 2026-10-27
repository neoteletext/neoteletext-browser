# NeoTeletext: hyperlinks

| | |
|---|---|
| **Status** | Adopted 20 October 2026 (the architecture's decision 41), every recommendation of section 16 taken as the decision; decision 42 confirms that the only underlining is for links and that the software draws it, never the page. Phases 1 and 2 built the same day, behind the `links` feature (section 15); section 19 lists what building them changed (decision 43). |
| **Owner's brief** | Hyperlinks shown with a blue underline in the interface, controlled by the browser; underlining must not be possible from teletext source. Links to teletext open in NeoTeletext; other links offer to open in the system's web browser. |
| **Companion documents** | [`hyperlinks-security.md`](hyperlinks-security.md) analyses this design's security; its findings are applied here (section 18). [`architecture.md`](architecture.md) (sections 9.5, 9.6 and 11), [`requirements.md`](requirements.md) (sections 4, 7, 8, 11 and 12), [`security-analysis.md`](security-analysis.md) (spoofing and the line of death). The engine's decision E.36 and its TTI Link Extension (`~/devel/neoteletext-engine/docs/spec/requirements/tti-link-extension.md`). |

---

## Contents

1. Summary
2. Goals and non-goals
3. Where things stand
4. What a page can do
5. Principles
6. Where links come from
7. What a link leads to
8. How link data flows
9. Drawing links
10. Using links
11. Following a link
12. Handing a link to the system's web browser
13. Changes to existing decisions and requirements
14. Proposed requirements
15. Phases
16. Decisions
17. Tests
18. Corrections from the security analysis
19. What building it changed

---

## 1. Summary

- **A link is drawn by the browser, over the page image, never by the page.** It is a blue hairline at screen resolution under a run of cells, with a thin contrasting edge.
- **Teletext's own underline is not drawn in the browser.** Within the browser, an underline therefore only ever means a link.
- **The underline is not the proof; the trusted strip is.** A page can still draw something that looks like an underline (section 4), as any web page can. So pointing at or focusing a link shows its destination in the trusted strip, which no page can reach.
- **The trusted core decides what is a link and where it leads.** The decoder finds candidate links, the core checks and keeps them, and the interface is told only where to draw.
- **A click follows the link the user saw,** in the frame that was on screen, not one the page swapped in at the last moment.
- **Where a link leads decides what happens:**
  - a page in the same channel opens at once;
  - another teletext channel goes through the site-change screen;
  - a web address declared as one goes, after a confirmation in the strip, to the system's web browser;
  - anything else is not a link.

## 2. Goals and non-goals

**Goals**

1. Links in page text that people recognise at a glance, drawn in one style the page cannot change.
2. No page can make text look like a browser link by its own means: not by teletext's underline attribute, and not convincingly by drawing.
3. A link's destination is always shown in the trusted strip before it is followed.
4. Leaving the channel, and leaving teletext for the web, are always the user's explicit decision in the strip.
5. Links work with the pointer and with the keyboard.

**Non-goals**

- Links in broadcast streams: no standard packet carries them (section 6.5).
- Links on pages shown small on the wall (section 10.1).
- Making any other process's link data trusted: the decoder and the page stay untrusted.

## 3. Where things stand

**The engine** (`~/devel/neoteletext-engine`):
- **Underline.** Teletext's underline is the Level 2.5 and 3.5 display attribute, set by the enhancement packets (X/26): bit 6 of `CellAttributes` (`CellAttributes::underline` in the engine's `neoteletext-core`). For G1 mosaics the same bit means "separated". The renderer draws it as the last dot line of a G0 or G2 character (the renderer's `styled_line`; REQ-RND-035). Nothing else sets it: TTI, edit.tf and the text writer have no underline syntax.
- **Links.** Decision E.36 makes the Fastext coloured keys the engine's links, with the prompts in row 24 drawn by the receiver, and rejects link markers drawn by the page. Decision E.64 added two TTI lines:
  - `LK`, a page number the page shows, linked to a target;
  - `LT`, a run of text, linked to a target. "The page draws the text itself; a receiver may show it as a link."

  The engine parses both (`neoteletext-formats`, `page.rs` and `tti.rs`): at most eight of each per page, with a target of at most 256 bytes. The engine's requirements for showing links (REQ-NAV-013 and REQ-NAV-015) moved to this repository with the browser and were withdrawn there; this document restates them.

**The browser:**
- **Drawing.** The decoder renders each page with the engine's `render_screen` into a 480 × 500 indexed image. The trusted core checks it and sends it on as a page image (an indexed PNG until decision 61). The interface draws it with `Canvas::page`, and already draws browser-owned things over pages in `Wall::draw_page`: the hover frame, the zoom outline, the close button, and the update glow.
- **Clicks.** A click is converted to a cell (`Wall::cell_at`) and sent as `ToCore::Cell { key, version, column, row }`. The trusted core decides it (`core/src/trusted_core/intents.rs`, `ToCore::Cell`) and follows a link it finds there (`core/src/trusted_core/links.rs`), by the library's link rules (`core/src/links.rs`).
- **Navigation.** Navigation exists only from the strip (requirements 4.5 and 11.1). Page-started navigations, the site-change screen and Fastext are not built (requirements 8.2, 8.4, 8.5 and 12.4).
- **Underline.** The browser has no underline code of its own.

## 4. What a page can do

A page controls every pixel of its own image. Even with the underline attribute ignored, it can still draw something that looks like an underline:
- **DRCS characters:** a page can define 12 × 10 glyphs of its own, including letters with a line under them, or a cell whose top dot line is solid, placed under the text above it. *Amended 21 October 2026:* the engine, as pinned, no longer decodes DRCS (its decision E.95), so a page that uses them shows its Level 1 characters, and this way is closed.
- **Mosaics:** the bottom sextants of G1 mosaics, and the bars and lines of the G3 smooth mosaics.
- **Colour:** at Level 2.5 and 3.5 a page chooses its own colours from 4,096, close enough to any blue.

A page can also try:
- **Bait and switch:** showing one destination when the user points, then changing the link in the next frame, just as they click.
- **Hiding a link's destination:** text that names one place while the link leads to another.
- **Flooding:** many links, or links that change every frame, to make the strip flicker or to hide a real destination among them.
- **Abusing the handoff:** links to `file:` addresses, to schemes that start programs (`ms-settings:`, `zoom:`, `steam:`), or to paths that the operating system's "open" would run.

A compromised decoder can do everything a page can, and can also invent links. Its links are therefore treated exactly like a page's.

## 5. Principles

1. **The receiver draws the link** (the principle of decision E.36): the page supplies only where a link is and where it leads.
2. **Proof lives in the strip.** The underline says "this is a link"; only the strip says where it leads, and only the strip confirms leaving.
3. **The trusted core decides.** It holds the links of every frame, checks them, resolves every click and every point, and starts every navigation.
4. **The target declares its kind.** Whether a link is teletext or the web is read from the link itself, before anything is fetched (section 7).
5. **No new trust in page data.** A link is exactly as trustworthy as the page it is on. Everything that would leave the channel is confirmed in the strip.

## 6. Where links come from

### 6.1 Page numbers in the text, found by the browser (withdrawn)

*Withdrawn on 27 October 2026 (decision 116):* the browser no longer finds page numbers in a page's text. Only what a page declares is a link: its Fastext links (X/27/0 or `LN`), its `LT` runs, and its `LK` numbers for typing. A number printed in the text is text; a page that wants it clickable declares it with an `LT` line. The rules below are kept as the record of what was built and removed.

A page number printed on a page, such as "Sport 140" or "see p302", became a link to that page in the same channel. It was found in the decoded grid, not in the image:
- three characters, the first 1 to 8, the others 0 to 9, standing alone: no letter or digit immediately before or after;
- in rows 1 to 23 (row 0 is the header, with its own page number and clock; row 24 is the Fastext row);
- in G0 text, shown, not concealed, in any size;
- **only if the page exists in the channel,** in the decoder's store or in the channel's contents. A number for a page that does not exist is just a number;
- not when it is clearly not a page number: preceded by a currency sign (£, $, €, ¥) or `#`, or followed by `%`, `:` and a digit, or `.` and a digit. A range such as "140-145" links both numbers;
- at most 24 per page.

The page's own `LK` line for the same number wins (section 6.2).

### 6.2 Links the page declares (TTI)

- **`LT,row,column,length,target`:** a run of text that is a link.
- **`LK,mpp,target`:** a page number that is a link, to a target other than that page. While the page is shown, typing that number follows the link too, as the engine specifies.

The engine already parses both. Their targets are those of the engine's TTI Link Extension (section E.1): a page or sub-page, a named service, or an address of a teletext channel or page file. One new form is proposed for links to the web (section 7.2).

### 6.3 Fastext

The coloured keys (X/27/0 and TTI `LN`) are links too. They follow the same path to the core, and their prompts are drawn by the receiver as decision E.36 requires, not underlined. Built on 23 October 2026 with requirement 12.4 (`page-controls.md`, decision 67). This document only makes sure they use the same link table and the same rules for following.

### 6.4 The channel descriptor

The descriptor already declares links (the engine's REQ-WEB-002): up to 16 **`aliases`**, each mapping a page number to `name:mpp` or to an address, and the **`services`** that `name:` targets refer to, each with the address of its own descriptor. An alias is a link exactly as an `LK` line is: typing the number, or clicking it where the page shows it, follows the alias, and the strip shows the alias's destination rather than "Page 140". Aliases enter the same link table as every other source, from the fetcher through the core's checks (section 8.2).

Declaring links in the descriptor for runs of text on pages it does not author (for example pages from a T42 feed) is not proposed.

### 6.5 Broadcast streams

No standard packet carries hyperlinks. X/27's designation codes 8 to 15 are reserved, and taking them would conflict with the standard's future use. Broadcast pages keep only their Fastext links and the page numbers found in their text (section 6.1).

## 7. What a link leads to

### 7.1 Classification

The trusted core classifies each target when it accepts the link, from the target alone, with no network request:

| Target | Kind | What following it does |
|---|---|---|
| `mpp` or `mpp:ssss`, or a page file in the current channel's own folder | **This channel** | Opens the page at once |
| `name:mpp`, a service named in the descriptor's `services`, or a descriptor alias to one | **Another channel.** Each named service is its own descriptor, at its own origin and folder, which the engine makes a channel of its own | The site-change screen (requirement 8.4), naming the service's channel |
| An address of a teletext channel or page file (`http` or `https`, as the TTI Link Extension's section E.1 defines), from an `LT`, `LK` or `LN` line or a descriptor alias | **Another channel** | The site-change screen (requirement 8.4); then the channel opens as a typed address would (content decides, requirement 6.2) |
| `web:` and an `https` or `http` address (section 7.2) | **The web** | The web-browser prompt in the strip (section 12), then the system's web browser |
| Anything else: `file:`, `data:`, `javascript:`, any other scheme, a bare path, an address with a user name or hidden characters, over 256 bytes | **Not a link** | Nothing; the text is not underlined |

**Why from the target and not from what the server returns.** Asking the server to find out would contact a server the user has not agreed to visit, telling it that they looked and from where. And the strip could not say "opens in your web browser" before the click.

**When a teletext address turns out not to be teletext.** After the user has confirmed the site-change screen, the channel opens as a typed address would. If content detection refuses it as not teletext (requirement 6.3), the strip offers to open it in the system's web browser instead (section 12). The same offer is made when an address typed into the strip turns out not to be teletext.

### 7.2 A target form for the web

The TTI Link Extension's addresses are all teletext: "a channel's folder, or a page file". A channel's folder looks like any web page's address (`https://example.com/news/`), so no rule could tell the two apart. A web link must therefore say that it is one:

```
LT,5,2,12,web:https://www.example.com/story
```

This is a change to the engine's TTI Link Extension, for the engine's owner: the form `web:` followed by an `http` or `https` address, under the same rules as other addresses. The engine's page encoder writes it as null, as it already writes other addresses. Allowed in `LT` and `LN`; not in `LK`, because typing a page number should never leave teletext.

### 7.3 A page in another channel

A link can name a page in another channel by a fragment: `https://weather.example/#302`, or `#302:0001` for a sub-page. Three hexadecimal digits, 100 to 8FF, and optionally a sub-page. The strip shows the page number with the channel, and the same form works when typed into the strip or bookmarked. Today the address rules stop the host at `#` and keep nothing after it (`core/src/address.rs`); this would be its first use. A fragment is never sent in a request, and is never part of an address handed to the system's web browser.

## 8. How link data flows

```
decoder                      trusted core                         interface
-------                      ------------                         ---------
finds links in each frame    checks every link (8.2)
  Links { key, version,  ->  keeps the last frames' links  ->  LinkRuns { key, version, runs }
          links }            per page (8.3)                        draws underlines (9)
                                                              <-  Point { key, version, cell }
                             strip: destination of link   ->      (pointer over a cell)
                                                              <-  Cell { key, version, column, row }
                             resolves against that frame;
                             follows (11)
```

### 8.1 Messages

- **From the decoder: `Links { key, version, links }`.** Sent with each frame, for that frame's `version`. Each link is a run (row, first column, length, whether double height) and a target, at most 32 runs per page. It is sent only when the links changed.
- **To the interface: `LinkRuns { key, version, runs }`.** Where to draw and nothing else: row, column, length, double height, and the kind (this channel, another channel, the web), so the pointer can show what kind of link it is over. No target, no address, no label: the interface holds no channel data it does not need (requirement 7.9).
- **From the interface: `Point { key, version, column, row }`,** or none when the pointer leaves the page. It is sent at most 30 times a second. The core keeps the last point per tab and resolves it again whenever new links arrive for that page, so the strip names the destination of the frame on screen.
- **From the fetcher:** the descriptor's aliases and services, which the core turns into links of the pages they name, checked as section 8.2 requires.
- **`Cell` gains `version`:** the frame the click was made on.

### 8.2 What the core checks

Every link from the decoder is checked again, as every message is (requirement 4.4):
- the page key is one of this decoder's channel's pages, and the version is one the core sent;
- each run lies in rows 1 to 23, within 40 columns, with a length of 1 to 40, and runs do not overlap;
- the target parses as one of the kinds in section 7.1, or the link is dropped. The parser is a separate, small grammar (page numbers, `name:mpp`, `web:`, and an address handed on to the address rules), written without panicking constructs and fuzzed, because this is the first page data the core has ever parsed: the address rules have only ever seen what the user typed;
- at most 32 runs a page; beyond that the page has no links at all, and the core logs it, so that a flood shows as no links rather than as a page whose real links are hidden among false ones.

### 8.3 Links per frame

The core keeps the links of the last four versions of each page on screen. A click or a point names its version. One with an older version than that, or a version the core never sent, is ignored.

## 9. Drawing links

### 9.1 No teletext underline in the browser

The decoder renders with the underline of G0 and G2 characters turned off; separated mosaics, which share the bit, are unchanged. This needs one engine change, a render option (`RenderOptions`, beside `rounding` and `reveal`):

- **`underline: bool`**, default on, so `neottx` and every other port keep the standard's rendering (REQ-RND-035).

The browser is never patched (requirement 2.5); the option goes through the engine's own change process (architecture section 5.1). Until it exists, the decoder can clear the underline bit of every G0 and G2 cell before rendering, since the grid is the decoder's own.

**What is lost.** Underline is rare in real teletext, and its loss changes no other attribute. The browser renders Level 2.5 and 3.5 pages faithfully otherwise, and says so in its conformance notes.

### 9.2 The underline

- **Where:** in `Wall::draw_page`, straight after the page image, in the image's own rectangle. It runs under the cells of the run, along the bottom of the cell; for double height, along the bottom of the lower half.
- **Thickness:** one device pixel, or two when a teletext dot is four or more device pixels high. It is always thinner than one teletext dot as drawn, so it is a line no page can draw at that size.
- **Colour:** one fixed blue, with a one-device-pixel edge on either side in white or black, whichever contrasts more with the cell's background. It shows on every teletext background, blue ones included.
- **Smoothing:** drawn anti-aliased at screen resolution, unlike the page's nearest-neighbour pixels.
- **Steady:** it does not flash with flashing text, and does not change with the page's colour or palette.
- **Only under shown text:** a run is trimmed to its shown, non-blank cells before drawing, and a run with none is not drawn, so a page cannot draw blue lines on empty space with `LT` runs. At most about 400 cells of a page can be underlined (eight `LT` runs and 24 found numbers).
- **Concealed text:** a link in concealed cells is not drawn until the user reveals it.
- **Only when large enough:** links are drawn only when a teletext dot is at least one and a half device pixels, so that the one-pixel hairline is at most two thirds of the page's finest line (section 19, item 1).

**Focus.** A link chosen with the keyboard gets the interface's focus ring around its cells, as well as its underline (section 10.3).

**Being over a link.** The pointer becomes a hand over a link, and an arrow pointing out of a box over a web link. The underline thickens by one device pixel.

## 10. Using links

### 10.1 Where links work

Only on the page that is open: the zoomed page, or the single page at the end of the page-size control's travel. On the wall, pages are drawn small, and a click zooms a page, as now. A click on a link in a small page zooms the page and does not follow the link.

### 10.2 The pointer

- **Pointing at a link** shows its destination in the trusted strip, in the place of the status line, cleaned as all untrusted text is (requirement 11.5). For example: "Page 302", "Page 302 of weather.example (leaves this channel)", or "www.example.com/story (opens in your web browser)". Hosts are shown with the address rules, and internationalised names safely (requirement 11.3).
- **Clicking a link** follows it (section 11) only if all of these hold:
  - the click's frame version is one the core holds for that page;
  - the target resolved from that frame's links **equals the target the strip is showing** at that moment;
  - that target has been unchanged at that cell for at least 500 milliseconds, the time browsers give a permission prompt before accepting a click.

  Otherwise the click does nothing. A link that changes under the pointer, or whose frame changed after the strip last spoke, is not followed.

### 10.3 The keyboard

- **Tab and Shift-Tab** move the focus between the open page's links, in reading order; the strip shows the focused link's destination.
- **Enter** follows the focused link.
- **Escape** leaves link focus.
- **Typing a page number** works as now, except that a number the page links with `LK` follows that link.

Links are announced to screen readers through the interface's accessibility tree (requirement 12.9), with their destination as their description.

## 11. Following a link

All following is done by the trusted core, as a page-started navigation (requirement 8.2), recorded as such in the tab's history:

1. **This channel:** the page opens in the tab, and Back returns to the page the link was on.
2. **Another channel:** the site-change screen, in the strip only, names the destination channel and says it came from a link on the page (requirements 8.4 and 8.5). On confirmation the channel opens as a typed address would. On cancelling, nothing changes (requirement 8.6).
3. **The web:** the web-browser prompt (section 12).
4. **A local file's links** (requirement 7.12): links to pages of the same file open at once; links to the web or to a web channel go through the site-change screen or the web-browser prompt. A web page's links never lead to `file:` addresses (section 7.1).

## 12. Handing a link to the system's web browser

**The prompt** is drawn in the strip, never in the page area, in the place of the site-change screen. It reads "Open www.example.com/story in your web browser?", with the full address on request and two buttons, Open and Cancel. It is accepted only by a click on Open, or by Enter while the prompt has focus:
- not from the page area;
- not from a key held down before the prompt appeared;
- not within 500 milliseconds of it appearing;
- not by Enter unless the user first moved the focus into the prompt, with Tab or a click: **the prompt never takes the keyboard focus by itself,** because Enter is a confirmation key the user may be pressing for something else.

One prompt at a time per window: a second web link replaces the first prompt, and never adds to it. A prompt is raised only from the selected tab's open page, only while the window is focused, and at most one hand-off happens per two seconds per window. Every hand-off is logged with the page key, the address and the time.

**What may be handed over.** Only an `https` or `http` address that the core's address rules accept:
- parsed, not matched by prefix;
- with a host of letters, digits, hyphens and dots only, as the address rules require today (internationalised hosts are refused until they can be shown safely, requirement 11.3);
- with no user name and no hidden characters;
- at most 2,048 bytes;
- **re-serialised by the core from its parsed parts** (scheme, host, port, path, query). The original string is never passed on, and an address that does not round-trip (a backslash, a percent-encoded host, anything the parser had to guess at) is refused. This removes any difference between how the core read the address and how the operating system or the web browser will.

The scheme is always `https://` or `http://`, so the address can never begin like a command-line option. `http` is shown as not secure. Refused before the opener is considered: `file:`, `data:`, `javascript:`, mail and telephone schemes, every application scheme (`ms-settings:`, `steam:`, `zoom:`, `vscode:` and the rest), UNC and bare paths, and anything beginning with `-`.

**How it is handed over.** By the trusted core, never by the decoder or the interface. The address goes as one argument to the platform's opener, with no shell:

| Platform | Opener |
|---|---|
| macOS | `/usr/bin/open -u <address>` (`-u`: open it as a web address, never as a file) |
| Windows | `ShellExecuteW` with the `open` verb on the address, which opens it in the default browser because its scheme is `https` or `http` |
| Linux | The XDG Desktop Portal's `org.freedesktop.portal.OpenURI` over the session bus, the interface made for sandboxed applications to open addresses; `xdg-open <address>` when no portal answers. `xdg-open` is a shell script that honours `$BROWSER` and dispatches to each desktop's opener, and has had argument-handling faults; the portal takes the address as a D-Bus string with no shell anywhere |

On Windows, `ShellExecuteW` needs either unsafe code or a crate that wraps it. Which of the two, or another opener program, is for the owner (section 16).

**The sandbox.** The trusted core is not sandboxed today (requirement 3.7). When it is, starting the opener is the one program it may start.

**Never automatic.** A page, a redirect, a refresh, discovery, a descriptor, an alias or a dropped file can never open the system's web browser; only a user's confirmation in the strip can. A test drives every other message kind with web links present and asserts the opener was never started.

**The opener is not waited for.** It is started with its standard streams closed and left to run, so a broken opener cannot hang the core. A failure to start it is shown in the strip as "Could not open your web browser".

## 13. Changes to existing decisions and requirements

| Where | Change |
|---|---|
| Requirement 4.5 | "No page intent may cause a tab to navigate to another channel": a cell intent may now start a page-started navigation. That navigation reaches another channel only through the site-change screen (8.4), and the web only through the web-browser prompt (section 12). Intents still carry no address: the core takes the target from its own link table. |
| Requirement 7.12 | Local files: as section 11, item 4. |
| Requirement 8.2 | Page links become the first page-started navigation built. |
| Requirements 8.4, 8.5 and 8.6 | Built for links, as section 11. |
| Requirement 11.2 | The strip also holds the link destination and the web-browser prompt. |
| Requirement 12.3 | The core now decides with the link table, and a click carries its frame's version. |
| Requirement 12.4 | Typing a number linked by `LK` follows the link. |
| Requirement 12.9 | Links in the accessibility tree. |
| Section 6.4 of the architecture | The messages of section 8.1. |
| Engine: `RenderOptions` | An `underline` option (section 9.1). |
| Engine: TTI Link Extension | The `web:` target form (section 7.2). Showing `LT` and `LK` as links again in a receiver, which REQ-NAV-013 and REQ-NAV-015 once required. |
| Engine: decision E.36 | Unchanged in principle: the receiver draws every link marker, and the page draws none. |

## 14. Proposed requirements

To be added to `requirements.md` as section 17 once adopted.

| Number | Statement | Reason | Verification |
|---|---|---|---|
| 17.1 | Teletext's underline attribute is not drawn on any page shown by NeoTeletext; separated mosaics are drawn as before. | An underline must mean only a link. | Test: a Level 2.5 page with underlined text and separated mosaics. |
| 17.2 | Links are drawn only by the interface, over the page image, in one style the page cannot change (section 9.2), and only when a teletext dot is at least two device pixels. | A page must not be able to draw the browser's link. | Test of the drawing at several scales; review. |
| 17.3 | The trusted core decides every link, from the decoder's candidates, checked as section 8.2 requires. The interface receives only where to draw. | The decoder and the page are untrusted; the interface holds no channel data. | Test with out-of-range, overlapping, excessive and invalid links. |
| 17.4 | Page numbers in the text are links when, and only when, section 6.1's rules hold. | Navigation teletext readers already expect, without false links. | Test of each rule, including currency, times and absent pages. |
| 17.5 | Every link, from a page, a Fastext packet or the descriptor's aliases and services, is classified from its target alone, before any request, as section 7.1 requires; a named service is another channel. | No request before consent; the strip can say where a link leads. | Test of every row of the table. |
| 17.6 | Pointing at or focusing a link shows its destination in the trusted strip. | The strip, not the underline, is the proof. | Test; review. |
| 17.7 | A click follows only the link of the frame it was made on, whose target equals the one the strip is showing, unchanged for at least 500 milliseconds. | Against bait and switch. | Test: a page that changes a link's target each frame. |
| 17.8 | A link that leaves the channel goes through the site-change screen; a link to the web goes through the web-browser prompt; neither can be confirmed from the page area. | Leaving is the user's decision, made where a page cannot reach. | Test for each kind, with clicks in the page area. |
| 17.9 | Only an `https` or `http` address the core's address rules accept, re-serialised from its parsed parts, is handed to the system's web browser, by the trusted core, as one argument with no shell, at most once per two seconds, from the selected tab of a focused window, and logged. | Opening an address must never run a program. | Test with `file:`, custom schemes, paths, a leading `-` and over-long addresses. |
| 17.10 | Links can be reached and followed with the keyboard, and are in the accessibility tree. | Not everyone uses a pointer. | Test. |
| 17.11 | The web-browser prompt and the site-change screen never take the keyboard focus by themselves, and accept Enter only after the user moved the focus into them. | A confirmation key pressed for something else must not confirm. | Test. |
| 17.12 | The core's link-target parser contains no panicking constructs and is fuzzed, as the message readers are. | It is the first page data the trusted core parses. | The fuzz target `link_target` in the full gate; the lint. |

## 15. Phases

1. **Links in the channel.**
   - The engine's `underline` render option (or clearing the bit in the decoder in the meantime).
   - Page numbers found in the text (withdrawn by decision 116).
   - The messages of section 8.1, the core's checks and its table of links per frame.
   - The underline drawn, the destination in the strip, the pointer and the keyboard.
   - Following links within the channel.

   No format changes.
2. **Declared links and leaving.**
   - TTI `LT` and `LK` targets.
   - The site-change screen.
   - The `web:` form (an engine change).
   - The web-browser prompt and the handoff.
   - Links to a page in another channel (section 7.3).
3. **Fastext,** on the same link table (requirement 12.4).

## 16. Decisions

All taken by the owner on 20 October 2026 (decision 41), each as recommended.

| Number | Question | Decided |
|---|---|---|
| 1 | May a link lead to another channel, amending requirement 4.5 as section 13 says? | Yes, through the site-change screen |
| 2 | Are page numbers in the text links automatically (section 6.1)? | Yes, only for pages that exist, in phase 1. *Reversed 27 October 2026 (decision 116): no; only declared links.* |
| 3 | The exact blue, and the contrasting edge | A blue near the web's link blue, edged white or black by contrast; chosen on screen |
| 4 | Links on the small pages of the wall, or only on the open page? | Only on the open page; a click on a small page zooms it |
| 5 | The `web:` target form in the engine's TTI Link Extension | Adopt it; there is no other way to tell a channel's folder from a web page |
| 6 | The engine's `underline` render option | Adopt it; clear the bit in the decoder only until it exists |
| 7 | How the core opens the web browser on Windows: `ShellExecuteW` through a small crate, a few lines of unsafe code in a crate of its own as the Dock menu has, or `explorer.exe` with the address | A crate of its own with the unsafe confined, as decision 39 did for the Dock menu |
| 8 | Addresses of a page in another channel by a fragment (section 7.3) | Yes, `#302` and `#302:0001` |
| 9 | Hand plain `http` web links to the system browser, marked not secure, or refuse them? | Allow and mark, as the address field does (requirement 5.9); `http` links are rare enough that refusing would also be reasonable |
| 10 | On Linux, the XDG Desktop Portal first, then `xdg-open`? The portal needs a D-Bus client crate (a library choice, requirement 14.3) | Yes, through the smallest D-Bus crate that passes `cargo-deny`; `xdg-open` alone until the crate is approved |

## 17. Tests

- **Rendering:** a Level 2.5 page with underlined text renders without the underline in the browser, and with it in `neottx`; separated mosaics are unchanged.
- **Finding numbers:** "Sport 140", "p302", "140-145", "£150", "12:30", "1984", "302%", a number for an absent page, concealed numbers, numbers in rows 0 and 24.
- **The core's checks:** runs outside the page, overlapping runs, 33 runs, targets of every kind in section 7.1, and links naming another channel's page or an unsent version.
- **Bait and switch:** a page whose link target alternates every frame is never followed; one that has been unchanged for 500 milliseconds is.
- **Leaving:** each kind of link from section 7.1 leads to the right screen or to none; a confirmation from the page area, from a held key or within 500 milliseconds is ignored; cancelling leaves the tab unchanged.
- **The handoff:** on each platform, an `https` address opens in the default browser; `file:`, `ms-settings:`, a bare path and an address made to begin with `-` are refused before the opener is started.
- **Drawing:** the underline's thickness and position at each scale; double height; the contrasting edge on every background colour; nothing drawn below two device pixels a dot.
- **Keyboard and screen readers:** Tab order, Enter, Escape, and the accessibility tree's descriptions.
- **Nothing else opens:** every message kind other than the Open strip input, driven with web links present, never starts the opener.
- **Aliases and services:** a descriptor alias and a `name:mpp` target each reach the site-change screen; typing an aliased number shows its destination first.
- **Focus:** a prompt that appears while Enter is held, or before the focus was moved into it, is not confirmed.

## 18. Corrections from the security analysis

Applied on 20 October 2026 from [`hyperlinks-security.md`](hyperlinks-security.md) section 13:

1. `name:mpp` targets are another channel, not another service of this one; they take the site-change screen (sections 6.4 and 7.1).
2. The descriptor's `aliases` and `services` are a link source and enter the same table (sections 6.4 and 8.1).
3. A click is accepted only when its frame's target equals the strip's shown target, and the core re-resolves the pointer on every new frame (sections 8.1 and 10.2).
4. The core's target parser is its own small grammar, panic-free and fuzzed (section 8.2, requirement 17.12).
5. The prompt never takes the focus by itself (section 12, requirement 17.11).
6. The XDG Desktop Portal before `xdg-open` on Linux (section 12, decision 10).
7. Runs are trimmed to shown cells, and the underlined total is bounded (section 9.2).
8. Hand-offs are limited to one per two seconds, from the selected tab of a focused window, and logged (section 12, requirement 17.9).
9. The `-` rule in finding page numbers is dropped; ranges link both numbers (section 6.1).
10. Plain `http` hand-offs are a decision for the owner (decision 9).

## 19. What building it changed

Built on 20 October 2026, phases 1 and 2, behind the `links` cargo feature (on by default; `--no-default-features` compiles hyperlinks out, and the gates build and test both ways). Decision 43 of the architecture asks the owner to confirm these:

1. **The size floor is one and a half device pixels a dot, not two.** Measured: on an ordinary screen at the default window size the open page is drawn 498 pixels high, a dot 1.99 pixels, and with the floor at two no links were drawn at all. The property the floor protects holds at one and a half: the one-pixel hairline is two thirds of a dot or less (requirement 17.2, amended).
2. **Page numbers in the text** (section 6.1): a lone `p` may come before a number ("p302"); a number joined to digits by `.`, `,` or `:` on either side is not a page number ("3.142", "1,500", "123:45"); a `-` no longer excludes one ("140-145" links both).
3. **A decoder's link message says whether an image follows,** so the trusted core attaches links to the image they were found in, and decides them again at once when only the links changed.
4. **The interface reports the pointer's link again whenever a new image arrives.** A flashing page renews its image twelve times a second and the trusted core keeps four images' links; without this the strip would lose its destination within a third of a second.
5. **The pointer over a link that leaves** is the system's shortcut shape (`CursorIcon::Alias`): there is no "arrow out of a box" pointer.
6. **The colours,** chosen on screen (decision 3): the link `#3D8BFF`, edged white on dark backgrounds and black on light ones; the strip's link text `#1A4FD8`.
7. **The strip's questions name the bare destination** ("Open https://www.example.com/story in your web browser?"); the strip's pointing text adds "(leaves this channel)" or "(opens in your web browser)".
8. **Linux hands over with `xdg-open`** until a D-Bus crate for the XDG Desktop Portal is approved (decision 10). Windows uses `ShellExecuteW`, which opened an address in Edge in the Windows 11 VM; `xdg-open` has not been run by hand on Linux.
9. **Not built:**
   - offering the system's web browser when a teletext address turns out not to be teletext (section 7.1): neither the fetcher nor the decoder reports that a channel's content was refused as not teletext, and that report has to be designed first;
   - Fastext (section 6.3; requirement 12.4), built on 23 October 2026 by decision 67 (`page-controls.md`);
   - links in the accessibility tree: the browser has none yet (requirement 12.9).
10. **The history records page-started visits** (requirement 8.2), but no rule reads the record yet.

