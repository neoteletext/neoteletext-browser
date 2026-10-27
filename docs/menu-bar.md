# NeoTeletext: the menu bar, and copy and paste in the browser's fields

| | |
|---|---|
| **Asked** | By the owner on 23 October 2026: "Native menu bar (File, Edit, View) is this needed if yes then build it". |
| **Decision** | 84 (`architecture.md` section 2). |
| **Amends** | Requirement 12.5 (menus and shortcuts) and requirement 12.11 (the clipboard in the browser's fields). |
| **Security analysis** | `menu-bar-security.md`. |

## 1. Is it needed?

**On macOS, yes.** Every Mac application has a menu bar at the top of the screen, and the system expects one:

- **It is where a Mac user looks for a command and its key.** The browser's keys were learnable only from the menu button and the tips.
- **The Window menu lists the open windows,** which the system fills in itself; with several windows (decision 72) it is how a Mac user moves between them.
- **The Edit menu is how copy and paste are found,** and with it the browser's text fields gain them (section 4).
- **Hide, Hide Others, Show All and Quit** belong to the application menu; before this, the menu bar held only what winit gives by default.
- **Menus are reachable from the keyboard and by VoiceOver** (Control-F2 opens the menu bar), which no drawn control of the window is yet (requirement 12.9).

**On Windows and Linux, no.** Chrome, Edge and Firefox show no menu bar there by default; the menu button's menu (decision 57) is the counterpart, and copy and paste work in the fields with Control-C, Control-X and Control-V (section 4). Requirement 12.5 is amended to say so.

## 2. The menus

Each item does exactly what its key or button does: the menu bar sends the interface core the key it names, so there is one path for both (requirement 12.5, "menu items and buttons must send the same intents").

| Menu | Items (keys) |
|---|---|
| **NeoTeletext** | About NeoTeletext; Settings… (Command-,); Hide NeoTeletext (Command-H), Hide Others (Option-Command-H), Show All; Quit NeoTeletext (Command-Q) |
| **File** | New Tab (Command-T), New Window (Command-N), New Private Tab (Shift-Command-N), New Private Window; Open Location (Command-L); Reopen Closed Tab (Shift-Command-T); Close Tab (Command-W), Close Window (Shift-Command-W) |
| **Edit** | Cut (Command-X), Copy (Command-C), Paste (Command-V), Select All (Command-A) |
| **View** | Reload (Command-R); Enter Full Screen (Control-Command-F), the page alone (decision 85), titled Exit Full Screen while the page is full screen; Show or Hide the Bookmarks Bar (Shift-Command-B); Remote Control (Shift-Command-R); Developer Console (Option-Command-I) |
| **History** | Back (Command-[), Forward (Command-]), Home (Shift-Command-H); Show All History (Command-Y) |
| **Bookmarks** | Bookmark This Page (Command-D), Bookmark Manager (Option-Command-B) |
| **Window** | Minimise (Command-M), Zoom; Show Previous Tab (Option-Command-Left), Show Next Tab (Option-Command-Right); Move Tab to New Window; then the open windows, which the system lists |

**Command-H is the system's Hide**, as in every Mac application; history stays on Command-Y, its key since decision 57. Open Location focuses the address field and selects its text, as Command-L does in every browser.

## 3. While keys go to a page

On a channel that takes input, keys go to the channel (decision 48), but the browser's own modifier, Command on macOS, keeps its keys for the browser. The menu bar's keys are all Command keys, so nothing a channel is typed into changes. Paste while keys go to a channel pastes into the channel, as Command-V did (decision 49).

## 4. Copy and paste in the browser's fields

Every text field the browser draws (the address field, the history and bookmarks searches, the bookmarks manager's and the star's forms, the settings panel's fields, the console's and cookies' searches) gains:

- **Cut, Copy and Paste** of its selection, through the system clipboard;
- **Select All**, which some had already.

On macOS the Edit menu's items and their Command keys do it; on Windows and Linux, Control-X, Control-C, Control-V and Control-A. Pasted text passes through the field's own rules: hidden characters dropped and its length bounded, as typed text is. A paste goes nowhere when no field has the keys and no channel takes them.

The clipboard is read and written by the interface process, which already reads it to paste into a channel (decision 49, the `arboard` library). Nothing the trusted core holds is copied: the address field shows only what the strip already shows.

## 5. Where the code is

- **The menus' content** is data in the `dock-menu` crate (`menu_bar`): each item's title, key and the browser key it stands for, tested on every platform.
- **Building them in AppKit** is in `dock-menu`'s `src/mac.rs`, beside the Dock menu, the one module of that crate permitted unsafe code (requirement 10.12); winit's default menu is turned off.
- **The fields' selection** is the interface core's (`Browser::selected_text`, `Browser::paste_text`); the clipboard calls are the interface process's.

## 6. Requirements

Requirements 12.5 and 12.11, amended.

## 7. What building settled

Built on 23 October 2026.

1. **Menu items send keys, not new messages:** a menu item reports its tag; the interface turns it into the browser key the table names, and the interface core does what that key does. No new message reaches the trusted core.
2. **Copy and paste reach every field through the interface core:** the field with the keys is asked for its selection; a paste is typed into it, so each field's own rules hold.
3. **The menu takes its keys first:** `AppKit` gives a Command key to the menu bar before the window sees it, so a key with a menu item does its work once. Measured by hand on macOS with keys posted to the interface: Command-N opened one window; Command-M, which only the menu answers, minimised it; Paste typed the clipboard once.
4. **The clipboard is held open** once used: on Linux what was copied is served only while it is.
5. **About NeoTeletext** stays in the application menu, as winit's default menu had it. Since 25 October 2026 it shows the name NeoTeletext and the version the trusted core reports, the installed one or, for a browser built and not installed, the one it was built as, through `orderFrontStandardAboutPanelWithOptions:`; before, it stopped the interface: its sandbox kept `AppKit` from finding the program's bundle (`sandboxing.md` section 6 item 5). Every item with the browser's action is checked to have a key the interface core takes (`every_menu_item_s_key_is_taken`); the system's items (Hide, Hide Others, Show All, Quit, Minimise, Zoom) are `AppKit`'s own and are checked by hand.
