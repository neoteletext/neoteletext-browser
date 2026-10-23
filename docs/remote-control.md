# NeoTeletext: the remote control

| | |
|---|---|
| **Asked** | By the owner on 23 October 2026: "design a teletext remote control which is on a sidebar that can pop in and out of the right hand side or undocked to float", then, with the owner away, "do everything in the todo list ... fully implemented". |
| **Decision** | 68, taken while the owner was away (`architecture.md` section 2). |
| **Builds on** | `page-controls.md` (decision 67): every button of the remote is a control that already exists; the remote adds no new power, only a place to press it. |

## 1. What it is for

A teletext set is driven from a remote: digits, the four coloured keys and Index, Reveal, Hold, Mix, sub-pages, page up and down. The browser has each as a key (decision 67), but keys are hidden, and a mouse user has no other way to them. The remote shows them all, lit when they apply, as the remote beside a television does.

## 2. Where it sits

- **Docked**: a panel along the window's right edge, below the trusted strip and the bookmarks bar, from the strip down to the window's foot. The page area, the list and the overview narrow to make room. It slides in and out over 200 milliseconds, with the window's motion curve; a slim handle on the right edge, the menu and its keys bring it out and put it away.
- **Floating**: the same panel, the same size, over the page area, moved by its title row. It never covers the trusted strip or the bookmarks bar: it is kept inside the window below them (requirement 11.4). A floating panel stays where it was put; when the window shrinks it is moved back inside.
- **Hidden**: only the handle shows.

Docked or floating is chosen by a button on its title row; the state and a floating panel's place are kept in `settings.txt` by the trusted core (the interface stores nothing, requirement 7.9) and restored at the next start.

**Why inside the window, not a window of its own.** A window of its own can be covered or imitated by another program, and floating windows behave differently on every desktop (requirement 11.1's reasoning). Inside the browser's window the remote is drawn by the same interface process as the rest, with the same rules. When the browser has more than one window (the to-do's item on multiple windows), each window has its own remote.

## 3. What it holds

From the top:

1. **The title row**: "Remote", the dock/float button, and the button that puts it away.
2. **The page number**: the three digits being entered, or the open page's number, as a set shows it.
3. **The number pad**: 1 to 9, then 0, in a telephone's layout, as on a remote. A digit is the same as the keyboard's digit (requirement 12.4).
4. **Page up and page down**: the next and previous page, as Up and Down.
5. **Fastext**: the four coloured keys in a row, red, green, yellow, cyan, and Index below them. Each is lit only while the open page's key leads somewhere with row 24 shown (requirement 22.2); a dim key does nothing.
6. **Sub-pages**: the sub-page before, the count ("2/4", or the count and a turning mark while the latest shows), and the next; lit only on a page with more than one.
7. **Reveal, Hold and Mix**: each lit while on, dim while off, and dim altogether while no page is open.

Every button has its tip (requirement 12.12), naming it, what it does and its key.

## 4. Keys

Command-Shift-R on macOS, Control-Shift-R elsewhere, shows or hides the remote (Command-R alone is Reload, and the shifted letter keeps it next to it).

## 5. Security

The remote is part of the window the interface process draws; each button produces an input that already exists, a digit or a page control, which reaches the trusted core as strip input and is decided there exactly as its key is. It shows nothing the interface did not already have: the page number it was told, and which controls the open page has (requirement 22.6). Floating, it is held below the strip so that it can never hide what the strip shows. No new message carries anything but the remote's place.

## 6. Requirements

Section 23 of `requirements.md`, requirements 23.1 to 23.4.

## 7. What building settled

Built on 23 October 2026.

1. **No button in the strip.** The strip's right end already holds mute, settings and the menu; the handle at the window's edge, the menu's "Remote control" and Command-Shift-R are enough, and the strip keeps to what decides or shows trust (requirement 11.2).
2. **Undocked, it floats where it stood**, and docking it again forgets nothing: its floating place is kept for the next time.
3. **The handle** sits beside the docked remote, or at the window's edge while it is away, and shows which way it will move; it is not drawn while the remote floats.
4. **A place the trusted core sends** slides from the last frame drawn, since the interface keeps no clock of its own outside drawing.
