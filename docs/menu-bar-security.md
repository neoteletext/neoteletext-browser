# NeoTeletext: the menu bar and the fields' clipboard, security analysis

The design is `menu-bar.md` (decision 84). This analysis follows `security-analysis.md`'s form.

## 1. What is new

- On macOS, a menu bar built by the interface process through AppKit, in the one module of `dock-menu` permitted unsafe code (requirement 10.12).
- Copy, cut and paste in the browser's own text fields, through the system clipboard, by the interface process.

No new message reaches the trusted core: a menu item stands for a key the interface already handles, and its effects are the strip input that key sends.

## 2. Threats

| Who | Can | Cannot |
|---|---|---|
| A page | Nothing: the menu bar is the system's, drawn outside the window, and a page sends no keys to it | Choose a menu item, or read the clipboard |
| A channel that takes input | Receive what the user pastes while keys go to it, as before (decision 49) | Read the clipboard otherwise; the browser's own Command keys never reach it |
| Another application | Put text on the clipboard, which a paste types into a field | Do more than typing it would: a pasted address is still only text in the field, opened only by Return and the address rules, and hidden characters are dropped |
| A compromised interface | Copy what it shows to the clipboard, as any application can | Copy what it never had: it holds no cookies, history beyond the panels' views, or keys (requirement 3.9) |

## 3. The unsafe code

The menu bar's AppKit calls sit beside the Dock menu's in `dock-menu/src/mac.rs`: an Objective-C class whose one method hears which item was chosen, by its tag, and menus built with fixed titles and keys from the crate's own table. Its input is that table and the user's choice; it parses nothing.

## 4. Remaining risk

Low. The clipboard is a channel the user controls; what it carries in is bounded and cleaned as typed text is, and what it carries out is what the window already shows.
