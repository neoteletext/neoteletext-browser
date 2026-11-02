# NeoTeletext: channel push, typing back, and the cursor

| | |
|---|---|
| **Status** | The input protocol, shared with the engine, is defined by the documentation items of `~/devel/neoteletext-common/PROTOCOL.html`, DOC-1781 and DOC-1782 for the two directions. This document keeps what the protocol leaves to the browser: push (section 4), controls (section 5), the pointer on the page (section 6), and when input goes (section 7); and, as history, the tests and the changes made when version 1, the 14-byte key record of decisions 44 and 45, was built (sections 16 and 17). The rest of version 1's design is in this repository's history. |
| **Companion documents** | [`interaction-security.md`](interaction-security.md), DOC-2033 for the input system's security; [`architecture.md`](architecture.md); the requirements in rdm section 18, whose input requirements point to REQ-INT-130 to REQ-INT-139 in rdm. |

---

## 4. Push

REQ-WEB-005 stands, with two additions and one change:

- **A heartbeat.** After 25 s of silence the browser sends a WebSocket ping and expects a pong within 10 s; otherwise it reconnects with the existing backoff and re-sends its subscription.
- **TLS,** so that `wss://` works (requirement 5.8; decision 41 chose `rustls`). The phone keyboard needs it too.
- **Rows of the open page may be pushed on their own.** The guidance "push each page whole" stands for a carousel; a channel answering a key may push only the rows that changed, and the decoder renders the page in progress (section 10).

- **One selection on each tab's connection.** A tab has its own WebSocket, decoder and store, and tells its channel what it selects with SELECT (DEC-1538, DEC-1527). Losing the focus changes nothing about that. The wall of pages with none zoomed selects nothing (page 0), and a tab leaving the channel sends page 0 before its connection closes: the fetcher's input closing lets it write what it was last told, and the trusted core ends it a little later.
- **What a frame's links are.** A LINKS record (REQ-1494) follows a frame in the same message, and gives the sub-page's `LT`, `LK` and `LN` links as its page file's lines would. A frame with none means the sub-page has no links, whole or changed rows (DEC-1517), and a frame of the header alone is how links changing with no row changing redraws the page.
- **The contents.** A live channel's index records give the contents, and each magazine's name is its bank's name, whatever its first page's section. Its SUBPAGES records give each page's list of sub-pages; nothing is fetched after `channel.ttx` (REQ-703, REQ-1505).

The WebSocket is the transport because every host has one. When WebTransport (HTTP/3, QUIC) is as common, it is the transport to move to: unreliable datagrams for key records and pushed rows, no head-of-line blocking, and a connection that survives a change of network, which is what `mosh` built for the terminal. Nothing in the records would change.

## 5. Controls (decision 117)

A page that takes the mouse declares its controls, the cells it answers itself, with the control triplet of DOC-1844, and the browser shows them as DOC-1947 says: the hand over them while keys go to the page's channel, and the hover and pressed looks of `hyperlinks.md` section 9.3. A click on a control is the MOUSE event every click over the page already sends (DOC-1938, decision 118); a control changes nothing about where a click goes. The TypeScript kit's buttons, list rows and other press targets declare themselves, and its own hover, drawn by the channel, is removed.

In the browser:
- **The decoder** reads a page's controls with the engine's `controls::controls`, from the local enhancement data of the sub-page it draws, and sends them with its links (`FromDecoder::Links`), untrusted.
- **The trusted core** checks them again (requirement 17.13) and gives them to the interface with the page's links (`PageLinks.controls`).
- **The interface** shows the hand over a control only while a click on the open page goes to the channel, and asks for the look under the pointer.

## 6. Where the pointer is on the page

The mechanism is DOC-1946, with its REQ-INT-127: every position a channel receives is the cell and the dot of the page image under the pointer, never a screen position, so the open page may be drawn at any size, in the window or full screen, on any display's scaling, and a channel answers it the same. In the browser:

- **The interface converts.** winit gives the pointer, and each contact of a touch screen, in physical pixels; the interface divides by the window's scale factor (`app/input.rs`, `cursor_moved` and `touched`), so the browser works in logical pixels, the system's scaling already gone. `Browser::page_point` (`interface-core/src/browser/typing.rs`) takes the rectangle where the open page's image was drawn in the last frame, `Wall::open`, scales the position into the image's 480 by 500 dots, rounding down and keeping it inside the image, and gives the cell from that dot, 12 by 20 dots to a cell (`canvas.rs`'s `CELL_WIDTH` and `CELL_HEIGHT`), so the dot is always inside its cell (decision 125). The rectangle is the image alone, as `canvas::draw_page_on_screen` returns it, at the image's own proportions inside the border of its screen colour one cell wide (decision 60); the border is not the page.
- **Only the open page at rest.** `Wall::open` is set only while the zoomed page is drawn still, with no slide to its neighbours under way, and in full screen (decision 85) the same way in the larger rectangle; a page zooming, sliding or on the wall has no open page, so a click or move there sends nothing to a channel.
- **Every move, paced by the trusted core** (decision 125). The interface gives each change of the dot over the open page as a move, buttons held or not; the trusted core paces them at most 60 a second with the engine's `pace::Pacer` (`core/src/typing/pointer.rs`), writes the waiting move before any other event, and never an unchanged dot; whatever the interface sends, the rate is the trusted core's.
- **Larger or smaller.** Drawn larger, several pixels show a dot and a move within it is not sent (the interface gives, and the trusted core sends, a move only when the dot changes), so the page's size adds no moves. Drawn smaller, some dots cannot be reached and moves step several dots; cells stay exact. The page size bar and full screen change only the rectangle.
- **One dot at a boundary.** `draw-vello` samples the image at each pixel's centre with nearest-neighbour sampling (`pages.rs`), and the conversion rounds the pointer down, so at a size that is not a whole multiple the dot sent may be the neighbour of the one drawn under the pointer's tip; never another cell's. Inferred from the code, not measured.
- **The trusted core checks.** The interface is not trusted (decision 20). `protocol`'s intent reader, in the trusted core, refuses a cell outside the page, and a dot outside the image or outside its cell (DOC-1928 REQ-INT-127, `protocol::interaction::dot_in_cell`, by the engine's geometry constants), for the mouse, the wheel and a contact; a refusal closes the interface, as every protocol error from it does (decision 125, requirement 18.20). *Amended 28 October 2026:* until then the dot was not checked.
- **A drag let go off the page.** A release is sent only while the pointer is over the open page (`terminal_button`). The pointer leaving the page, onto the border, the browser's controls or out of the window (winit's `CursorLeft`), is now gone (DOC-1938 REQ-INT-123, `RETURN-CHANNEL.md` REQ-RCH-034): the interface gives `Intent::Gone`, and the trusted core sends the waiting move and then MOUSE action 3 with the buttons held, which releases them without a click; the release that follows off the page is not sent. Keys stopping going to the channel does the same for the pointer and every contact (decision 125, requirement 18.17). *Amended 28 October 2026:* until then a drag let go there left the channel believing the button held.
- **Touch** (decision 125, requirement 18.18). On Windows and Linux winit reports each contact with an identifier and a phase; macOS's trackpads report none. While keys go to the open page's channel, a contact that begins on the page goes to the trusted core with the system's identifier and its position converted as the pointer's; the trusted core numbers it with the engine's `contacts::Contacts`, the first as MOUSE button 0 and the others as TOUCH. A contact that leaves the page is cancelled for the channel, so gone. A contact that begins on a link is the browser's and follows the link if it lifts on it (requirement 18.19). winit does not tell a pen from a finger, so a pen goes as touch.
- **Forty columns.** A page is 40 columns by 25 rows at every presentation level (DEC-1636), so a cell is at most column 39 and row 24, and the image and the conversion are always that size. The intent reader refuses a cell past them, and the frame's encoder and reader refuse an image of any other size (REQ-1645).

---

*The sections below are history: version 1, as built on 20 October 2026.*

## 7. When input goes

What the protocol leaves to the receiver, for NeoTeletext. The wire's side of each of these is in the protocol document: focus DOC-1933, the round trip DOC-1918, the cursor record DOC-1954, a selection DOC-1753, the pointer DOC-1945 and DOC-1946. The boundaries the input crosses are `interaction-security.md` section 4.

### 7.1 Which process does what

| Step | Process | Does |
|---|---|---|
| 1 | Interface | Takes the window's key, mouse, wheel, paste and composition events. Sends an *intent* to the trusted core only when the active tab's open page takes input and the browser's own modifier (Command on macOS, Control-Shift elsewhere) is not held. Draws the cursor the trusted core allows. |
| 2 | Trusted core | Checks every condition again (REQ-1378), applies the rate limit, paces the pointer's moves and numbers the contacts of a touch screen, tracks held keys and the input count, keeps what is not yet acknowledged for a resumption, **encodes the protocol**, and alone sends HELLO, RESUME, RESIZE, SELECT, FOCUS and the releases, stamping every input event with the page on screen. |
| 3 | Fetcher | Writes each message it is given to the channel's push WebSocket at once, those given at one moment together; holds a move while the socket's unsent bytes are over a bound; measures the round trip by WebSocket ping and pong. It takes nothing else and opens nothing (REQ-INT-133). |
| 4 | Decoder | Finds cursor and control records in pushed packets and `CU` lines, untrusted, and passes them on. |
| 5 | Trusted core | Checks each cursor and control record; applies WELCOME; passes the interface only what it may draw. |

The interface never encodes the wire format: it sends intents, so a compromised interface can at worst type into the one channel the trusted core already lets it type into. The phone keyboard is a sixth process, `neoteletext-keyboard`, which alone holds the pairing secret; its keys go to the trusted core and on to the interface, which treats them as its own.

### 7.2 Selected and focused

The browser sends the viewer's input only for the connection's selected page, and only while that page is focused (REQ-1554). While the tab shows "Loading", "Connecting" or nothing of the selected page, no input is sent. Losing either focus never sends SELECT and never stops the page's updates (REQ-1553). Each tab has its own connection (DEC-1527).

### 7.3 When keys and the pointer go

**The keyboard focus** (REQ-1378). Keys go to a channel only while all of these hold: the tab is the one in front; the page selected in it is the channel's; the channel declares input over a WebSocket whose session has been welcomed; the window has the focus; and the viewer is not editing the address. Then every key goes except the browser's own: those with the browser's modifier (Command on macOS, Control-Shift elsewhere), and the strip, the overview and a background tab, which never take keys, and the magazines and the page list, which take the arrows from a click on them until a click elsewhere (decision 105). Command-Escape (Control-Shift-Escape elsewhere) leaves the page; plain Escape is the channel's.

Whatever the keys do, the trusted core sends SELECT on a tab's own connection whenever the tab's page or sub-page changes, and SELECT of nothing when the tab leaves the channel's pages.

When keys start going, the trusted core sends FOCUS_IN. When they stop, whether another tab came to the front, the window lost the focus, the address is being edited or another part of the browser took the keys, it sends FOCUS_OUT, so the channel lets go of a key held as the viewer left, and FOCUS_IN when they come back. The page stays selected and active through all of it.

**The pointer focus.** Every press, release and move over the page goes to the channel while keys go to it, moves paced as DOC-1945 says; a click with the browser's modifier is the browser's. A link the page declares is the browser's on every page: a click on it follows it, and nothing pressed on it goes to the channel. When the pointer leaves the page, or keys stop going, the waiting move and then gone. The wheel goes to the channel while the pointer is over the page. Outside the page the browser's own actions apply and nothing goes to the channel: the wheel steps to the next or previous page, which is a new selection, sent with SELECT.

**Touch** (REQ-RCH-047, REQ-RCH-048). While keys go to the channel every contact on its page is the channel's, two fingers and a pinch included; the browser's gestures, a pinch to zoom and a swipe to step sub-pages, act on pages that do not take input and outside the page, and the system's gestures from the screen's edges are the system's. The browser zooms a page that takes input with its own controls. A link is the browser's on every page: a contact that begins on a link the page declares follows it when it lifts there, and goes to the channel not at all.

**Paste and composition.** The browser's paste shortcut pastes into the page as PASTE while keys go to it. The window's input method stays enabled while typing; its preedit goes as COMPOSE, and its committed text as text.

### 7.4 The strip

The trusted strip says "Keys go to *host*" while keys go, with "Not secure" on a plain `ws://` socket and the round trip of the push connection itself, measured by WebSocket ping and pong at most once a second while keys go (REQ-RCH-016), not from acknowledgements, which measure only the channel's receipt. No page can draw into the strip.

### 7.5 The cursor

The interface draws the cursor as an overlay in a style no page can draw: an amber bar 0.15 of a cell wide, blinking with a half period of 530 ms, or, with BLOCK, an amber outline 0.12 of a cell thick, so the character under it shows. Never an underline (decision 42). The cursor is shown only on the page it names, only while keys go to its channel.

**No echo.** The browser draws nothing of the viewer's keys: what is typed shows as the channel draws it on its page. Provisional echo, the viewer's own keys drawn at once in amber until the channel acknowledged them, was withdrawn on 28 October 2026 at the owner's word (the engine's decision E.134), with its capability, its flags and its width.

### 7.6 Page selection

A channel that takes input selects pages by videotex's convention, which it implements: `*nnn#` goes to a page, `#` to the next, `*#` to the one before, `*00` shows the page again; three plain digits where the channel has no other use for them. The browser interprets none of it: it sends the keys.

The browser's own selections are ordinary SELECTs. The wall of pages selects nothing while it is shown, drawing from the tab's cache of page source; it fills a missing visible tile by an ordinary page load, one at a time in screen order, and then sends SELECT of nothing (DEC-1552).

## 16. Tests

- **Push:** each transport delivers a page and a row; a dead socket is found by the missing pong and reconnected with the subscription and a resume record.
- **Links on an input channel:** declared links are underlined as everywhere (page numbers in the text are no longer links, decision 116); a plain click on a link follows it and is not reported, as on any page, and no button pressed on a link reaches the host (decision 122, the owner's revision of decision 118 on 28 October 2026: "links must be acted upon by the client browser"); a click on a control, or on any cell off a link, is the host's; Tab goes to the host and focuses no link.
- **Resume:** after a dropped socket the resume record goes first and the host's re-push of the open page is drawn.
- **Typing:** keys go only under every condition of 18.2; every key goes, Escape, Enter, Control-C, the F-keys, digits and colour keys included; the strip's keys, a background tab's keys and the browser's shortcuts never go; closing the page stops them and releases a held key; on a channel without input three digits still choose a page locally.
- **Page selection:** against `neoteletext-site terminal`, `*302#` opens page 302, `#` the next, `*#` the one before, and `302` alone opens 302 where the page has no field.
- **Records:** every kind round-trips; over 46 bytes refused; the clock counts up and wraps; a key's physical code is its position and its code its meaning, checked on a non-QWERTY layout; the 121st key in a second dropped but never a key-up; every cursor field's bounds; a record for a page not open changes nothing; `acked` out of order.
- **Cursor:** drawn only on the open page while a cursor is placed; with no record, or a record with no cursor, nothing is drawn and keys still go; never an underline. *Amended 28 October 2026:* the echo is removed at the owner's word ("remove that mechanism entirely"): what is typed shows only as the channel draws it, and a cursor record's echo width and its echo and masked flags are not used.
- **A game:** against a `neoteletext-site` mode that pushes a moving page twenty times a second, key down and up with repeat reach it, and the frames are drawn at the display rate with no cursor on screen.
- **Latency:** a key on the wire within 1 ms; a pushed row on screen within 20 ms; a page pushed twenty times a second with the codecs measured; numbers in the full gate.
- **The phone:** the keyboard process's readers fuzzed; a key with a modifier, over 32 bytes, or text over 995 bytes refused; keys act as local keys; the QR only from the panel; Finish ends the session; five wrong PINs lock.
- **Compiled out:** builds and tests without `interaction` and without `phone-keyboard`.

## 17. What building changed

Built on 20 October 2026, all three phases, in the engine (its decisions E.90 to E.94) and here (decision 45). Where the build departs from sections 1 to 16, this section is the design. The owner confirmed every item on 21 October 2026 (decision 46):

1. **The cursor record is on data channel 12, not 13.** EN 300 708 Table 16 gives data channel 13 to Datavideo. The record is type 4 of the audio profile's frame on the profile's own channel 12, always one fragment, which the audio receiver steps over (engine decision E.91).
2. **`input` sits in the descriptor's `push` object:** `"push": { "input": true }` (E.92).
3. **The engine does not encode a `CU` line as a packet.** The formats crate reads and writes the line; the decoder reads it from the page it already has, so the formats crate takes nothing from the input profile (E.91).
4. **Every fetched file and pushed message is read whole** and its pages completed at once. A pushed page had shown only when the next page of its magazine came, and the last page of each magazine of a fetched channel never arrived at all. A fetched copy of a page the channel pushed is older and is dropped, since the contents' order may bring it after the push.
5. **The trusted core numbers the records,** not the interface: it alone sends page, focus, resume and release records. *Amended 28 October 2026:* the interface no longer numbers its keys, which it did only for its echo, now removed.
6. **`Sent { seq }` is not sent.** The round trip runs from the core's send, within a millisecond of the wire on this computer; the fetcher wakes at once for a key. Each connection's own thread reads its socket and wakes the fetcher when bytes arrive, so the channel's answer is taken at once too: the round trip's median fell from 6.3 ms, when the fetcher looked at its sockets every 5 ms, to 0.28 ms (requirement 18.5).
7. **A channel moves a viewer with a REDIRECT record** (DOC-1960, DEC-1689), and nothing else moves one: a cursor record never does, whatever page it names. A redirect to a page of the channel opens it whether or not keys go to the channel (DEC-1707), as a terminal shows what its host sends: `*200#` on the reference channel opens page 200. The trusted core carries it out, keeping the tab's address, and refuses a page the channel does not list, saying its own selection again. A redirect to another channel's address goes by the address rules and shows the redirect notice (REQ-1686, REQ-1688). The viewer choosing a page overrides it.
8. **Command-Escape** (Control-Shift-Escape elsewhere) leaves the page, as section 5 has it; plain Escape is the host's.
9. **The menu button opens the phone keyboard's panel;** the browser has no other menu yet. Phone keys go from the trusted core to the interface, which acts on them as on its own keys, so they type into a page or choose pages exactly as the keyboard does. *Amended 22 October 2026 (decision 57):* the menu button opens the browser's menu, which holds the phone keyboard (`history-and-bookmarks.md` section 14 item 9). *Amended 24 October 2026 (decision 99, finding F-20):* outside a page open for typing, phone keys no longer act as the window's own keys: they only move between pages. The trusted core passes on only the digits, the four arrows and the four colour keys, and the interface gives them to the page view alone (three digits choose a page, the arrows step, the colour keys are the open page's Fastext keys; the digits of text typed on the phone choose a page too); while the address field or another text field is edited, the strip asks its question, or a panel, popover or list is open, phone keys are dropped. A compromised keyboard process could otherwise type an address, change a setting, or move to Go and confirm a hand-off.
10. **A block cursor** is an amber outline, so the character under it shows. *Amended 28 October 2026:* the echo, amber on a dark cell, is removed: typed text shows only as the channel draws it.
11. **QR codes** are drawn by Project Nayuki's `qrcodegen` in the engine (its decision E.93). A join address of qrkeyboard's length is a 37-module code; as teletext mosaics, one sixel a module, it is 24 columns by 15 rows, and a QR reader read it from a rendered page.
12. **`neottx view` leaves with Control-],** as telnet does, since a terminal window has no Command key (E.94).
13. **qrkeyboard's public relay does not run protocol 3 yet** (its `RELEASE_STATUS.md`). Everything was tested against a copy of the relay run locally; `NEOTELETEXT_QRKEYBOARD_RELAY` points the browser at one. *Amended 21 October 2026 (decision 46):* qrkeyboard.com runs protocol 3 since that day, and the engine's screen paired through it.
14. **Editing the address takes the keys from the page, and the channel is told,** with the same "keys left" record (kind 6) as when the window loses the focus, and "keys came" when the editing ends. The interface says when editing starts and stops, a message of its own beside the window's focus, which also governs handing links over (requirement 17.9).
- **By hand:** typing into `neoteletext-site terminal` over a slow link; a phone driving the browser; a phone driving a channel whose page carries the QR, on the browser and on `neottx`.
