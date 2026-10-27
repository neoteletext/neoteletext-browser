# NeoTeletext: dragging tabs, reordering and overflow

| | |
|---|---|
| **Asked** | By the owner on 23 October 2026: "design and spec requirements and implement tab dragging and reordering and overflow handling - if you roll over the tab bar and there is overflow then a window (connected) must appear underneath and the tabs wrap into that". |
| **Decision** | 69, taken while the owner was away (`architecture.md` section 2); amended by decisions 102 (tabs of one width, the panel as a dialog) and 103 (a tab's address in its tip), 25 October 2026. |
| **Amends** | Requirement 12.1, which named drag to reorder and overflow as not built. |

## 1. What was wrong

Tabs shrink to 90 points; past that the strip stopped drawing them. A tab that did not fit could not be seen, chosen or closed, though it stayed open, kept its channel and was saved with the session. Tabs could not be moved.

## 2. Reordering by dragging

- **A press on a tab** chooses it, as before. Moved more than 4 points with the button held, it is **dragged**: it follows the pointer along the strip, and a line between two tabs shows where it will go. Released, it moves there; released where it started, or with Escape, nothing changes.
- **The order is the trusted core's** (requirement 12.1): the interface asks to move tab *a* to place *b* as strip input; the trusted core moves it, keeps the active tab active, saves the session (requirement 20.21), and sends the tabs back. The interface never orders tabs itself.
- **Close and mute buttons** on a tab do not start a drag.
- **Between windows** (the to-do's item on multiple windows): a tab dragged out of the strip and released below it opens a window of its own; released on another window's strip it joins that window. Built with multiple windows (decision 72, `multiple-windows.md`).

## 3. Overflow

- **Every tab is 230 points wide** however many there are (decision 102): a tab never shrinks, so none moves under the pointer as tabs come and go. The strip shows the tabs that fit, in order; when the active tab would not fit, it takes the last place, so the tab being shown is always in view. A **count**, "+5", follows the last tab shown, before the new-tab button.
- **When the pointer has rested on the tabs for 300 milliseconds** while tabs overflow, a **panel joined to the strip's underside** appears, with no seam between them: the tabs that did not fit wrap into it in rows, at the strip's tab width, as if the strip had grown downward. Every tab there can be chosen, closed, muted, middle-clicked closed and dragged, in the panel or back into the strip. A pointer crossing the strip on its way to the toolbar does not open it (decision 102: opening at once laid the panel over the toolbar and took the address field's click).
- **While it is open, the tabs keep their places:** a tab chosen in the panel becomes active where it is, and moves to the strip only once the panel shuts; so it stays under the pointer, and a tab dragged from the panel is drawn where the pointer has it.
- **It stands apart from the window as a dialog does** (decision 102): the strip and the panel one raised grey surface, a blue edge two points deep at the panel's foot, a shadow below it, and the window below dimmed to 55%.
- **It goes** when the pointer leaves the strip and the panel. A press on the count opens it at once, for a pad or a touch screen, and a second press shuts it until the pointer leaves the strip.
- **What it covers.** The panel lies over the toolbar and the top of the page area while it is open, as the menu's list does; a page cannot draw it, open it or press it (requirement 11.1), and it shows only the tabs' titles the trusted core sent.

## 4. A tab's address

Resting on a tab for half a second shows its tip (requirement 12.12): the tab's title, which is its host, and on a line under it the tab's whole address, in the secondary text's colour, as Chrome's tab card shows a page's title over its site (decision 103). The trusted core sends each tab's address with its title, cleaned and bounded as the strip's address is; a line break is refused on the wire, so a channel cannot add lines to the tip. The tip is the interface's, below the pointer, as every tip is; on a tab in the panel it shows the same.

## 5. Keys and screen readers

Command-Option-Left and Right on macOS, Control-Alt-Left and Right elsewhere, choose the tab before and after, whether shown or not; Command-1 to 8 choose the first eight and Command-9 the last, as in Chrome. Screen readers wait on the accessibility tree (requirement 12.9).

## 6. Requirements

Requirements 12.12 (amended), and 12.13 to 12.15.

## 7. What building settled

Built on 23 October 2026.

1. **Away from macOS the keys are Control-Alt and the arrows**, not Control-Page-Up and Page-Down: the interface's keys have no page keys, and one pair of keys on every desktop is simpler to learn.
2. **A tab pressed is chosen at once**, before any drag, as Chrome does; a drag then moves the tab already active.
3. **Seen on macOS**: 25 tabs, the strip showing 13 with the active one last and the count "+12". The panel opening as the pointer rolls on, and dragging, are driven by tests: pointer events posted to the window do not reach it on macOS, as found before (`TECH-DEBT.md`, "By hand").
4. **25 October 2026 (decisions 102 and 103):** tabs of one width, the panel's wait, its held places, the count shutting it, its raised look and the tab's address were built with tests that fail on the code before (requirements 12.12 and 12.14). The width, 230 points, is the tabs' width before, when few were open; on a 1,440-point window five fit, on the smallest window, 1,000 points, three. Seen by hand: not yet.
