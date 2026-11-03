# NeoTeletext: security analysis of the picture converter

| | |
|---|---|
| **Status** | Written 3 November 2026 with the design, before it was built, and checked against what was built the same day (section 6). |
| **Method** | That of `security-analysis.md`: what is protected, who attacks, each boundary crossed, the answers, and the risk that remains, ranked. |
| **Design** | [`picture-tools.md`](picture-tools.md) (DEC-2390). |
| **Rule** | Decision 58: no weakening or downgrading of security, and no fallback to a weaker mode. DEVELOPMENT.md section 2.3: a feature that reads outside input has its own analysis, whose findings the design takes. |
| **Builds on** | [`security-analysis.md`](security-analysis.md) sections 5.3, 5.6 and 5.8; [`sandboxing.md`](sandboxing.md) section 7.6; the engine's REQ-2330 (bounded decoding) and its fuzz target `picture_convert`. |

## 1. What is new

Three things the browser did not do before:

1. **It reads a new kind of untrusted file: pictures.** PNG, JPEG, GIF, BMP and WebP have large, compressed, often-attacked decoders. These are the `image` crate's pure Rust decoders (`png`, `zune-jpeg`, `gif`, `image-webp`, and `image`'s own BMP reader), with `moxcms` for colour profiles; none is C, and none uses threads with the crate's default features off.
2. **It writes a file the user names.** Until now the confined trusted core wrote only its own settings folder and the broker wrote nothing for it.
3. **It shows a page the browser made itself from outside input** at a built-in address.

## 2. What is protected

As `security-analysis.md` section 2, and in particular: the user's files (none read or written but the one picture chosen and the one place chosen to save); the trusted core's state; the trusted strip's truth (a converted page must not pass for a channel's); and availability (a hostile picture must not hang or exhaust the browser).

## 3. Attackers

- **A hostile picture**, from anywhere: a mail attachment, a download, a web page the user saved. It may be malformed, a decompression bomb, claim enormous dimensions, carry a hostile colour profile or orientation tag, or be a polyglot that is also a TTI file.
- **A compromised decoder**, after a fault in a picture decoder: it answers the trusted core with anything it likes.
- **A compromised trusted core**, as in `security-analysis.md` section 5.8, now with two new broker requests.
- **A compromised interface**, as in section 5.6, now with a new kind of strip input.

## 4. Boundaries and answers

### 4.1 The picture into the browser (new)

| Threat | Answer |
|---|---|
| A fault in a picture decoder, reached by a crafted file | The picture is decoded only in a decoder process started for it alone (`--picture`), confined as every decoder is: no files, no network, no programs, its pipes only, on every platform (requirement 3.4, decision 58). The trusted core, the broker and the interface link no picture decoder. The decoders are memory-safe Rust; the engine's library denies panicking constructs on the picture's sizes and is fuzzed (`picture_convert`, 37,405 inputs on 3 November 2026 without a fault). |
| A decompression bomb or a header claiming huge dimensions | The engine bounds the file at 64 MB, each side at 32,768 pixels, and the decoded size at 512 MB, checked from the header before any pixel is decoded (its REQ-2330). Its own test found that the `image` crate's memory limit let a 30,000 by 30,000 header through to decoding, so the engine checks the decoded size itself first. The broker reads at most 64 MB, and the trusted core's channel to the decoder carries it in 64 KB pieces. |
| A picture that takes long to convert | The decoder is killed after 30 seconds and the panel says so (`PICTURE_DEADLINE`); a conversion took at most 1.5 seconds in a release build. Each conversion has a generation, so a late answer changes nothing. |
| A polyglot: a file that is both a picture and a TTI page | The broker's picture read and its channel read are separate: a picture read never opens a channel, and a channel read of the same file is refused unless the engine recognises it as teletext, as before. The picture reaches only the picture decoder. |
| The broker's filter misjudging a file | The filter only lets a file through to a confined decoder: it is a narrowing, not a safeguard. It looks at the first twelve bytes and parses nothing. |

### 4.2 The decoder's answer into the trusted core (boundary 3)

| Threat | Answer |
|---|---|
| A compromised picture decoder sends a hostile page | The page is bytes the trusted core never parses: it goes, as a file channel's do, to a channel's own decoder at `neoteletext:picture`, which reads it with the engine's bounded, fuzzed TTI reader. A hostile page can draw anything in that tab, as any channel can; it cannot reach the strip. |
| Oversized or malformed answers | Each form at most 96 KB and the summary at most 200 bytes, read strictly by `protocol::picture`, fuzzed through the decoder message target; anything else ends the exchange with an error. |
| False words | The summary and the refusal are cleaned of hidden characters and bounded before the panel shows them, and they are shown only in the panel, never in the strip. |
| A channel's decoder claiming a converted picture | Ignored and logged: only the decoder started with `--picture` is read for one (`decoder_messages.rs`). A channel's decoder that is sent picture choices refuses them and ends. |

### 4.3 The interface's choices (boundary 6)

The choices are five bytes, each checked against its list by the protocol and again by the trusted core. A compromised interface can choose any picture conversion, open the picture and save dialogs (as a compromised interface could already open the file dialog), and nothing more: it never names a path, and the dialogs are the system's, shown by the chooser with titles fixed in the chooser.

### 4.4 The trusted core and the broker (boundary 10)

| Threat | Answer |
|---|---|
| A compromised core reads the user's files as "pictures" | The broker reads only the picture last chosen in the picture dialog, by its canonical path, not through a link, not in the dot-folders, applications' data or the browser's own folders, and only when it starts as a picture. A choice in the picture dialog grants nothing else: it is not a channel grant. What a compromised core can read is what the user chose to convert. |
| A compromised core writes the user's files | The broker writes only to the place the system's save dialog returned, of the kind the dialog was for, once, and then forgets it; the core never names a path. The system's dialog asked before replacing a file. A link in that place is refused rather than followed (`O_NOFOLLOW` on Unix). At most 256 KB. What a compromised core can write is one file of its choosing where the user just chose to save, which is what saving is. |
| A fault in the broker's new parsers | Two fixed requests: a path, read as the other paths are, and a kind byte with bytes bounded by length; both in the broker's request reader, which is fuzzed (`broker_requests`). |

### 4.5 The converted page and spoofing

The page shows at `neoteletext:picture`, which the strip marks as the browser's own, as it marks `neoteletext:home`; the strip names its channel "Converted picture". A picture can be made to look like any page, as a channel's can; it cannot change the strip, which is outside the page area (requirements 11.1 to 11.4). The address is not kept in history or bookmarks, and the page is not kept after the browser closes.

## 5. Risks that remain, ranked

| Rank | Risk | Likelihood | Impact | Main controls |
|---|---|---|---|---|
| 1 | A fault in a picture decoder | Low to medium: picture decoders are a common target, though these are memory-safe | Contained to the confined picture decoder, which ends with its one answer | Process confinement on every platform, bounds before decoding, fuzzing, the deadline |
| 2 | A compromised core and the save request | Low (needs a fault in the trusted core) | One file written where the user had just chosen to save | The broker writes only the save dialog's choice, once, of its kind, no links |
| 3 | A hostile picture made to look like a channel's page | Low | A misleading page in a tab marked as the browser's own | The strip names it "Converted picture" at a built-in address |

## 6. Checked against what was built, 3 November 2026

- The trusted core, the broker, the interface and the chooser link no picture decoder: `neoteletext-picture` is a dependency of `neoteletext-decoder` alone (`native/decoder/Cargo.toml`).
- Tests: the protocol's bounds (`picture_messages_are_bounded`, the round trips); the broker's read and save (`only_the_chosen_picture_is_read_and_only_as_a_picture`, `a_page_is_saved_once_where_chosen`); the trusted core's handling (`a_converted_page_opens_at_its_built_in_address`, `an_older_answer_is_ignored_and_words_are_cleaned`, `nothing_is_saved_before_a_page_is_made`, `the_converted_pages_address_is_built_in`); the decoder (`a_picture_becomes_page_100_in_both_forms`, `every_choice_is_a_valid_conversion`, `bytes_of_no_picture_are_refused`); the panel (`a_choice_is_sent_to_the_trusted_core`, `saving_waits_for_a_converted_page`, `hold_mosaics_belongs_to_level_15`); the hostile harness accepts a decoder started with `--picture` and no other new argument.
- Fuzzing: the decoder and broker message targets read the new kinds; the engine's `picture_convert` target covers the decoding.
