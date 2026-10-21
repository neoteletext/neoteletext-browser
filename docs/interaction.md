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

---

*The sections below are history: version 1, as built on 20 October 2026.*

## 16. Tests

- **Push:** each transport delivers a page and a row; a dead socket is found by the missing pong and reconnected with the subscription and a resume record.
- **Links on an input channel:** page numbers are underlined as everywhere; a plain click on a link is reported to the host as a cell and follows nothing; Command-click follows it and is not reported; Tab goes to the host and focuses no link.
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
9. **The menu button opens the phone keyboard's panel;** the browser has no other menu yet. Phone keys go from the trusted core to the interface, which acts on them as on its own keys, so they type into a page or choose pages exactly as the keyboard does.
10. **The echo** is amber on a dark cell, in the interface's monospace face; a block cursor is an amber outline, so the character under it shows.
11. **QR codes** are drawn by Project Nayuki's `qrcodegen` in the engine (its decision E.93). A join address of qrkeyboard's length is a 37-module code; as teletext mosaics, one sixel a module, it is 24 columns by 15 rows, and a QR reader read it from a rendered page.
12. **`neottx view` leaves with Control-],** as telnet does, since a terminal window has no Command key (E.94).
13. **qrkeyboard's public relay does not run protocol 3 yet** (its `RELEASE_STATUS.md`). Everything was tested against a copy of the relay run locally; `NEOTELETEXT_QRKEYBOARD_RELAY` points the browser at one. *Amended 21 October 2026 (decision 46):* qrkeyboard.com runs protocol 3 since that day, and the engine's screen paired through it.
14. **Editing the address takes the keys from the page, and the channel is told,** with the same "keys left" record (kind 6) as when the window loses the focus, and "keys came" when the editing ends. The interface says when editing starts and stops, a message of its own beside the window's focus, which also governs handing links over (requirement 17.9).
- **By hand:** typing into `neoteletext-site terminal` over a slow link with echo on and off; a phone driving the browser; a phone driving a channel whose page carries the QR, on the browser and on `neottx`.
