# NeoTeletext: the Tools menu's picture converter

| | |
|---|---|
| **Asked** | The owner, 3 November 2026, while away and wanting no questions: "you must find the rust code recently written which converts images to teletext and i want that integrated into the browser on a "tools" menu which can read images, convert and save them". |
| **Decision** | DEC-2390 of this repository's records in rdm, waiting for the owner's confirmation; the engine's DEC-2325 made the converter a library for it. |
| **Security** | [`picture-tools-security.md`](picture-tools-security.md), whose findings this design takes. |
| **Builds on** | The engine's `neoteletext-picture` (its REQ-2327 to REQ-2337), the decoder process (requirement 3.4), the bookmark reader's pattern of a decoder started for one file (decision 71), the file dialog and the broker's grants (DEC-1551, REQ-1698), the start page's built-in address (requirement 19.7). |

## 1. What it does

A Tools menu (on macOS a menu of the menu bar; on Windows and Linux an item of the menu button's menu, "Tools: convert picture to teletext…"; Command-Shift-P or Control-Shift-P everywhere) opens the picture panel at the right of the window. The panel has:

- **Choose picture…**: the system's open dialog, titled "Convert Picture to Teletext", filtered to PNG, JPEG, GIF, BMP and WebP pictures.
- **Presentation level**: 1.5, 2.5 or 3.5 (Level 1 encodes exactly as 1.5).
- **Fit**: crop, contain or stretch.
- **Alignment**: the nine places, centre by default.
- **Background**: the eight Level 1 colours, black by default; it fills the bars and transparent areas.
- **Hold mosaics**: off or on, at Level 1.5 only.
- **Save as TTI…** and **Save as T42…**: the system's save dialog, offered once a page is made.
- A line saying what is happening, what the converter made, why it made nothing, or where the page was saved.

Every choice converts the picture again at once. The page made opens in the active tab at `neoteletext:picture#100`, drawn by the browser's own decoder and renderer like any channel's page, so it shows exactly as a receiver would show it. Objects are never used: the browser's decoders draw with the engine's `modern-classic` profile, which has no objects, so Levels 2.5 and 3.5 keep their colour changes in the page's own packets X/26 (the engine's DEC-2325 item 9).

## 2. Where each part runs

```
user ──► interface (panel) ──[6]──► trusted core ──[10]──► broker ──► chooser (open or save dialog)
                                         │                     │
                                         │◄── Chosen(path) ────┘
                                         ├──[10]──► broker: read the chosen picture (bytes)
                                         ├──[3]──► decoder --picture: choices + picture ──► TTI, T42, summary
                                         ├──► neoteletext:picture: the TTI to a channel's decoder, unread
                                         └──[10]──► broker: write the page where the user chose
```

1. **The interface** draws the panel and sends each choice as strip input (`ToCore::Picture`), and the dialogs' requests as `ToCore::OpenFile` with the kinds `Picture`, `SavePictureTti` and `SavePictureT42`. It converts and saves nothing.
2. **The trusted core** checks each choice, asks the broker for the dialog, and on the choice asks the broker to read the picture. It never parses the picture or the page: it hands the picture to a decoder started with `--picture` and the page made to a channel's decoder, each as bytes (requirement 3.2).
3. **The broker** shows the dialog through the chooser, remembers the picture chosen and reads only that file, only when it starts as one of the five formats, at most 64 MB (the engine converter's `MAX_INPUT_BYTES`, asserted equal to `protocol::picture::MAX_PICTURE_FILE`); a choice in the picture dialog grants nothing else. A save dialog's choice is remembered as one place of one kind, written once with the bytes the core sends, then forgotten; a link in its place is refused.
4. **A decoder started with `--picture`**, confined as every decoder is (no files, no network, no programs), takes the choices and then the picture as data, converts it with `neoteletext-picture`, and answers once with the TTI page file, its T42 packets and the converter's summary, or with why it made none. It is killed if it has not answered within 30 seconds.
5. **The page** is held by the trusted core while the browser runs and shown at `neoteletext:picture`, a built-in address like `neoteletext:home`, which history and bookmarks do not keep.

## 3. Messages

| Pipe | Message | Kind | Bound |
|---|---|---|---|
| Interface to trusted core | `ToCore::Picture(PictureChoice)`, strip input | 148 | Five bytes, each index within its list; hold mosaics at Level 1.5 only |
| Interface to trusted core | `ToCore::OpenFile` kinds 4, 5 and 6: picture, save TTI, save T42 | 123 | As before |
| Trusted core to interface | `ToInterface::PictureStatus { converted, text }` | 149 | Cleaned text, at most 200 bytes |
| Trusted core to decoder | `ToDecoder::Picture(PictureChoice)`, only to a decoder started with `--picture` | 145 | As above; a channel's decoder refuses it and ends |
| Decoder to trusted core | `FromDecoder::PictureConverted { tti, t42, summary }` | 146 | Each form at most 96 KB; the summary at most 200 bytes, cleaned by the core |
| Decoder to trusted core | `FromDecoder::PictureRefused(reason)` | 147 | At most 200 bytes, cleaned by the core |
| Trusted core to broker | `Request::ReadPicture(path)` | 20 | A path; answered only for the picture last chosen |
| Trusted core to broker | `Request::Save { kind, bytes }` | 21 | A save kind; at most 256 KB |

A channel's decoder that sends either picture answer is ignored and logged; only the decoder started for the picture is read for one.

## 4. Requirements

The requirements are in rdm: REQ-2392 (the menu and the panel), REQ-2393 (conversion in a decoder and the page at `neoteletext:picture`), REQ-2394 (the broker's picture read) and REQ-2395 (saving), with their states.

## 5. What building settled

Recorded in DEC-2390 for the owner to confirm: the panel at the right of the window rather than a dialog over the page, so the page shows beside it; conversion at once on every choice; the page at a built-in address in the active tab; objects never used; the save written by the broker to the place the system's save dialog chose, once; the 30-second deadline; Level 1 left out of the panel.
