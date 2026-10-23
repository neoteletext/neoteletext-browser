# NeoTeletext: multiple windows

| | |
|---|---|
| **Asked** | By the owner on 23 October 2026: "must be able to open multiple windows", then, with the owner away, "do everything in the todo list ... fully implemented". |
| **Decision** | 72, taken while the owner was away (`architecture.md` section 2). |
| **Meets** | Requirement 12.8, which asked for windows from the start and was not met. |
| **Builds on** | `tab-strip.md` (decision 69): a tab dragged off the strip leaves for another window or a window of its own. |

## 1. What a window is

A window is a trusted strip, its tabs, the active one among them, and what that tab shows: its wall of pages, its page list, its open page. Every window has all of it; nothing of the strip is shared but what is the browser's own (the settings, the bookmarks and their bar, the remote control's place, history), which every window shows the same. Tabs belong to one window at a time, and a channel to one tab.

## 2. One interface process for every window

Each window is drawn by the one interface process, not by an interface process of its own.

- **On macOS a process with windows is an application** with its own Dock icon and menu bar; a process for each window would be a Dock icon for each window.
- **Nothing is gained for security.** Every window shows only what the trusted core sent it and asks only by the intents and strip input it already had; the interface's code is outside every security property but the strip's (requirement 3.9, decision 20), and a second process of the same code adds no boundary.

## 3. How the processes say which window

Two context messages, and no change to any other:

- **The trusted core to the interface: `Target(n)`.** The messages that follow are for window *n*, until the next. The trusted core sends it only when the window changes, so a run of messages for one window carries it once.
- **The interface to the trusted core: `From(n)`.** The intents that follow are from window *n*. An intent from a window not open, as one closed while its intents were on their way, is not acted on.

Three messages open and close windows: `OpenWindow(n, place)` and `CloseWindow(n)` from the trusted core, which numbers the windows, and the strip inputs `NewWindow`, `TabToWindow(tab, window, place)` and `CloseWindow` from the interface. Window 0 means "a new window" and is never a window's number; the first window is 1 (`protocol::FIRST_WINDOW`).

## 4. In the trusted core

The session holds the window at hand's tabs as before, and sets the other windows aside, each with its tabs, its active tab, its place and size and whether it is private. Switching to another window swaps them, so every tab place in a message is the window's own, as it always was. What a window shows (the frames sent, the channel told which pages are shown, and those pages) is set aside with it.

- **What a channel says goes to its own window**: a page arrived, a frame, a link, a cookie, a fetcher's word are handled with the channel's window at hand, and sent there.
- **What every window shares** (the star, the bar, the settings, the remote) is sent to the window at hand when it changes; every other window is marked out of date and sent it again when it is next at hand.
- **Sound** follows the window last used: the channel whose page that window opened sounds, as before (the engine's REQ-AUD-012). Which channels sound is sent to every window, for its tabs' marks.
- **Every window's active channel is awake** (decision 47): the window not focused still draws and updates.
- **At most 64 tabs** (`protocol::MAX_TABS`) are open in all windows together, and at most 16 windows (`MAX_WINDOWS`); past them a new tab or window is refused with a notice. Before, the trusted core had no bound, and a 65th tab could not have been sent.

## 5. Opening, closing and moving

- **New window**: Command-N on macOS, Control-N elsewhere, and the menu. It opens with the home address.
- **New private window**: the menu. Every tab opened in it is private; it is never kept with the session.
- **Move tab to new window**: the menu; or a tab dragged off the strip and let go below it, more than 24 points below the strip (`TEAR_DISTANCE`), or outside the window. The new window opens where the tab was let go, the size of the one it left. A window's only tab is not torn off: it would leave an empty window behind.
- **Into another window**: a tab let go on another window's strip joins it there, before the tab it was let go on, or at the end.
- **Closing**: the window's close button, the system's, or its last tab closed. The trusted core closes the window and its tabs; the last window's closing ends the browser, as before.
- **Closed windows** are kept with the recently closed tabs: the tabs of a window that closed with several share its number, and reopening any of them, by Shift-Command-T or the history panel, reopens them all in a window of their own, as Chrome reopens a closed window. A window that closes because its last tab closed is that tab, closed alone.

## 6. The session

`session.txt` is version 3 (`history-and-bookmarks.md` section 15.4): a `window` line begins each window, with its place and size when known, followed by its `active` line and its tabs; `closed` lines carry, after their place, the number of the window they closed with. At start the first window is the one the interface opens, and the others open after it, each where it was.

## 7. Security

Requirement 12.8: each window has its own trusted strip, drawn by the interface process from what the trusted core sent for that window; moving a tab between windows is strip input the trusted core carries out; a page's frames go only to the window whose tab shows it. `From` is the interface's word, as all strip input is: a compromised interface could make an intent act on another of the user's windows, which it could do anyway by drawing that window's strip (open decision 3). It cannot name a window that is not open, see another window's pages but through what the trusted core sent, or open more than the bounds allow.

## 8. What building settled

Built on 23 October 2026.

1. **Command-Shift-N stays "New private tab"**, as decision 57 made it; "New private window" is in the menu without a key, so that no key changes meaning.
2. **A window that closes with its last tab keeps that tab alone** in the recently closed, not as a window: there is nothing of a window to keep but the tab. Reopening it opens it in the window at hand.
3. **A tab torn off keeps whether it is private**: a private tab moved into a window that is not private stays private, and the other way round; only tabs opened in a private window are made private.
4. **Seen on macOS** (23 October 2026): Command-N opened a second window with the home page beside the first, which went on updating its channel; Command-W in it closed it and the browser went on; two windows were kept and restored at their places after the browser was stopped and started. Dragging a tab out, and the menu's items, are driven by tests, since posted pointer events do not reach the window on macOS (`TECH-DEBT.md`, "By hand").
