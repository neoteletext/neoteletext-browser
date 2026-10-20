# NeoTeletext: channel push, typing back, and the cursor

| | |
|---|---|
| **Status** | Adopted 20 October 2026 (the architecture's decision 44), every recommendation of section 14 taken as the decision. Built the same day, all three phases (decision 45); section 17 lists what building changed. |
| **Owner's brief** | A channel pushes teletext lines to the browser as a transmitter pushes them to a television. Optionally the viewer types back, each key as it is pressed, as a terminal does, with the least latency that can be had: over the channel's WebSocket, or from a phone through qrkeyboard.com. A cursor on the screen, optionally. Nothing more onerous than a terminal. |
| **Companion documents** | [`interaction-security.md`](interaction-security.md) is the security analysis. [`architecture.md`](architecture.md), [`requirements.md`](requirements.md), [`hyperlinks.md`](hyperlinks.md) (the same pattern: the channel supplies data, the browser draws the marks, the trusted core decides). The engine's REQ-WEB-002 and -005, its audio profile (area AUD, whose record carriage this reuses), and qrkeyboard's `DEVELOPER_GUIDE.md` (`~/devel/qrkeyboard/qrkeyboard`). |

---

## Contents

1. Summary
2. The model
3. What exists
4. Push
5. Typing into a page
6. The key record
7. The cursor record
8. Local echo
9. qrkeyboard
10. Latency
11. Processes and messages
12. Changes to the engine
13. Proposed requirements
14. Decisions
15. Phases
16. Tests

---

## 1. Summary

- **A channel pushes lines** down a WebSocket, as the engine already specifies: 42-byte packets, or whole pages. The browser's decoder takes them as it takes a broadcast. Server-Sent Events stay as a push-only alternative; nothing else is added.
- **A channel with a WebSocket may take keys back.** It says so in its descriptor. The viewer's keys then go back one at a time, as a terminal sends them, and the channel answers by pushing the rows that changed. There is no other way in: no requests per key, no forms over this path. A channel without a WebSocket is a broadcast.
- **The open page is the terminal.** While the viewer has a page open on a channel that takes input, every key goes to the channel, as every key goes to the host from a terminal with focus. Closing the page is standing up from the terminal. The strip says where the keys are going the whole time. No dialog, no click, no mode to discover.
- **qrkeyboard is a phone as a keyboard,** in two places: a channel can put the QR in a page and take the phone's keys itself, with no part for the browser (that works on a broadcast television too); or the browser can show the QR and take the phone as its own keyboard, whose keys do what the real keyboard's do.
- **Local echo hides the network.** When the channel asks, a typed character appears in the cursor's cell at once, provisionally, and the channel's next push confirms it: the idea of `mosh` on a 40-by-25 grid.
- **Latency is measured.** Every key carries a number; the channel's cursor record says which it has acted on; the browser shows the round trip. The browser's own path from a pushed line to the screen is cut from up to 200 ms to under 20 ms while a page is being typed into.
- **Security does not move.** Keys go only to the channel of the page the viewer opened, with the strip saying so; nothing a channel sends can request, navigate or run anything; the cursor and the echo are overlays no page can draw.

## 2. The model

A terminal on a line to a host. The host sends characters and places the cursor; the terminal draws them and sends each key as it is pressed; the host decides what the key means and sends back what changed. Local echo, when the host wants it, hides the line's delay. A terminal with no transmit line is a printer; a channel with no WebSocket is a broadcast.

What this gives: nothing executes in the browser (requirement 1.3); every page stays a plain teletext page; and the browser does not know or care whether the page is a form, a game or a shell.

A game is the plainest case: there is no cursor at all. The viewer opens the page, the arrow keys and the fire key go to the channel as they go down and come up, and the channel pushes the rows that changed, twenty times a second if it likes (a whole page is a kilobyte). Nothing else is needed: no cursor record, no echo. The cursor and the echo exist for the channels that are typed into, and a channel that is not sends neither.

**Terms.** A *channel* and a *page* are as the architecture defines them (section 3). The *push stream* is the channel's WebSocket (or Server-Sent Events) connection, already defined by the engine's REQ-WEB-005. A *key record* is one key event in a few bytes (section 6). A *cursor record* is the channel's answer: where the cursor is, whether to echo, and the last key it acted on (section 7). The *open page* is the page zoomed to fill the page area (`hyperlinks.md` section 10.1): the terminal.

## 3. What exists

- **The engine** specifies push over WebSocket and Server-Sent Events (REQ-WEB-005: a binary WebSocket message is whole 42-byte packets, a text message a TTI page) and builds both in `neoteletext-web`, with reconnection and a 64 KB message limit. The browser sends `subscribe all` once and nothing after. There is no heartbeat once a stream is open. The descriptor declares `push: { address, method }` on the channel's own origin.
- **The browser's fetcher** receives Server-Sent Events and passes pushed bytes to the decoder at once. Plain `ws://` should already work, since the engine's agent handles both methods; `wss://` cannot, for want of TLS (requirement 5.8).
- **Records on independent data lines** are built for audio: packet 4/31, data channel 12, a CRC per record (REQ-AUD-003, -007). The cursor record reuses them.
- **The fetcher takes no input** (requirement 4.7). There is no path from a key to a channel.
- **Timing today:** the decoder renders at each 83 ms tick and the interface draws a change within 100 ms, so a pushed line reaches the screen in up to 200 ms: fine for a carousel, too slow for typing.
- **One conflict:** REQ-AUD-008 says a binary WebSocket message is an audio record; REQ-WEB-005 says it is packets. Section 12 resolves it.

## 4. Push

REQ-WEB-005 stands, with two additions and one change:

- **A heartbeat.** After 25 s of silence the browser sends a WebSocket ping and expects a pong within 10 s; otherwise it reconnects with the existing backoff and re-sends its subscription. A silent Server-Sent Events stream is reconnected after 60 s.
- **TLS,** so that `wss://` works (requirement 5.8; decision 41 chose `rustls`). The phone keyboard needs it too.
- **Rows of the open page may be pushed on their own.** The guidance "push each page whole" stands for a carousel; a channel answering a key may push only the rows that changed, and the decoder renders the page in progress (section 10).

The WebSocket is the transport because every host has one. When WebTransport (HTTP/3, QUIC) is as common, it is the transport to move to: unreliable datagrams for key records and pushed rows, no head-of-line blocking, and a connection that survives a change of network, which is what `mosh` built for the terminal. Nothing in the records would change.

## 5. Typing into a page

As a terminal, and with no more steps than one:

1. **A channel that takes input says so** in its descriptor: `"input": true`, allowed only with push over a WebSocket. A page cannot say it.
2. **The viewer opens a page** of that channel: zooms it, as they do to read it. From then on **every key goes to the channel**, as every key goes to the host from a terminal with focus, and the strip shows a key glyph with "Keys go to adventure.example", with "Not secure" beside it on a plain connection, as the address field already shows.
3. **The viewer closes the page** (the close button, a click outside it, another page from the list, another tab, or Command-Escape on macOS and Control-Shift-Escape elsewhere), and the keys are the browser's again.

What goes: every key as it is pressed and released, Escape, Enter, Control-C, the F-keys, the digits and the colour keys included, text an input method commits, and a click on a cell of the page. What never goes: anything typed in the strip, the page list or the overview; a background tab's keys; and the browser's own shortcuts, which are exactly the keys a terminal emulator keeps for itself: Command combinations on macOS, Control-Shift combinations on Windows and Linux. Everything else is the host's.

There need be no cursor: a game has none, and keys go just the same. When there is one (section 7), it is where the channel says it is writing, nothing more: it is not clicked, and it does not gate the keys.

**Links on an input channel.** Links work as everywhere else (`hyperlinks.md`): the browser finds page numbers in the text, declared links show, and pointing at one names its destination in the strip, exactly as a terminal emulator finds addresses in a host's output and makes them clickable. Because the host may want clicks (kind 5), the click is shared as terminals share it when the host has mouse reporting on: a plain click on any cell, link or not, goes to the host; Command-click (Control-Shift-click elsewhere) follows the link, which is the browser's modifier in any case. Tab and Enter go to the host like every other key, so on an input channel links are followed with the pointer, not the keyboard; opening a page by a link tells the host so (kind 4), as any page change does.

That is all the viewer sees. Opening a page of a channel that takes input, with the channel's identity in the strip, is the consent, as sitting at a terminal is.

### 5.1 Page numbers: the teletext and videotex conventions

Typing a page number is the one key sequence with a convention of its own, and there are two, each belonging to one of the two kinds of channel:

- **Teletext:** the receiver holds the pages, so the receiver takes three digits and shows the page; the digits never leave the set. That is a channel **without input**, and the browser does it today: three digits choose a page of the channel.
- **Videotex (Prestel):** the terminal holds nothing, so every key goes to the host, and the host takes the page command: `*` then the page number then `#` goes to that page; `#` alone goes on to the next page; `*#` goes back to the one before; `*00` shows the page again. A plain digit is data, a menu choice or an answer, which the host interprets. That is a channel **with input**, and the host does it, as a Prestel computer did, by pushing the page.

So a channel that takes input is asked to follow the videotex convention for page selection, `*nnn#`, `#` and `*#`, and may also take three plain digits as a page number where it has no other use for them, as a teletext service on a videotex host would. The browser does not interpret either; it sends the keys. A channel may show the number as it is typed by pushing row 0 with the digits in the header's page field, as a Prestel terminal's top line showed `*123`, or by setting the cursor with echo. The reference channel `neoteletext-site terminal` and `neottx view` implement both conventions, so authors have the pattern to copy.

## 6. The key record

Fourteen bytes, so a key costs nothing to parse and a microcontroller can send one:

```
byte 0     version (1)
byte 1     kind
bytes 2-3  sequence number, big-endian, from 1, wrapping
bytes 4-7  the browser's clock when the key happened, milliseconds, wrapping
bytes 8-9  a
bytes 10-11 b
bytes 12-13 c
bytes 14-  text, kind 3 only: UTF-8, at most 32 bytes
```

Fourteen bytes with `c`, which only key down and key up use.

The clock is the browser's own, from any start, and only its differences mean anything: as cloud-gaming input does, it lets a channel order keys across network jitter, tell a key held 40 ms from one held 400 ms, and, against the acknowledgement, measure delay one way. It is never the time of day.

| Kind | Meaning | a | b | c |
|---|---|---|---|---|
| 1 | Key down | key code: what the key means | modifiers: 1 Shift, 2 Control, 4 Alt (Option); 16 repeat. Command never goes | physical key: where it is |
| 2 | Key up | key code | modifiers | physical key |
| 3 | Text committed by an input method or a phone | 0 | 0 | 0 |
| 4 | The viewer opened a page (or none) | page number as teletext writes it, 0x100–0x8FF, or 0 | 0 | 0 |
| 5 | A click on a cell of the open page | column 0–39 | row 0–24 | 0 |
| 6 | The keys left the page (0) or came to it (1) | 0 or 1 | 0 | 0 |
| 7 | The connection was re-made; the last sequence number sent. The host answers by pushing the open page again, so the viewer never looks at a stale screen | that number | 0 | 0 |

**Key codes** (`a`): what the key means on the viewer's layout. A printable character is its Unicode scalar value (the Basic Multilingual Plane; others arrive as kind 3); a named key is `0xF000` plus a number from one table in the engine, the W3C `KeyboardEvent.key` names numbered: the editing keys (`Enter`, `Backspace`, `Tab`, `Escape`, `Delete`, `Insert`), the arrows, `Home`, `End`, `PageUp`, `PageDown`, the F-keys 1 to 24, the numeric keypad's keys, the media keys (`MediaPlayPause`, `MediaStop`, `MediaRewind`, `MediaFastForward`, `MediaTrackPrevious`, `MediaTrackNext`, `AudioVolumeUp`, `AudioVolumeDown`, `AudioVolumeMute`), and every television key qrkeyboard's `remote` and `teletext` layouts send (`ColorF0Red` to `ColorF3Blue`, `Teletext`, `Info`, `Subtitle`, `Guide`, `Settings`, `Exit`, `ChannelUp`, `ChannelDown`, `BrowserBack`, `BrowserHome`, `ContextMenu`, `Power`). The table is written once, in the engine, and a phone's layout maps one to one.

**Physical keys** (`c`): where the key is, the W3C `KeyboardEvent.code` names numbered in the same table (`KeyW`, `Digit1`, `Numpad5`), so a game asks for the key at W's position and gets it on an AZERTY keyboard too; 0 when unknown, as from a phone.

A record is one binary WebSocket message, at most 46 bytes. At most 120 a second (qrkeyboard's relay limit, and more than any typist); beyond that, keys are dropped but never a key-up. A key held when the page closes is released.

**Sessions across connections.** A connection is a session; nothing in the records identifies a viewer across two. A channel that must know a returning viewer (a game's score, a shell's login) uses the engine's cookies, which the WebSocket upgrade carries as any request to the channel does (REQ-WEB-006; the engine is to confirm it on the upgrade), as a web application would.

## 7. The cursor record

Optional, as the cursor is. A channel that is typed into sends it to place the cursor, to turn echo on, and to say which key it has acted on; a game sends none, or sends it with no cursor only for the acknowledgement that lets the browser measure the round trip. Without it the browser draws no cursor, echoes nothing and measures nothing, and keys go exactly as before.

One packet: a record on an independent data line, packet 4/31, data channel 13 (provisional until checked against EN 300 708's allocations, as the audio design checked channel 12), carried and reassembled exactly as the audio profile's records, so one receiver serves both.

| Field | Bytes | Meaning |
|---|---|---|
| `page` | 2 | The page it is for; ignored for any other page |
| `acked` | 2 | The last key record acted on, or 0 |
| `row`, `column` | 1, 1 | The cursor: row 1–23 and column 0–39; row 255 for none |
| `width` | 1 | How far local echo may run from the column, 1–40 cells; 0 for none |
| `flags` | 1 | 1 echo; 2 masked (echo `*`); 4 block cursor rather than a bar |

A TTI page may carry it as a line, `CU,row,column,width,flags`, which the engine's encoder emits as the packet. There is no cursor in the teletext standards, X/26's reserved modes are not ours to take, and a record on a data line is the mechanism the audio profile proved.

## 8. Local echo

With `echo` set, a printable key appears in the cursor's cell at once, drawn by the interface in its own monospace face in the cursor's colour, so it is visibly provisional; the cursor moves right; Backspace removes the last echo; the echo never runs past `width` cells from the cursor's column; `masked` echoes `*`. Each echo remembers its key's sequence number and is removed when a cursor record's `acked` reaches it, since the page pushed with that record now shows the real character, or after two seconds with no answer. Left and Right within those cells move the local cursor as well; the channel's next record wins if it disagrees.

Without `echo`, nothing shows until the channel pushes: right for a password, a menu or a game.

## 9. qrkeyboard

A QR on a screen; the phone that scans it becomes that screen's keyboard, remote or gamepad, through a relay at `qrkeyboard.com`, encrypted end to end so the relay sees nothing but sizes and timing. Keys arrive as W3C names, down and up; its `teletext` layout is the colour keys, the digits and page keys. Two screens can take it:

**The channel is the screen.** The channel's server creates the qrkeyboard instance, holds its token, pairs and decrypts, and draws the QR into a page as mosaics (13 cells by 9 rows for a 25-module QR). The viewer scans the page; the phone drives the channel; the channel pushes what changed. The browser has no part: this works on `neottx`, on any receiver, and on a broadcast television. qrkeyboard ships a browser SDK only, so a channel author would otherwise have to implement its pairing and cryptography; **`neottx keyboard`** does it for them: a bridge that creates the instance, prints the QR as mosaic rows for the author's page, pairs with the phone, and forwards its keys to the channel's input socket as key records, exactly as the browser would send them. A channel then takes a phone and a browser alike.

**The browser is the screen.** From the menu, "Phone keyboard" opens a panel with the QR, an optional PIN, and one line: "Scan with your phone camera to type here. Anyone who can see this code can type." The phone's keys are then the browser's keys: they choose pages, follow links, and type into a page as section 5 describes, through the same checks. The strip shows "Phone connected" and a Finish button. Built as a `neoteletext-keyboard` process, sandboxed with the network only, which alone holds the pairing secret: it talks to the relay, pairs (the QR's key, or SPAKE2 with the PIN), decrypts, and sends each key to the trusted core as a bounded message with no modifiers. The cryptography is RustCrypto (`aes-gcm`, `hkdf`, `sha2`; `p256` for the PIN mode), each a library approval.

## 10. Latency

**Budget**, key down to character on a 60 Hz display:

| Step | Budget | How |
|---|---|---|
| Local echo on screen | One frame, 16 ms | The interface draws echoes at display rate |
| Key record on the wire | 1 ms | Three pipe writes; the fetcher sleeps until its input or a socket's reader wakes it, not a 5 ms sleep |
| The network, there and back | The network's: 1 ms on a local network, 20–40 ms across a country | `TCP_NODELAY`; one key per message |
| The channel | The author's | Push the rows that changed, with the cursor record |
| Pushed packet to screen | 20 ms | The decoder renders the page being typed into on arrival, coalesced to one render per 16 ms; the core sends the frame at once; the interface draws at the next display frame |

qrkeyboard adds the relay's two hops before the first step.

**Measured, not reasoned:** every key record has a number and the cursor record's `acked` closes the loop, when the channel sends one (a game that wants the readout sends the record with no cursor); the core keeps the send times of the last 256 unacknowledged records, never the keys, and the developer readout shows the round trip. The browser's own path is a timed test in the full gate against `neoteletext-site terminal`, a reference channel that echoes keys into a page: pass or fail numbers. The 20 ms figure is a budget, not a measurement, until that test runs: it includes the core encoding a 480-by-500 indexed PNG and the interface decoding it for every frame, which a game at twenty frames a second does forty times a second across two processes. The test pushes a page twenty times a second with both codecs in the measurement, and the budget is revised from the number before phase 2.

## 11. Processes and messages

```
keys (typing)                         key records
interface ──────▶ trusted core ───────────────────▶ fetcher ──▶ channel's WebSocket
    ▲   cursor, echo, "typing into"      ▲ cursor records
    └────────────────────────────────────┼───────── decoder ◀── pushed packets ◀── fetcher
phone ──▶ relay ──▶ neoteletext-keyboard ────────┘ keys, as the interface's own
```

| Direction | Message | Meaning |
|---|---|---|
| Interface → core | `Typed { key, page, version }` | A key while a page of a channel that takes input is open: down, up or text |
| Core → interface | `Typing { channel, on, cursor, echo, round_trip }` | Whether keys go to the channel, and the cursor record's place and flags |
| Core → fetcher | `Send(bytes)` | One key record, at most 46 bytes: the only thing a fetcher takes from the core, and it cannot change what is fetched |
| Fetcher → core | `Sent { seq }` | Written to the network, for the round-trip clock |
| Decoder → core | `Cursor { page, record }` | A cursor record from the push stream, untrusted |
| Core → decoder | `Typing(bool)` | Render the open page on arrival |
| Keyboard → core | `PhoneKey { … }` | A key from the paired phone, untrusted, bounded, no modifiers |
| Core → keyboard, interface | `Finish`; `Phone { state, qr, pin }` | End the phone's session; the panel and the strip |

The trusted core decides everything: it sends a key record only while the active tab's open page belongs to a channel that declares input and the key is not the browser's; it checks every cursor record's bounds; it treats a phone key exactly as the interface's.

## 12. Changes to the engine

As engine decisions (E.90 onward):
1. **REQ-WEB-002:** the descriptor's `"input": true`, allowed only with `push.method: "websocket"`.
2. **REQ-WEB-005:** a binary message from the browser is a key record; rows of an opened page may be pushed alone; the heartbeat. **REQ-AUD-008 amended:** audio records travel as 4/31 packets over a WebSocket, never as bare binary messages; binary means packets.
3. **Area INT:** the key record and its key-code table; the cursor record on data channel 13; the audio crate's record and fragment code shared with it.
4. **The TTI `CU` line,** kept by the writer, emitted by the encoder.
5. **The agent:** `Input::Send(bytes)` on the push socket; `Output::Cursor`; `TCP_NODELAY`; the heartbeat.
6. **The acquisition:** the page in progress can be rendered.
7. **`neottx qr`** writes a QR as mosaic rows; **`neottx keyboard`** pairs with a phone through qrkeyboard and forwards its keys to a channel's input socket as key records; **`neottx view`** types back over its WebSocket, so authors can test a channel without the browser.
8. **Decision E.38 stands:** the records are `no_std`; the WebSocket, TLS and cryptography stay out of the microcontroller crates.

## 13. Proposed requirements

To be added to `requirements.md` as section 18 once adopted.

| Number | Statement | Reason | Verification |
|---|---|---|---|
| 18.1 | The fetcher receives push over WebSocket, plain and TLS, and Server-Sent Events, with a heartbeat; pushed packets reach the decoder at once. | A channel pushes lines as a transmitter does. | Tests against `neoteletext-site` in each mode. |
| 18.2 | A key goes to a channel only while the tab is active, the open page is that channel's, and its descriptor declares input over its WebSocket; every key goes but the browser's own shortcuts, Command combinations on macOS and Control-Shift combinations elsewhere; closing the page stops it. On such a channel a plain click goes to the host and Command-click follows a link. | The open page is the terminal; the browser keeps exactly what a terminal emulator keeps, links included. | Test of each condition, Control-C and the F-keys among the keys that go; the link tests. |
| 18.3 | While keys go to a channel the strip says "Keys go to" the channel, with "Not secure" on a plain connection. | The viewer must always know who is listening. | Test; review. |
| 18.4 | The fetcher takes from the trusted core only key records of at most 46 bytes, written to the push socket; nothing it takes changes what is fetched. | Requirement 4.7's one exception, bounded. | Test; the fetcher's reader fuzzed. |
| 18.5 | A key record is sent within 1 ms of the key; the push socket has `TCP_NODELAY`; a pushed packet of the page being typed into is rendered on arrival, coalesced to the display rate, and drawn at the next frame, within 20 ms. | The latency budget. | A timed test in the full gate, pass or fail. |
| 18.6 | The trusted core checks every cursor record and passes the interface only the cursor, the field, the flags and the round trip. | The decoder is untrusted. | Test with out-of-range records. |
| 18.7 | The cursor and the echo are drawn by the interface over the open page's image, in styles no page can draw, never as an underline; an echo is only the viewer's own printable key within the record's width, provisional, removed on acknowledgement or after two seconds. | A page must not forge them; typing must not pretend. | Tests; review. |
| 18.8 | The round trip is measured from acknowledgements, for at most 256 unacknowledged records, and shown in the developer readout; no key is ever logged. | Measure; keys are private. | The test of 18.5; a log test. |
| 18.9 | The phone keyboard runs in its own sandboxed process, which alone holds the pairing secret; its keys reach the trusted core bounded and without modifiers and act exactly as local keys; its QR is shown only from the panel, with its one-line warning; one phone at a time; Finish in the strip; a dropped phone holds the screen for at most 30 s. | A phone is a keyboard, no more; whoever scans has it. | Tests of the reader and the bounds; the sandbox test; review. |
| 18.10 | All of this is behind the `interaction` feature, the phone keyboard behind `phone-keyboard`, both on by default and both compilable out. | The owner wants features able to be compiled out. | The gates build without them. |

## 14. Decisions

All taken by the owner on 20 October 2026 (decision 44), each as recommended; 2 and 10 were taken earlier the same day.

| Number | Question | Decided |
|---|---|---|
| 1 | Amend requirement 4.7 for the one exception: bounded key records on the channel's own WebSocket, while a page of the channel is open | Yes; it is the feature |
| 2 | Every key goes to the channel while a page of a channel that takes input is open, with no dialog, no click and no mode; closing the page stops it | Yes: that is a terminal, and the strip says who is listening |
| 3 | The binary key record and its key-code table in the engine (area INT) | Yes: a terminal sends bytes, and a microcontroller can |
| 4 | The cursor record on packet 4/31, data channel 13, sharing the audio profile's record code; the channel checked against EN 300 708 first | Yes |
| 5 | Render the page being typed into on arrival, coalesced to the display rate | Yes; the carousel keeps its pace |
| 6 | The cursor: a blinking bar, or a block when asked, in an amber chosen on screen | Yes |
| 7 | May a phone's keys reach the address field? | Only after the viewer puts the caret there with the pointer |
| 8 | TLS (`rustls`) and RustCrypto (`aes-gcm`, `hkdf`, `sha2`, and later `p256` for PINs) in a new keyboard process | Yes, each a library approval; ship the no-PIN mode first |
| 9 | REQ-AUD-008 resolved as "a binary WebSocket message is packets" | Yes |
| 10 | Typing over a plain `ws://` connection, with "Not secure" in the strip, or refused until TLS? | Allow it, as plain channels are allowed. **Decided by the owner on 20 October 2026: allowed, with "Not secure" in the strip.** |
| 11 | `neottx qr`, `neottx keyboard` and `neottx view`'s typing back, in the engine | Yes; without the bridge, a phone for a channel means every author implementing qrkeyboard's cryptography |
| 12 | Page selection on a channel with input follows the videotex convention (`*nnn#`, `#`, `*#`, `*00`), with three plain digits also accepted where the host has no other use for them; the browser interprets neither | Yes: it is what a terminal did, and the reference channel and `neottx view` carry the pattern |

## 15. Phases

1. **Push and latency:** WebSocket push wired and heartbeat; TLS; the decoder renders on arrival while typing; the fetcher waits in `poll`; `neoteletext-site terminal`; the timed test.
2. **Typing:** the descriptor's `input`; the key and cursor records in the engine; `Typed`, `Send`, `Cursor`; the cursor and the strip's state; local echo; the round-trip readout; `neottx view` typing back.
3. **The phone keyboard:** the keyboard process, the panel, the strip's state; `neottx qr` and `neottx keyboard`.

The channel-as-screen use of qrkeyboard needs nothing built.

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
