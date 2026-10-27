# NeoTeletext: security analysis of push, typing back and the cursor

| | |
|---|---|
| **Status** | Written 20 October 2026 against [`interaction.md`](interaction.md) as proposed, and revised with it when the design was simplified to the terminal model. Section 9 records what building changed. Since 21 October 2026 the input system's own analysis is `~/devel/neoteletext-common/INPUT.md` section 11 (decision 48), which takes this one's threats over for protocol version 2; this document stands for the browser's processes and boundaries. |
| **Method** | The method of [`security-analysis.md`](security-analysis.md): what is protected, who attacks, each boundary the feature crosses, the mitigations, and the remaining risk ranked. |

---

## 1. Summary

The browser has only ever sent a channel requests for its files. This feature sends it **the viewer's keystrokes**, lets a channel **place a cursor and ask for echo**, and, last, lets **a phone across the internet be the keyboard** through a third party's relay. Keystrokes are the most sensitive thing a browser handles.

The design holds because of four properties:
1. **Keys go only where the viewer sat down:** to the channel of the page the viewer opened, while the strip says "Keys go to …". No page, record or message can start it; only the viewer opening a page of a channel that declares input. This is a terminal's rule, and no weaker: a terminal window with focus sends every key to its host.
2. **What leaves is bounded and dumb:** a 14-byte record of the viewer's own key, where it is on the keyboard, and the browser's own clock, written to the channel's own WebSocket. The fetcher takes nothing else from the trusted core; fetching is unchanged.
3. **What comes back is drawn, never obeyed:** a cursor record places a cursor and clears an echo; it cannot request, navigate or change a setting. The cursor and the echo are overlays no page can draw.
4. **A phone is a keyboard and no more:** its keys do what the local keyboard's do, through the same checks, from a process that holds the pairing secret and nothing else.

The largest remaining risk is **keystrokes over a plain `ws://` connection** until TLS exists, which decision 10 of the design puts to the owner.

## 2. What is protected

Beyond `security-analysis.md` section 2: the viewer's keystrokes (where they go, who sees them); the viewer's reading (which pages they open, which typing now tells the channel); the screen's truth (a cursor or an echo means what the browser says); the trusted core (two new parsers: cursor records from the decoder, keys from the keyboard process); the browser's own keys (shortcuts, the strip, Escape); and the pairing secret of a phone.

## 3. Who attacks

A channel author with every byte of the push stream and the descriptor; a network attacker on a plain connection; a compromised fetcher, decoder, interface or keyboard process; a hostile relay; anyone who can see the screen; a person with a phone who scanned a QR they were not meant to. They want: the viewer's keys; to type as the viewer; to make the viewer type into the wrong place; to make the browser fetch or navigate; to learn what the viewer reads.

## 4. The boundaries

```
A              B                 C/D                 E / F
channel ─────▶ fetcher ────────▶ trusted core ◀───── decoder (cursor records)
  ▲  push        │  pushed bytes    │  ▲                 ▲ pushed packets
  └─ key records ┴◀── Send ─────────┘  │ Typed
                                       │
phone ──▶ relay ──▶ neoteletext-keyboard ── PhoneKey
```

### A. The channel into the fetcher

| Threat | Mitigation |
|---|---|
| `input` declared without a WebSocket, or a push address off the channel's origin | Refused by the descriptor parser, as `push` is today |
| Cursor records in a push stream, a file or a broadcast | Decoded and dropped unless the channel declares input over a WebSocket and the page is open |
| A push flood, now that the decoder renders on arrival | Coalesced to one render per 16 ms per page; the 64 KB message limit stands |
| A hijacked socket after reconnection | The resume record carries only a sequence number; there is no token to steal. A viewer's identity across connections is the engine's cookies, with the engine's rules (REQ-WEB-006), never anything in the records |
| A plain `ws://` connection | "Not secure" in the strip beside "Keys go to", as the address field shows it; decision 10 |

### B. The fetcher and the trusted core

| Threat | Mitigation |
|---|---|
| `Send` influences fetching (requirement 4.7) | It is the only kind the fetcher reads; at most 46 bytes; written to the push socket and nothing else; it names no address, page or connection |
| A compromised fetcher sends keys it was not given | It has none: the core gives it a record only under section 5 of the design |
| The fetcher's new reader | Panic-free (the no-panic lint list) and fuzzed (`fetcher_commands`): input messages only, read by the engine's input stream reader (INPUT.md, REQ-INT-133) |
| A false `Sent { seq }` | Only the round-trip clock reads it, bounded to 256 entries |

### C. The decoder into the trusted core

| Threat | Mitigation |
|---|---|
| A cursor record outside its bounds | Every field checked again (row 1–23, column and width within 40, known flags, a page of the channel); the rest dropped |
| A cursor to deceive: over a link, over the strip | It cannot leave rows 1–23 of the page image; it takes no clicks itself; the strip is outside the page area |
| `echo` to make the browser show characters | Only the viewer's own printable keys, only in the field, only while typing; a page already shows what the channel pushes |
| A false `acked` | Removes an echo of the viewer's own character early; the pushed page then shows what the channel chose anyway |

### D. The interface and the trusted core

| Threat | Mitigation |
|---|---|
| **A page lures the viewer into sending keys.** | Nothing a page does starts it: keys go to a channel only because its descriptor declares input and the viewer opened one of its pages, which the strip names. A page cannot declare input, and a channel that does gets keys from its own pages only, which it could already read on its own server |
| Keys typed in the strip, the list, the overview, a background tab | The interface sends `Typed` only from the open page; the core checks the active tab, the page and the channel again |
| Shortcuts captured by a channel | Command combinations on macOS and Control-Shift combinations elsewhere never go, exactly the keys a terminal emulator keeps; Control-C and the F-keys go, as from a terminal; the close button, a click outside the page or the browser's close shortcut is the way out |
| The viewer forgets where the keys go | "Keys go to adventure.example" in the strip the whole time; closing the page, the tab or the channel stops it |
| Page numbers typed into a game | They go to the channel, as to a Prestel host, which takes `*nnn#` by convention (design section 5.1); on a channel without input the browser takes three digits locally as a teletext set does. The viewer who wants the browser's own navigation closes the page, as they would leave a terminal |
| A browser link on a terminal page, and a click the host also wants | Shared as terminals share it: a plain click is the host's, Command-click is the link's, and the strip names the destination either way; a link followed tells the host the page changed, as any page change does |
| The physical key code as a fingerprint of the keyboard | It says which position a key is at, as `KeyboardEvent.code` does on every web page; the layout it implies is already implied by the characters typed |
| Typing rhythm as a side channel | The channel sees arrival times, as any terminal host does, and now the browser's own millisecond clock on each key, which is the same information without the network's jitter; it is relative to an arbitrary start and is never the time of day; stated in section 6 |
| **A compromised interface** sends `Typed` for keys never pressed | It can type into the channel whose page is open, as the viewer: the existing residual of open decision 3, bounded to that channel. It cannot type into another, since the core requires the open page's channel to declare input |

### E. The phone, the relay and the keyboard process

| Threat | Mitigation |
|---|---|
| The keyboard process attacked through what it parses: relay JSON, frames, ciphertext, decrypted JSON | Its own process, sandboxed with the network only; no files, pages or cookies; the no-panic list; its readers fuzzed; it holds only the instance's token and the pairing keys |
| A passive relay or network observer reads keys | It cannot: AES-256-GCM end to end from a secret it never sees |
| The relay's operator, or whoever compromises its server, reads the PIN and the keys (finding F-18) | Not prevented: the relay serves the phone's page, the code that encrypts, so it can serve code that sends the PIN and the keys elsewhere. Bounded by section F: a hostile phone is a hostile local keyboard with no modifiers, which outside a page open for typing only moves between pages. Serving the phone's page apart from the relay would close it, and qrkeyboard.com is not this repository's server (decision 96) |
| A hostile relay relabels the phone's keys or renames the screen | qrkeyboard's configuration hash in the QR, computed as its SDK computes it |
| Replay or reordering by the relay | Counter nonces accepted once and in order; fresh salts per connection |
| Anyone on the relay sends a salt and the screen says a phone is connected (finding F-71) | Without a PIN a phone is connected only by its first frame that opens under the QR's key, its first key (the engine's decision E.116 item 3), and the panel says to press one (decision 101); with a PIN, by the PIN run's confirmation |
| **Whoever photographs the QR has the keyboard** | The QR appears only from the panel, with its one-line warning; a PIN is offered; one phone; Finish in the strip; a dropped phone holds the screen 30 s, not qrkeyboard's default 5 minutes |
| TLS to the relay | The browser's first TLS; a certificate failure is a hard stop (requirement 7.11); the relay's host is fixed and never taken from a page |
| New cryptography dependencies | `cargo-deny`; pinned; the no-PIN mode needs no `p256`, so PIN pairing is a separate approval |

### F. The keyboard process into the trusted core

| Threat | Mitigation |
|---|---|
| A compromised keyboard process types anything | Into a page open for typing it is a compromised local keyboard: no more than a hostile person at the keys. Outside one it only moves between pages (decision 99): the trusted core passes on only the page-navigation keys, and the interface gives them to the page view alone (`a_phone_outside_a_typing_page_only_moves_between_pages`) |
| Modifiers from the phone pressing shortcuts | A phone key carries no modifier; a message claiming one is dropped |
| Phone keys into the address field, a panel, or the strip's question (finding F-20: typing an address, changing the proxy, or moving to Go and confirming a hand-off or a change of site) | Never: the core's filter holds a phone to the digits, the arrows and the colour keys outside a page open for typing, and does not depend on the interface's code (decision 20); the interface drops phone keys while a field is edited, the question is asked, or a panel, popover or list is open. Not taken: refusing a confirmation within a delay after a phone key, which a keyboard that waits defeats |
| A flood | 120 a second at the relay, the process and the core; key-ups kept |

## 5. Spoofing

A page can draw a block in a cell, not a one-pixel bar or the interface's provisional face; and nothing it draws makes the strip say "Keys go to …". A page can draw a QR: that is the channel-as-screen pattern, and the phone that scans it talks to the channel, never to the browser; the browser's own QR is only ever in its panel. Visual discipline remains the medium risk it always is.

## 6. Privacy

Typing tells the channel the viewer's keys and the pages they open, which is what typing into a page means; the strip says so. Keys are never logged: the round-trip clock keeps numbers and times. The phone keyboard tells the relay the screen's name, the layout, addresses, sizes and timing, and nothing else. Nothing persists past the window.

## 7. Remaining risks, ranked

| Rank | Risk | Likelihood | Consequence | What bounds it |
|---|---|---|---|---|
| 1 | Keystrokes over a plain `ws://` connection, read or injected on the network | High on a public network, until TLS | High: the viewer's typing to that channel | "Not secure" beside "Keys go to"; decision 10; TLS in phase 1 |
| 2 | A compromised interface types into the channel whose page is open | Low | Medium: that channel only | Open decision 3 |
| 3 | The keyboard process taken through what it parses | Low | Medium: a compromised local keyboard | Sandbox; no panics; fuzzing |
| 4 | A QR photographed from a public screen | Medium on a public screen | Medium: a stranger types | PIN; one phone; 30 s; Finish; the warning |
| 5 | A page imitates the cursor, the echo or the strip's words | Medium | Low: nothing happens without the strip | The strip; the hairline; the provisional face |
| 6 | The relay's operator, or a compromise of its server, serves the phone code that reads the PIN and the keys (finding F-18) | Low | Medium: the keys typed on the phone, and a hostile keyboard | Section F's bounds; the PIN; one phone; decision 96 |

## 8. Checks that keep these properties

- **"Nothing else sends":** every message kind and every key, driven with no page open, with a page of a channel without input open, in a background tab, and in the strip, never produces a `Send`; with a page of a channel with input open, only `Typed` from that page does, and never a browser shortcut.
- **Bounds:** a `Send` over 46 bytes refused; `PhoneKey` with a modifier, over 32 bytes, or text over 995 refused; the 121st key in a second dropped and never a key-up; the clock holds 256.
- **Cursor records:** every field's bounds; a record for a page not open or a channel without input changes nothing; `acked` out of order; a flood coalesced.
- **Fuzz targets:** `fetcher_commands`, `input_records` (cursor and control records), `phone_keys`, and the keyboard process's relay and frame readers, in the full gate; the engine fuzzes the input stream and the control record itself.
- **Log test:** no key code or text in the log or the readout.
- **Phone:** the configuration hash matches a fixture from `~/devel/qrkeyboard`; a reused counter refused; five wrong PINs lock; Finish ends the session.
- **Review** of the strip's "Keys go to" state and of the panel, against the spoofing list (requirement 14.5).
- **Compiled out:** builds and tests without `interaction` and without `phone-keyboard`.

## 9. What building changed

- **A cursor record may show another page of the same channel** (`interaction.md` section 17 item 7). It cannot leave the channel, fetch, or act while keys do not go to the channel; the viewer's own choice of page overrides it. A hostile channel can move the viewer among its own pages, which it could already do by pushing what it likes into the page open. Rank 5's spoofing risk is unchanged.
- **The fetcher reads input messages only**, one message kind, whole items of the input stream checked by the engine's reader before the agent writes it, which itself drops anything over 4 096 bytes or for a channel that does not declare input (INPUT.md, REQ-INT-133). Fuzzed as `fetcher_commands`.
- **Phone keys travel core to interface,** which acts on them as its own keys while a page is open for typing, and otherwise gives only the page-navigation keys to the page view (decision 99, finding F-20, 24 October 2026); the interface's messages back are the same intents a keyboard makes, checked by the trusted core as always. A phone key has no modifier by construction: the keyboard process's message has no field for one. Fuzzed as `phone_keys` and, for the keyboard process's readers, `relay_messages`.
- **Remaining risk 1 is narrowed:** TLS (`rustls`) is built, so a channel on `wss://` types encrypted; a plain `ws://` channel still shows "Not secure".

## 10. Controls (decision 117)

Controls (INPUT.md section 7.3) change nothing about where input goes: a click on one is the MOUSE event every click over the page sends, at its cell, and while keys do not go to the channel a page shows none. What they add, the control rectangles from the decoder and the look the interface asks for, is analysed in `hyperlinks-security.md` section 16. The browser draws the looks itself and sends the channel nothing for them.

## 11. No input modes (decision 118)

Since decision 118 a channel learns every press and release of the pointer over its own page while keys go to it, with the cell and the dot; its drags; and its plain moves only within the pointer areas its page declares (INPUT.md section 7.4), which the trusted core checks itself, so a compromised interface cannot widen them. It learns nothing of the pointer anywhere else: not over the browser's own parts, another tab, or a page whose channel keys do not go to. Moves are sent only when the dot changes and are not counted against the rate of REQ-INT-109, so a moving pointer can never crowd out a key. Every key now goes as KEY with its text, releases included; what a channel learns of typing rhythm is unchanged, since it already received every key.

