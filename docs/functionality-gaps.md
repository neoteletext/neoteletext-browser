# NeoTeletext: what the browser does and what it lacks

| | |
|---|---|
| **Asked** | By the owner: "examine the broad functionality of neoteletext. what is missing, what have i forgotten", re-analysed on 23 October 2026 against what was built since, and turned into this document. |
| **State** | As of 23 October 2026, commit `04d1612`. Each row names its decision or requirement where one exists; the normative text is `requirements.md`. |

**Built** means in the code with its tests. **Partly** says what is missing. **Not built** has no code. **Withdrawn** was decided against.

## 1. Teletext itself

| Feature | State | Where |
|---|---|---|
| Fastext, the Index key and the receiver's row 24 | Built | Decision 67, `page-controls.md` |
| Reveal, hold, mix and sub-pages | Built | Decision 67 |
| The remote control | Built | Decision 68, `remote-control.md` |
| The channel's date, time and name in the header (SVC-002, SVC-005) | Not built | — |
| The channel's initial page; every channel opens at 100 | Not built | — |
| A subtitle or newsflash mode, the page boxed over nothing | Not built | — |
| Full screen of the page alone | Designed, not built | Decision 85, `full-screen.md`, requirement 12.16 |

## 2. Finding and using channels

| Feature | State | Where |
|---|---|---|
| Discovery of channels | Not built | — |
| Search across a channel's pages | Not built | — |
| Forms, a channel asking for input other than typing | Not built | Requirement 7.10 |
| Saving, printing, exporting or copying a page's text | Not built | — |
| Alerts when a watched page changes | Not built | — |
| Earlier versions of a page | Not built | — |
| Fonts named by channels and pages | Withdrawn | Decision 66 |

## 3. Browser basics

| Feature | State | Where |
|---|---|---|
| The site-change screen | Partly: built for links and Fastext; redirects and discovery results do not show it | Requirement 8.4 |
| The certificate-error screen | Built | Decision 83, `certificate-errors.md`, requirement 7.13 |
| Other refusals (an address refused, a site that will not open) explained in the window | Partly: in the strip's notice and the console's Log only | Decision 70 |
| Dragging tabs, their overflow, and multiple windows | Built | Decisions 69 and 72 |
| The menu bar on macOS | Built | Decision 84, `menu-bar.md`, requirement 12.5 |
| Cut, copy, paste and select all in every text field | Built | Decision 84, requirement 12.11 |
| The input method's text shown while it is composed | Not built | Requirement 12.11 |
| Screen readers and keyboard reach of every control | Not built | Requirement 12.9 |
| Reduced motion | Not built | — |
| A 4:3 page aspect | Not built | — |
| Files dropped on the window | Not built | — |
| Help, and the interface in other languages | Not built | — |
| Removing a bookmark from a history row's menu | Built | Decision 64 |
| Ranking the address field's suggestions | Unchanged since decision 57 | `history-and-bookmarks.md` |

## 4. Shipping

| Feature | State | Where |
|---|---|---|
| The installer and the updater | Built, proved end to end with test keys on every desktop | Decisions 54 and 73 to 82 |
| Signing, the release keys and the update host | The owner's: the key ceremony, the certificates and the host | `PENDINGTASKSFORANDREW.md` |
| Checks by hand of decisions 63 to 65 on Windows and Linux | Not done | — |
| Phones and microcontrollers | Not built | — |
| Diagnostics in the console's Log | Built | Decision 70 |

## 5. Security and quality

| Feature | State | Where |
|---|---|---|
| The hostile interface harness | Partly: against the trusted core alone; not with running helpers | Requirement 3.9 |
| A review of spoofing in every layout | Not done | Requirement 14.5 |
| A check of the engine's guarantees | Not done | Requirement 2.3 |
| The grace period for closed tabs, the live-feed threshold, refresh schedules | Not built | — |
| One decoder shared by tabs on the same channel | Not built | Decision 11 |
| A latency test | Partly | — |

## 6. Documents

Every document was brought up to date with the code on 23 October 2026 (`aae1144`). Each feature built since carries its own design and security analysis: `certificate-errors.md`, `menu-bar.md` and `full-screen.md`.
