# NeoTeletext: channel push, typing back, and the cursor

| | |
|---|---|
| **Status** | The design of typing back, the cursor and the phone keyboard is now `~/devel/neoteletext-common/INPUT.md`, the input system shared with the engine (protocol version 2, adopted 21 October 2026: the browser's decision 48, the engine's E.100). This document keeps what INPUT.md does not cover, push (section 4), and as history the tests and the changes made when version 1, the 14-byte key record of decisions 44 and 45, was built (sections 16 and 17). The rest of version 1's design is in this repository's history. |
| **Companion documents** | [`interaction-security.md`](interaction-security.md), INPUT.md section 11 for the input system's security; [`architecture.md`](architecture.md); [`requirements.md`](requirements.md) section 18, whose input requirements point to INPUT.md's REQ-INT-130 to REQ-INT-139. |

---

## 4. Push

REQ-WEB-005 stands, with two additions and one change:

- **A heartbeat.** After 25 s of silence the browser sends a WebSocket ping and expects a pong within 10 s; otherwise it reconnects with the existing backoff and re-sends its subscription. A silent Server-Sent Events stream is reconnected after 60 s.
- **TLS,** so that `wss://` works (requirement 5.8; decision 41 chose `rustls`). The phone keyboard needs it too.
- **Rows of the open page may be pushed on their own.** The guidance "push each page whole" stands for a carousel; a channel answering a key may push only the rows that changed, and the decoder renders the page in progress (section 10).

The WebSocket is the transport because every host has one. When WebTransport (HTTP/3, QUIC) is as common, it is the transport to move to: unreliable datagrams for key records and pushed rows, no head-of-line blocking, and a connection that survives a change of network, which is what `mosh` built for the terminal. Nothing in the records would change.

## 5. Controls (decision 117)

A page that takes the mouse declares its controls, the cells it answers itself, with the control triplet of INPUT.md section 7.3, and the browser shows them as INPUT.md section 9.7 says: the hand over them while keys go to the page's channel, and the hover and pressed looks of `hyperlinks.md` section 9.3. A click on a control is the MOUSE event every click over the page already sends (INPUT.md section 6.3, decision 118); a control changes nothing about where a click goes. The TypeScript kit's buttons, list rows and other press targets declare themselves, and its own hover, drawn by the channel, is removed.

In the browser:
- **The decoder** reads a page's controls with the engine's `controls::controls`, from the local enhancement data of the sub-page it draws, and sends them with its links (`FromDecoder::Links`), untrusted.
- **The trusted core** checks them again (requirement 17.13) and gives them to the interface with the page's links (`PageLinks.controls`).
- **The interface** shows the hand over a control only while a click on the open page goes to the channel, and asks for the look under the pointer.

## 6. Where the pointer is on the page

The mechanism is `~/devel/neoteletext-common/INPUT.md` section 9.9, with its REQ-INT-127: every position a channel receives is the cell and the dot of the page image under the pointer, never a screen position, so the open page may be drawn at any size, in the window or full screen, on any display's scaling, and a channel answers it the same. In the browser:

- **The interface converts.** winit gives the pointer in physical pixels; the interface divides by the window's scale factor (`app/input.rs`, `cursor_moved`), so the browser works in logical pixels, the system's scaling already gone. `Browser::pointer_on_page` (`interface-core/src/browser/typing.rs`) takes the rectangle where the open page's image was drawn in the last frame, `Wall::open`, and scales the position into the image's 480 by 500 dots, rounding down; `wall::cell_in` gives the cell from the same position, 12 by 20 dots to a cell (`canvas.rs`'s `CELL_WIDTH` and `CELL_HEIGHT`). The rectangle is the image alone, as `canvas::draw_page_on_screen` returns it, at the image's own proportions inside the border of its screen colour one cell wide (decision 60); the border is not the page.
- **Only the open page at rest.** `Wall::open` is set only while the zoomed page is drawn still, with no slide to its neighbours under way, and in full screen (decision 85) the same way in the larger rectangle; a page zooming, sliding or on the wall has no open page, so a click or move there sends nothing to a channel.
- **Larger or smaller.** Drawn larger, several pixels show a dot and a move within it is not sent (the trusted core sends a move only when the dot changes), so the page's size adds no moves. Drawn smaller, some dots cannot be reached and moves step several dots; cells stay exact. The page size bar and full screen change only the rectangle.
- **One dot at a boundary.** `draw-vello` samples the image at each pixel's centre with nearest-neighbour sampling (`pages.rs`), and the conversion rounds the pointer down, so at a size that is not a whole multiple the dot sent may be the neighbour of the one drawn under the pointer's tip; never another cell's. Inferred from the code, not measured.
- **The trusted core checks.** The interface is not trusted (decision 20). `protocol`'s intent reader refuses a cell outside the page; it does not yet check the dot, which INPUT.md REQ-INT-127 asks it to refuse outside the image or outside its cell. Until it does, a misbehaving interface can put any 32-bit dot in a MOUSE or WHEEL event, which confuses only the channel's own page.
- **A drag let go off the page.** A release is sent only while the pointer is over the open page (`terminal_button`), so a drag let go on the border or outside the window leaves the channel believing the button held. MOUSE's gone (INPUT.md REQ-INT-123, `RETURN-CHANNEL.md` REQ-RCH-034), sent as the pointer leaves, releases it; it is built in the engine (E.133) and not yet in the browser.
- **Side panels** are not drawn, so a cell is at most column 39; drawing them would widen the image to 72 cells and the conversion with it.

---

*The sections below are history: version 1, as built on 20 October 2026.*

## 16. Tests

- **Push:** each transport delivers a page and a row; a dead socket is found by the missing pong and reconnected with the subscription and a resume record.
- **Links on an input channel:** declared links are underlined as everywhere (page numbers in the text are no longer links, decision 116); a plain click on a link follows it and is not reported, as on any page, and no button pressed on a link reaches the host (decision 122, the owner's revision of decision 118 on 28 October 2026: "links must be acted upon by the client browser"); a click on a control, or on any cell off a link, is the host's; Tab goes to the host and focuses no link.
- **Resume:** after a dropped socket the resume record goes first and the host's re-push of the open page is drawn.
- **Typing:** keys go only under every condition of 18.2; every key goes, Escape, Enter, Control-C, the F-keys, digits and colour keys included; the strip's keys, a background tab's keys and the browser's shortcuts never go; closing the page stops them and releases a held key; on a channel without input three digits still choose a page locally.
- **Page selection:** against `neoteletext-site terminal`, `*302#` opens page 302, `#` the next, `*#` the one before, and `302` alone opens 302 where the page has no field.
- **Records:** every kind round-trips; over 46 bytes refused; the clock counts up and wraps; a key's physical code is its position and its code its meaning, checked on a non-QWERTY layout; the 121st key in a second dropped but never a key-up; every cursor field's bounds; a record for a page not open changes nothing; `acked` out of order.
- **Cursor and echo:** drawn only on the open page while a cursor is placed; with no record, or a record with no cursor, nothing is drawn and keys still go; the echo stays within the record's width, is masked when asked, and is removed on acknowledgement or after two seconds; never an underline.
- **A game:** against a `neoteletext-site` mode that pushes a moving page twenty times a second, key down and up with repeat reach it, and the frames are drawn at the display rate with no cursor on screen.
- **Latency:** a key on the wire within 1 ms; a pushed row on screen within 20 ms; a page pushed twenty times a second with the codecs measured; numbers in the full gate.
- **The phone:** the keyboard process's readers fuzzed; a key with a modifier, over 32 bytes, or text over 995 bytes refused; keys act as local keys; the QR only from the panel; Finish ends the session; five wrong PINs lock.
- **Compiled out:** builds and tests without `interaction` and without `phone-keyboard`.

## 17. What building changed

Built on 20 October 2026, all three phases, in the engine (its decisions E.90 to E.94) and here (decision 45). Where the build departs from sections 1 to 16, this section is the design. The owner confirmed every item on 21 October 2026 (decision 46):

1. **The cursor record is on data channel 12, not 13.** EN 300 708 Table 16 gives data channel 13 to Datavideo. The record is type 4 of the audio profile's frame on the profile's own channel 12, always one fragment, which the audio receiver steps over (engine decision E.91).
2. **`input` sits in the descriptor's `push` object:** `"push": { "address": …, "method": "websocket", "input": true }`, refused with server-sent events (E.92).
3. **The engine does not encode a `CU` line as a packet.** The formats crate reads and writes the line; the decoder reads it from the page it already has, so the formats crate takes nothing from the input profile (E.91).
4. **Every fetched file and pushed message is read whole** and its pages completed at once. A pushed page had shown only when the next page of its magazine came, and the last page of each magazine of a fetched channel never arrived at all. A fetched copy of a page the channel pushed is older and is dropped, since the contents' order may bring it after the push.
5. **The trusted core numbers the records,** not the interface: it alone sends page, focus, resume and release records. The interface numbers its keys for its echo, and the core tells it which it has seen acknowledged.
6. **`Sent { seq }` is not sent.** The round trip runs from the core's send, within a millisecond of the wire on this computer; the fetcher wakes at once for a key. Each connection's own thread reads its socket and wakes the fetcher when bytes arrive, so the channel's answer is taken at once too: the round trip's median fell from 6.3 ms, when the fetcher looked at its sockets every 5 ms, to 0.28 ms (requirement 18.5).
7. **A host goes to a page by sending it.** When a cursor record names another page of the same channel than the one open, while keys go to that channel, the browser shows that page, as a terminal shows what its host sends: `*200#` on the reference channel opens page 200. It is the only thing a channel's record can make the browser do, within the channel, and never a fetch; the viewer choosing a page overrides it.
8. **Command-Escape** (Control-Shift-Escape elsewhere) leaves the page, as section 5 has it; plain Escape is the host's.
9. **The menu button opens the phone keyboard's panel;** the browser has no other menu yet. Phone keys go from the trusted core to the interface, which acts on them as on its own keys, so they type into a page or choose pages exactly as the keyboard does. *Amended 22 October 2026 (decision 57):* the menu button opens the browser's menu, which holds the phone keyboard (`history-and-bookmarks.md` section 14 item 9). *Amended 24 October 2026 (decision 99, finding F-20):* outside a page open for typing, phone keys no longer act as the window's own keys: they only move between pages. The trusted core passes on only the digits, the four arrows and the four colour keys, and the interface gives them to the page view alone (three digits choose a page, the arrows step, the colour keys are the open page's Fastext keys; the digits of text typed on the phone choose a page too); while the address field or another text field is edited, the strip asks its question, or a panel, popover or list is open, phone keys are dropped. A compromised keyboard process could otherwise type an address, change a setting, or move to Go and confirm a hand-off.
10. **The echo** is amber on a dark cell, in the interface's monospace face; a block cursor is an amber outline, so the character under it shows.
11. **QR codes** are drawn by Project Nayuki's `qrcodegen` in the engine (its decision E.93). A join address of qrkeyboard's length is a 37-module code; as teletext mosaics, one sixel a module, it is 24 columns by 15 rows, and a QR reader read it from a rendered page.
12. **`neottx view` leaves with Control-],** as telnet does, since a terminal window has no Command key (E.94).
13. **qrkeyboard's public relay does not run protocol 3 yet** (its `RELEASE_STATUS.md`). Everything was tested against a copy of the relay run locally; `NEOTELETEXT_QRKEYBOARD_RELAY` points the browser at one. *Amended 21 October 2026 (decision 46):* qrkeyboard.com runs protocol 3 since that day, and the engine's screen paired through it.
14. **Editing the address takes the keys from the page, and the channel is told,** with the same "keys left" record (kind 6) as when the window loses the focus, and "keys came" when the editing ends. The interface says when editing starts and stops, a message of its own beside the window's focus, which also governs handing links over (requirement 17.9).
- **By hand:** typing into `neoteletext-site terminal` over a slow link with echo on and off; a phone driving the browser; a phone driving a channel whose page carries the QR, on the browser and on `neottx`.
