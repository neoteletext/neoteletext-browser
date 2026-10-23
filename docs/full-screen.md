# NeoTeletext: full screen, the teletext page only

| | |
|---|---|
| **Asked** | By the owner on 23 October 2026: "design and spec a fullscreen mode which makes the current teletext screen ONLY fullscreen". |
| **Decision** | 85 (`architecture.md` section 2). Designed and specified; not built. |
| **Amends** | Requirement 11.4 (the trusted strip always visible). **Adds** requirement 12.16. |
| **Security analysis** | `full-screen-security.md`. |

## 1. What it is

**Full screen shows the open teletext page and nothing else**, filling the display, as a television shows its teletext:

- The page scaled to the largest size that fits the display, its proportions kept, centred.
- Around it, the page's screen colour (EN 300 706 Table 4's Default Screen Colour, or clause 12.3's Full Screen Colour), as the page area already draws it.
- No trusted strip, no tab strip, no toolbar, no banks, no page list, no wall of pages, no overview, no remote control, no tips, and the pointer hidden after two seconds without moving.

The page is drawn exactly as in the window: the same engine rendering (requirement 2.1), flash, reveal, hold, mix, sub-pages, the browser's link underline (decision 42), and the channel's sound (decision 37).

## 2. Entering

Only a window with an open page can enter full screen. With the wall showing, the selected page opens first, as a press of Return does.

| How | macOS | Windows and Linux |
|---|---|---|
| Key | Control-Command-F, the system's own | F11, the system's own |
| Menu | View > Enter Full Screen (decision 84's menu bar) | The menu button's menu: Full screen |
| Pointer | A double click on the open page | The same |
| The window's green button | Enters the system's full screen of the whole window, as it does now; it is not this mode | — |

On macOS the window enters a full-screen space of its own, as every Mac application's full screen does; elsewhere it becomes borderless and covers its display. Each window has its own full screen; the others stay as they are.

**On entering, the browser says how to leave:** "Full screen: Escape to leave", drawn by the interface for three seconds in the screen colour's margin beside the page, never over the page, in the strip's own style.

## 3. Inside

What a viewer does with a television's teletext works, and nothing else is needed:

- **Page numbers, page up and down, Fastext and Index, Reveal, Hold, Mix and the sub-page keys** (decision 67): the page changes within the channel and stays full screen.
- **A click on a link to a page of the same channel** opens it, full screen.
- **Sound** plays as in the window.
- **Typing into a channel that takes input** (decision 48) works: while keys go to the channel, a small trusted badge, "Keys go to *host*", shows in the margin, as the strip's typing badge does in the window, since the strip that would say it is hidden.

## 4. The strip on demand

The trusted strip is one movement away:

- **The pointer at the top edge** for half a second brings the strip, the same strip the window draws, down over the top of the page; it goes when the pointer leaves it. Everything in it works as in the window.
- **Any key the browser keeps for itself** (its own modifier's keys: Command on macOS, Control elsewhere) shows the strip for three seconds as it acts.

## 5. Leaving

Full screen ends, and the window is as it was, when:

- **Escape** is pressed while keys do not go to a channel;
- **the full-screen key** (Control-Command-F, F11) is pressed again, which a channel never receives, so it leaves even while typing;
- **the system ends it** (macOS's full-screen exit, Mission Control, a display unplugged);
- **anything needs the window:** a link or Fastext key that leaves the channel (its site-change screen is in the strip, requirement 8.4), a certificate refused (decision 83), the tab closed or another chosen, a new tab or window, a panel of the strip opened, the developer console, or the trusted core's question in the strip.

Leaving does not change the open page.

## 6. What it does not do

- It is not kept with the session: a restored window opens as a window.
- The wall, lists and overview do not have a full screen of their own.
- Two pages side by side, subtitles over a picture, and a picture behind the page are not part of it.

## 7. Where it lives

- **The interface core** gains a layout, `FullScreen`, that draws only the open page, its margin, the leaving notice, the typing badge and, on demand, the strip; and the rules of sections 3 to 5.
- **The interface process** asks winit for the window's full screen (`Window::set_fullscreen`, borderless on the window's display) and leaves it.
- **The trusted core is not involved:** no message changes, and what it decides (links, Fastext, typing, sound) is decided as in the window.

## 8. Requirements

Requirement 11.4 amended and requirement 12.16 added.

## 9. Before building

1. **The keys** follow each system's own full-screen key so that a viewer's habit works; the double click is the pointer's way in. The owner may prefer a single teletext key, such as `f`, as well.
2. **macOS's full-screen space** shows the menu bar when the pointer reaches the top edge; the strip's half-second wait at the edge keeps the two from fighting, and is to be measured when built.
