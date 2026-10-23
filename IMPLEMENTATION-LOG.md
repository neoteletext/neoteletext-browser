# Implementation log

The questions met while implementing without the owner, and the answer taken, as `~/devel/neoteletext-common/DEVELOPMENT.md` section 3.3 asks: number, question, decision, reason. A row is amended in place or superseded by a later row that says so. Behaviour the standard leaves open is a decision (`docs/architecture.md` section 2), not a row here.

## The round of 23 October 2026: the whole to-do list, the owner away

| Row | Question | Decision | Reason |
|---|---|---|---|
| 1 | Removing fonts from channels: does the browser hold any code for them? | None to remove: the fetcher never asked for a font and no font helper was built; only the documents named them. The requirements are withdrawn (decision 66), and the engine decides its own channel-font requirements by its own process. | The owner's word was to remove them completely; what existed was the promise in the documents. |
| 2 | Do hold and mix come with Fastext, reveal and sub-pages? | Yes, both (decision 67). | The engine has them; a rotating page cannot be read without hold, and subtitle pages are made for mix; the remote control will carry them. |
| 3 | Which keys? | The engine viewer's: `r` `g` `y` `c` (and F1 to F4), `i`, `?`, `H`, `m`, Left and Right on a page with sub-pages. | One set of keys across the engine's viewer and the browser. |
| 4 | Should the address field and history show the chosen sub-page? | No: an address naming a sub-page opens there, but choosing one does not change the address or add to history (`page-controls.md` section 8 item 4). | Stepping sub-pages is like turning a dial, not a navigation; Chrome adds no history for in-page state either. |
| 5 | The receiver-drawn row 24 of `LN` pages lives in the engine's `neoteletext-app`, which the browser cannot use. | Asked the engine to move it; the keys work meanwhile from X/27/0. | Rule 2.1: the engine does the teletext. |

