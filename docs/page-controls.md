# NeoTeletext: Fastext, reveal, hold, mix and sub-pages

| | |
|---|---|
| **Asked** | By the owner on 23 October 2026: "implement fastext, reveal, subpages"; whether hold and mix come with them was left open. |
| **Decision** | 67, taken while the owner was away (`architecture.md` section 2). |
| **Engine** | Each is the engine's: REQ-NAV-003 and REQ-NAV-011 (Fastext), REQ-L1-010 and REQ-NAV-007 (reveal and mix), REQ-NAV-006 (hold), REQ-NAV-005 (sub-pages), REQ-SEC-024 (the keys only while row 24 is shown). The browser asks the engine for each and decides nothing about teletext itself (requirement 2.1). |

## 1. What a teletext set offers, and what the browser lacked

A teletext set's remote has four coloured keys, an Index key, Reveal, Hold, Mix and a way through a rotating page's sub-pages. The engine has all of them; until now the browser offered none: a page's coloured links were drawn but could not be followed, concealed text could not be shown, a rotating page could not be stopped, and only the latest sub-page of a page was ever shown (the review of 23 October 2026).

## 2. Fastext

- **Where the links come from.** The engine's `PageLinks::from_page` reads packet X/27/0, which broadcast pages carry and which the engine's page encoder writes from a TTI page's `FL` and `LN` lines, so every channel's pages have them alike. Four coloured keys, red, green, yellow and cyan, and Index.
- **The keys.** As the engine's own viewer: `r`, `g`, `y`, `c` or F1 to F4, and `i` for Index, while a page is open and no channel takes the keys. The remote control carries them too (`remote-control.md`, decision 68).
- **Only while their row is shown** (REQ-SEC-024). A page can draw anything in its rows, but the coloured key's promise is row 24: a key works only while row 24 has something drawn and the page is not boxed away, as the engine's viewer decides it. The decoder reports this with the links.
- **Where a key leads.** A page of the channel, with its sub-page if the link names one, opens at once. A page of another service the channel's descriptor lists goes through the site-change screen in the strip, as every link that leaves a channel does (requirement 8.4). A link naming a web address is not followed by a key: it is shown on its page and followed by a click, as a page's other links are.
- **The link row of a TTI page** (REQ-NAV-011): a page whose links come from `LN` lines has its row 24 drawn by the receiver from the links' labels; the decoder draws it with the engine's `links::receiver_links` (the engine's E.108): each key's label in its colour, and a link to another service, which this browser does not follow, blank, where the row the page file carries shows it in white with "»" (`link_lines_draw_the_receivers_row_24`).

## 3. Reveal, hold and mix

- **Reveal** shows concealed text (REQ-L1-010): quiz answers, punchlines. `?`, as the engine's viewer, toggles it on the open page. It is per page and lapses when another page is opened, as on a set.
- **Hold** stops the open page changing: a new version, or the next sub-page of a rotating page, is not shown until hold is let go (REQ-NAV-006). `H`. Decided to come with the others: a rotating page cannot be read otherwise, and the engine has it.
- **Mix** draws the cells' backgrounds transparent, as text over the picture (REQ-L1-020). `m`. Decided to come too, since the engine has it and subtitle and newsflash pages are made for it; with no picture behind, the window's own background shows through.

Each is strip input from the interface (requirement 11.1) and lapses when the page changes. None is a security matter: concealed text is the page's own, sent to the decoder already.

## 4. Sub-pages

- **Kept.** The engine's store keeps every sub-page it receives, within its bound; the decoder now reports a page's sub-pages, in their order, and which is shown.
- **Shown.** By default the latest received, so a rotating page rotates as broadcast. Choosing one, by Left and Right while the page is open (the engine viewer's keys) or in the sub-page list, shows that one and holds it there until another page is opened or the latest is chosen again.
- **In the address.** `#302:0002` opens page 302 at sub-page 0002; the address rules accepted the sub-page before and dropped it. The address field shows the sub-page while one is chosen, and Back and history keep it.
- **On the page.** While the open page has more than one sub-page, the strip below it shows "2/4" with a step each way, with its tip.

## 5. Messages

- **To the decoder:** `View { page, reveal, hold, mix, subpage }`, whenever the open page's view changes; the decoder renders that page so.
- **From the decoder:** `SubPages { page, subcodes, shown }`, bounded, whenever they change for a page on screen; and, within the existing `Links` message, the page's five Fastext targets and whether row 24 is shown, so that a key is checked against the image the user saw, as a click is (requirement 12.3).
- **From the interface:** `PageControl` with a key (a colour, Index, reveal, hold, mix, a sub-page step or a sub-page chosen), strip input.
- **To the interface:** the open page's controls: how many sub-pages and which is shown, whether reveal, hold and mix are on, and which coloured keys lead somewhere.

## 6. Security

- A Fastext target is untrusted, like any link the decoder finds (`hyperlinks-security.md`): the trusted core checks it against the page image the user was looking at, decides it, and leaves the channel only through the site-change screen. A page cannot make a key act while its row 24 is hidden.
- The sub-page list and its length are bounded; a hostile channel with thousands of sub-pages is held to the engine's store bound.
- Nothing here gives the interface anything new: it already draws the page; it learns the count of sub-pages, nothing more.

## 7. Requirements

Section 22 of `requirements.md`, requirements 22.1 to 22.6.

## 8. What building settled

Built on 23 October 2026.

1. **Keys of the remote.** The keyboard has no coloured keys, so the engine viewer's letters stand for them, and F1 to F4 as in its viewer. The remote control carries buttons for all of them.
2. **Left and Right** step sub-pages only on a page with more than one; elsewhere they size the pages as before, so nothing is lost.
3. **The sub-page control** showed the count with a turning mark while the latest was shown as it arrived, and "2/4" once one was chosen; pressing the count returned to the latest. *Removed 28 October 2026 at the owner's word:* nothing is drawn over the page; the arrows and the sub-page list choose a sub-page, and the remote's Latest button returns to the latest.
4. **The address field** does not show the sub-page chosen, and Back and history keep the page, not the sub-page: an address naming a sub-page opens there, which is what was asked. Departure from section 4, for the owner to confirm.
5. **A sub-code of 3F7F** is a page's own sub-code on many broadcasts, so a list may hold it; a link or a choice never names it, since it means "any".
6. **The engine's bound of 64 sub-pages** is the engine's `viewer::MAX_LISTED_SUBPAGES` since its commit 0d05d1f; the decoder lists by it, and the interface core, which does not depend on the engine, restates it with an assertion in the decoder that the two agree (requirement 2.4). *Note of 26 October 2026:* the bound is to be the standard's, 8,190, from the engine's `neoteletext_core::limits`, with no restatement (decision 112). *Note of 28 October 2026:* every sub-page up to 8,190 is listed and reachable, told apart by all thirteen bits of its sub-code, and the sub-codes of data broadcasting and enhancement data pages are never listed (decision 119; the engine's REQ-NUM-006, REQ-NUM-022, REQ-NUM-024 and REQ-NUM-030). Item 5 is the engine's REQ-NUM-009: a page may carry sub-code 3F7F, which a link cannot name. *Built 28 October 2026 (decision 121):* the bound is `neoteletext_core::limits::MAX_SUBPAGES`, with no restatement; the decoder steps through its store and tells the trusted core a count, the place of the sub-page drawn and its neighbours, never the list; a page with a catalogue (`NNN.sub`) is counted and stepped through by its catalogue, every sub-page held or not; the sub-page list shows places to 8,190, and the list asks for its rows' titles by range.
7. **The receiver-drawn row 24** of a page with `LN` lines (the engine's REQ-NAV-011) is drawn by the decoder with the engine's `links::receiver_links` since the re-pin at `00c90ea` (the engine's E.108; section 2; `link_lines_draw_the_receivers_row_24`).
