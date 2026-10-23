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
| 6 | The to-do said the remote's design would be shown to the owner before building; the owner then asked for everything built while away. | Designed (`remote-control.md`) and built; for the owner to confirm (decision 68). | The later instruction governs; the design is recorded for review. |
| 7 | Should the floating remote be a window of its own? | No: a panel inside the browser's window, held below the strip. | A separate window can be covered or imitated by another program (requirement 11.1's reasoning); with multiple windows each window gets its own remote. |
| 8 | Which key shows the remote? | Command-Shift-R (Control-Shift-R elsewhere). | Command-R is Reload; the shifted letter keeps the remote beside it and clashes with nothing the browser takes. |
| 9 | Overflow: which tabs stay on the strip? | The first that fit, the active one taking the last place when it would not; the rest wrap into the panel in order. | The tab being shown must always be in view; Chrome keeps it in view too. |
| 10 | Does the panel cover the toolbar? | Yes, while the pointer is on the strip or the panel, as the menu's list does. | The owner asked for a panel joined underneath; it is the interface's own, which no page can draw or press. |
| 11 | The latency test fails on a loaded machine: loosen its budget, or run it alone? | Run it alone (nextest `threads-required` for it) and retry it twice (`native/.config/nextest.toml`). | The 2 ms budget is what tells a woken fetcher (0.3 ms) from a polling one (6.3 ms); loosening it would let polling pass, while polling fails every retry. |
| 12 | The review of tips: which controls lacked one? | Only a zoomed page's close button; given "Back to all the pages (Escape)". | Every other icon, symbol or cut text had one by then. |
| 13 | The three `zbus` advisories: has the Linux keyring client moved to `zbus` 6? | No (`secret-service` 5.2.0 is the newest, on `zbus` 5): the narrow exceptions stay, the item waits. | Nothing to do until the client moves. |
| 14 | "console.log style tab": what does it show, with no scripts? | The browser's own log, with levels and sources, filtered and searched, absorbing the Notices tab; and, when the engine carries them, the channel's own log lines (decision 70). | Notices were already a partial log; one Log tab, as a web browser's console, is what the owner described. |
| 15 | Where do browser-wide lines (confinement, settings) show, since the console is per channel? | In every channel's Log tab, in order among the channel's own. | A web browser's console shows browser messages on every page; the line is the same whichever tab is open. |

