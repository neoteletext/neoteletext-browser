# NeoTeletext: dragging tabs, reordering and overflow

| | |
|---|---|
| **Asked** | By the owner on 23 October 2026: "design and spec requirements and implement tab dragging and reordering and overflow handling - if you roll over the tab bar and there is overflow then a window (connected) must appear underneath and the tabs wrap into that". |
| **Decision** | 69, taken while the owner was away (`architecture.md` section 2); amended by decisions 102 (tabs of one width, the panel as a dialog) and 103 (a tab's address in its tip), 25 October 2026, and 123 (the version at the strip's right end), 28 October 2026. |
| **Amends** | Requirement 12.1, which named drag to reorder and overflow as not built. |

## 1. What was wrong

Tabs shrink to 90 points; past that the strip stopped drawing them. A tab that did not fit could not be seen, chosen or closed, though it stayed open, kept its channel and was saved with the session. Tabs could not be moved.

## 2. Reordering by dragging

- **A press on a tab** chooses it, as before. Moved more than 4 points with the button held, it is **dragged**: it follows the pointer along the strip, and a line between two tabs shows where it will go. Released, it moves there; released where it started, or with Escape, nothing changes.
- **The order is the trusted core's** (requirement 12.1): the interface asks to move tab *a* to place *b* as strip input; the trusted core moves it, keeps the active tab active, saves the session (requirement 20.21), and sends the tabs back. The interface never orders tabs itself.
- **Close and mute buttons** on a tab do not start a drag.
- **Between windows** (the to-do's item on multiple windows): a tab dragged out of the strip and released below it opens a window of its own; released on another window's strip it joins that window. Built with multiple windows (decision 72, `multiple-windows.md`).

## 3. Overflow

*Withdrawn in part on 18 November 2026 by DEC-6637:* the panel this section describes, and its code, are gone. The tabs are one row of one width that slides with easing to show the active tab whole, with the count of those not shown; a rest on the tabs, or a press on the count, slides out the tab filmstrip in the panel's place, a row of still pictures of every tab (REQ-6640). A tab's title is its channel's name, else its host (REQ-6641). What follows is the panel as it was built.


- **Every tab is 230 points wide** however many there are (decision 102): a tab never shrinks, so none moves under the pointer as tabs come and go. The strip shows the tabs that fit, in order; when the active tab would not fit, it takes the last place, so the tab being shown is always in view. A **count**, "+5", follows the last tab shown, before the new-tab button.
- **When the pointer has rested on the tabs for 300 milliseconds** while tabs overflow, a **panel joined to the strip's underside** appears, with no seam between them: the tabs that did not fit wrap into it in rows, at the strip's tab width, as if the strip had grown downward. Every tab there can be chosen, closed, muted, middle-clicked closed and dragged, in the panel or back into the strip. A pointer crossing the strip on its way to the toolbar does not open it (decision 102: opening at once laid the panel over the toolbar and took the address field's click).
- **While it is open, the tabs keep their places:** a tab chosen in the panel becomes active where it is, and moves to the strip only once the panel shuts; so it stays under the pointer, and a tab dragged from the panel is drawn where the pointer has it.
- **It stands apart from the window as a dialog does** (decision 102): the strip and the panel one raised grey surface, a blue edge two points deep at the panel's foot, a shadow below it, and the window below dimmed to 55%.
- **It goes** when the pointer leaves the strip and the panel. A press on the count opens it at once, for a pad or a touch screen, and a second press shuts it until the pointer leaves the strip.
- **What it covers.** The panel lies over the toolbar and the top of the page area while it is open, as the menu's list does; a page cannot draw it, open it or press it (requirement 11.1), and it shows only the tabs' titles the trusted core sent.

## 4. A tab's address

Resting on a tab for half a second shows its tip (requirement 12.12): the tab's title, which is its host, and on a line under it the tab's address, cut to 512 bytes as a view's is so that every tab fits one message, in the secondary text's colour, as Chrome's tab card shows a page's title over its site (decision 103). The trusted core sends each tab's address with its title, cleaned and bounded as the strip's address is; a line break is refused on the wire, so a channel cannot add lines to the tip. The tip is the interface's, below the pointer, as every tip is; on a tab in the panel it shows the same.

## 5. Keys and screen readers

Command-Option-Left and Right on macOS, Control-Alt-Left and Right elsewhere, choose the tab before and after, whether shown or not; Command-1 to 8 choose the first eight and Command-9 the last, as in Chrome. Screen readers wait on the accessibility tree (requirement 12.9).

## 6. Requirements

Requirements 12.12 (amended), 12.13 to 12.15, and 12.18.

## 7. What building settled

Built on 23 October 2026.

1. **Away from macOS the keys are Control-Alt and the arrows**, not Control-Page-Up and Page-Down: the interface's keys have no page keys, and one pair of keys on every desktop is simpler to learn.
2. **A tab pressed is chosen at once**, before any drag, as Chrome does; a drag then moves the tab already active.
3. **Seen on macOS**: 25 tabs, the strip showing 13 with the active one last and the count "+12". The panel opening as the pointer rolls on, and dragging, are driven by tests: pointer events posted to the window do not reach it on macOS, as found before (`TECH-DEBT.md`, "By hand").
4. **25 October 2026 (decisions 102 and 103):** tabs of one width, the panel's wait, its held places, the count shutting it, its raised look and the tab's address were built with tests that fail on the code before (requirements 12.12 and 12.14). The width, 230 points, is the tabs' width before, when few were open; on a 1,440-point window five fit, on the smallest window, 1,000 points, three. Seen by hand: not yet.

## 8. The browser's version

At the strip's right end, in every window, the browser shows its version and the word BETA, "0.1.0 BETA" (decision 123, requirement 12.18), on the owner's instruction of 28 October 2026.

- **The version** is the one the trusted core reports running, the installed one or, for a browser built and not installed, the one it was built as: the same report the settings panel's Updates section and the macOS About panel show, so the three cannot disagree. A development build, one compiled without `NEOTELETEXT_RELEASE_VERSION`, is built as the workspace's first two numbers and a count one higher than the last compile's (REQ-7047, kept in the ignored `native/build-number`), so after a rebuild and a restart the label says which build is running; `neoteletext --version` prints the same version and starts nothing. Until the trusted core has reported, which it does as the window opens, the label is BETA alone.
- **Its place:** before the window's own buttons on Windows and Linux, at the window's edge on macOS, whose window buttons are at the left; 16 points of room on each side; in the new-tab button's band, in the list rows' status style (12 points, small capitals) and the secondary text's grey, which reads on the strip's black and on its raised grey while the panel is open. It has no face or outline, and a press on it is the empty strip's, which drags the window.
- **The tabs keep their reach:** the label's room comes from the tabs, so where they would have reached it they overflow into the panel a tab sooner (section 3), every tab still reached by the count, the panel and the keys. Where the strip has not room for the label beside one tab, the count and the new-tab button, it is left out; it never covers a tab, the count, the new-tab button or a window button.
- **Measured, 28 October 2026,** drawn through the processor back end in Roboto on Linux: "0.1.0 BETA" is 58.5 points wide, so the label and its room take 90.5. From that width: on macOS as many tabs fit as before, five on a 1,440-point window and three on the smallest, 1,000 points. Where the browser draws the window's buttons (Windows and Linux) one tab fewer fits: four and the count on a 1,440-point window, where five fitted, and two and the count on a 1,000-point one, where three fitted, the third missing by half a point. Seen in those drawings: the label sits in grey small capitals at the right end, level with the tabs' titles, apart from the new-tab button and the window's buttons.
- **A page cannot forge or cover it:** it is drawn in the strip, which no page image is drawn over (requirement 11.1), from the trusted core's report.

## 9. A tab's sound button

A tab whose channel is sounding shows a loudspeaker, a button that mutes it (decisions 37 and 38: the owner asked for tabs that show and mute their sound as Chrome's do): with sound waves while heard, crossed out while muted, as Chrome's tab shows it. It sits before the tab's close button, where Chrome places it, 24 points wide (`TAB_SPEAKER`). A press on it does not start a drag (section 2).

## 10. The window's frame

The frame is drawn as Chrome draws it on each platform (the owner's answer of 15 October 2026, `native-interface-design.md` section 12.1). On macOS the system's own window buttons sit at the tab strip's left end and the tabs start after them, 72 points in (`FRAME.left_inset`), the room those buttons take. On Windows and Linux there is no system frame: the browser draws its own minimise, maximise and close buttons at the strip's right end, each 44 points wide (`WINDOW_BUTTON_WIDTH`), the strip drags the window, and a press within 6 points of the window's edge (`RESIZE_MARGIN`) resizes it.

## 11. Files dragged over the window

Files dropped on the window behave as Chrome's do (DEC-1549 items 2 to 5, REQ-1700), on macOS and Windows only: Linux takes no drop (DEC-1551 item 4). Where each rule was read in Chromium's source is DEC-1549's record.

- **On the tab strip,** over the left quarter of a tab the files go before it, over its middle half they replace it, over its right quarter they go after it, and over empty strip they go at the end; over the toolbar outside the address field they replace the active tab (`tab_container_impl.cc`). Files of a type known not to be teletext are removed from such a drop, as Chrome removes what it cannot show, by their names as DEC-1769 lists them; a drop left with nothing is refused and the strip says so.
- **The arrow:** while files are over the strip, an arrow 20 pixels high and wide (`ARROW`), Chrome's size, marks where they would land: above the strip pointing down, or below it pointing up when there is no room above.
- **A background tab comes forward** when the pointer rests over its middle for 500 milliseconds (`HOVER_TO_SWITCH`), Chrome's wait (`hover_tab_selector.cc`).
- **On the page area** the current tab is never replaced: each file opens its own tab, the first in front, and a file that is not teletext opens its tab and shows the refusal (REQ-1700 item 1).
- **On the address field** a drop does not navigate: the first file's address is put in the field, selected, for the viewer to press Enter (REQ-1700 item 3).
