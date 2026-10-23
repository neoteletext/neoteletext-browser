# NeoTeletext: history and bookmarks

| | |
|---|---|
| **Status** | Proposed 22 October 2026 on the owner's request for an architecture and requirements for browser-style history and browser-style bookmarks; adopted and built the same day by decision 57, while the owner was away, on the owner's instruction to "FULLY implement history and bookmarks - make any decisions you need to". Section 13's items were decided as recommended but item 10; section 14 lists what building changed, for the owner to confirm. |
| **Security** | [`history-and-bookmarks-security.md`](history-and-bookmarks-security.md), whose findings this design takes. |
| **Requirements** | Section 20 of [`requirements.md`](requirements.md), with requirements 3.6, 12.5, 12.6, 19.1, 19.2 and 19.7 amended and 19.5 withdrawn; section 12 below is the text as proposed. |
| **Builds on** | Each tab's back-and-forward list (requirement 8.1), the star and the bookmarks in `settings.txt` (requirements 19.1 and 19.5), the start page (requirement 19.7), the settings panel (`settings-home-console-proxy.md` section 1.3), the broker and the trusted core's confinement (decision 56). |

## 1. What is wanted

The history and bookmarks a user of Chrome, Firefox, Safari or Edge expects, made for teletext.

**History** is the record of the pages the user has seen, across every tab and between runs: a page that lists them by day and searches them, ways to remove any of them, the address field suggesting what was seen before, recently closed tabs brought back, the Back and Forward buttons' lists, and tabs that record nothing.

**Bookmarks** are the pages and channels the user chose to keep: named, in folders, edited and moved in a manager, reached from the star, a bar, a menu, the address field and the start page.

What this browser has today: each tab keeps its back-and-forward list in memory only (requirement 8.1), and nothing of it survives the tab; the star keeps up to 1 000 channel addresses, unnamed and in one flat list, in `settings.txt` (requirement 19.5); the start page lists the first 99 (requirement 19.7). Requirement 12.6 already says history and bookmarks are shown from data the trusted core supplies; "History has no page."

### 1.1 What makes teletext different

- **A page is a place.** An address names a channel and, after `#`, a page: `https://news.example/teletext/#302` is the weather. A user moves through dozens of pages a minute by typing three digits, so history keeps a page once a day, not once a visit (section 2.1), as Chrome's history page shows a page once a day however often it was visited.
- **Pages change while they are shown.** A pushed update, a new subpage or a clock is not a visit; only a navigation is.
- **Titles come from the channel.** A channel's title and a page's title are written by the channel's author, so they are hostile text (section 9 and the security analysis).
- **A channel holds at most 800 pages**, which bounds how much history one channel can make in a day.

### 1.2 Not in this design

Synchronising between devices; reading another browser's files; a history of downloads, which do not exist. Restoring the window and its tabs is section 15, added on the owner's instruction of 22 October 2026.

## 2. History

### 2.1 What is recorded

A **row** of history is one address on one day:

| Field | What | Bound |
|---|---|---|
| Day | The local calendar day of the visit, as `2026-10-22` | |
| Address | The channel and page, as the address rules accepted it (requirement 11.3), without a subpage | 2 048 bytes, the longest address |
| Channel title | The channel's title when last visited, cleaned (requirement 11.5) | 256 bytes (`protocol::MAX_TITLE`) |
| Page title | The page's title when last visited, cleaned | 160 bytes (`protocol::MAX_PAGE_TITLE`) |
| Last visit | Seconds since 1970 (UTC) | |
| Visits | How many times that day | |
| Typed | How many of them the user typed in the address field | |

**A visit** is a navigation of a tab to an address, of any kind (typed, a bookmark, a history row, a link, Back and Forward, Home, three digits typed on the page), recorded when the tab first shows that page: its first frame is drawn. A page number with no page behind it, which never draws, is not recorded. A pushed update, a new subpage or a redraw of the same page is not a visit.

**A reload** (decision 63) is not a visit, as neither Chrome nor Firefox counts one: when its page draws, the address's row for the day takes the reload's time as its last visit, and is made if the day has none, but its visits and typed visits stay as they were, so suggestions are not raised by reloading.

**Not recorded:** the browser's own `neoteletext:` pages; anything in a private tab (section 2.7). *Amended by decision 65:* "Remember history" is gone (section 2.6).

Recording happens in the trusted core, which already knows each navigation and each frame drawn. Nothing about history is sent to a decoder, a fetcher or the phone keyboard: a channel never learns what else the user has seen (requirement 7.3).

### 2.2 How long, and how much

- **90 days.** Rows whose day is older are removed at start and at midnight. Chrome keeps 90 days of history.
- **At most 50 000 rows**, the oldest removed first: room for 90 days of more than 550 distinct pages a day, which is more than half of a full channel every day of the three months.
- **The file at most 16 MiB**, the oldest rows removed first: a typical row is about 120 bytes, so 50 000 rows are about 6 MB and the row bound applies first; the byte bound applies only to unusually long addresses, and bounds what the trusted core reads at start.

### 2.3 The file

`history.txt`, beside `settings.txt` in the settings folder (requirement 19.1), the one folder the confined trusted core may write (decision 56), created readable and writable by the user alone on macOS and Linux (mode 0600). Plain text, so a user can read what is kept about them:

```
neoteletext-history 1
2026-10-22	1792656300	3	1	https://news.example/teletext/#302	News Example	WEATHER
```

The first line is the format and its version. Each further line is one row: day, last visit, visits, typed, address, channel title, page title, separated by tab characters. A tab is never in a field: the address rules refuse hidden characters (requirement 11.3), and cleaning removes them from titles (requirement 11.5).

**Written by appending:** each new visit appends the row's new line, so a visit costs one short write. When rows repeat, the last line for a day and address stands. The file is **compacted** (written whole to a temporary file renamed over it, as `settings.txt` is) at start, at quit, when it holds twice as many lines as rows, and **at once after any removal** (section 2.5), so removed history leaves the file before the panel says it is gone. A crash loses at most the line being written; the reader drops a broken last line.

**Read strictly**, as `settings.txt` is (requirement 19.2): the version line; at most 16 MiB; each line exactly seven fields, each within its bound and rule (a real day, numbers within range, an address the address rules accept, titles that are already clean); a line that breaks a rule is dropped and logged and the rest stand. A fuzz target, `history_file`, checks the bounds and that what was read writes a file that reads back the same.

### 2.4 The history panel

A panel of the trusted strip, as the settings panel is (`settings-home-console-proxy.md` section 1.3): drawn by the interface from the trusted core's view, in the strip's colours, below the strip; no page can draw it or send it input. Opened by Command-Y on macOS and Control-H elsewhere (Chrome's keys), by the menu button and the History menu (section 6), and closed by Escape or the keys again.

- **A search field** at the top. The trusted core searches each row's address, channel title and page title for the text, ignoring case.
- **The rows, newest first, under a heading for each day:** "Today: Wednesday 22 October 2026", "Yesterday: …", then each date. Each row shows the time (`14:05`), the channel's title, the page number and the page's title, and below them the address as the address rules print it (section 9).
- **Choosing a row** opens its address in the active tab as a navigation the user starts (requirement 8.2); Command-click or a middle click, and Control-click away from macOS, opens it in a new tab.
- **Each row has a menu** (decision 64, as Chrome's): "More from this site", which searches `host:` and the row's host, "Remove from history", and "Remove bookmark" when the address is bookmarked; rows can be selected with their check boxes for "Remove selected".
- *"Forget this channel" was withdrawn by decision 65:* Chrome has no such command; "More from this site" finds a site's rows, which can then be selected and removed.
- **"Clear history…"** opens the clearing choices (section 2.5).
- **Recently closed** tabs head the panel when there are any (section 2.8).

The trusted core sends the panel **at most 100 rows at a time**, the rows on screen and a margin (more than a screen holds at the tallest window), and the next rows when the user scrolls to them (requirement 3.6: nothing beyond what is on screen). Each row in a view carries a number the trusted core gave it for that view; the interface asks to open or remove a row by that number, never by an address, so it cannot open or remove what it was not shown.

### 2.5 Removing history

- **A row, selected rows, a channel's rows,** from the panel.
- **A time range,** from "Clear history…": the last hour, the last 24 hours, the last 7 days, the last 4 weeks, or everything (Chrome's ranges). A row is in the range when its last visit is.
- **Everything** also empties recently closed tabs and the typed-address record that inline completion uses.

The file is compacted at once (section 2.3). *Amended by decision 64:* as Chrome does, removal also reaches the tabs' back-and-forward lists, never the entry a tab is at, and the recently closed tabs (section 16); this section had said no browser does, which Chrome's source shows wrong. The ranges begin with the last 15 minutes, as Chrome's do.

### 2.6 "Remember history"

*Withdrawn on 23 October 2026 by decision 65:* history follows Chrome, which has no such setting; a private tab is the way to browse without a record. It was a setting of the settings panel, on by default, kept in `settings.txt` as `history on`; off, nothing was recorded.

### 2.7 Private tabs

A **private tab** records nothing: no history rows, no typed address, no place in recently closed tabs. It still reads history and bookmarks for its address field's suggestions, as Chrome's incognito windows do, and the user may still bookmark a page from it, which is the user's own act. Opened by Command-Shift-N on macOS and Control-Shift-N elsewhere, and from the menu button. The strip marks it with "Private" in its own colour beside the address, and its tab shows a mask, so the user can always tell.

Cookies are already kept only for a fetcher's life (the engine's REQ-SEC-013), so a private tab differs from another tab only in what history keeps. Private tabs share nothing with other tabs, as no two tabs do (requirement 7.3).

### 2.8 Recently closed tabs

The trusted core keeps the **last 25 closed tabs**, each with its whole back-and-forward list and position (Chrome keeps 25); *amended by decision 64,* they are kept with the session between runs, as Chrome's are (section 16). Command-Shift-T on macOS and Control-Shift-T elsewhere reopens the most recent one where it was; the history panel and the History menu list them by title. They are not kept between runs: restoring a session is its own design (section 1.2).

### 2.9 Back and Forward lists

Pressing and holding Back or Forward, or a secondary click on it, opens a list of the tab's entries that way, the nearest first, at most 12, then, when it is full, up to 5 chapter stops where the host changes (decision 64, as Chrome's; section 16), each with its page number and title; the last line is "Show all history". Choosing an entry moves the tab there, as Back pressed that many times would. The list is the trusted core's view of that tab's own entries.

### 2.10 The address field's suggestions

While the user types in the address field, the interface sends the text to the trusted core, which answers with at most 8 suggestions, drawn below the field in the strip (a bound on the view, to be fitted to the smallest window when built):

1. **Bookmarks** whose name or address contains the text, in the order they are in the bookmarks.
2. **History rows** whose address, channel title or page title contains the text, the most visited first and then the most recent, one per address.

Each suggestion shows the address as the address rules print it, and the title in a lighter style beside it. Up and Down move through them; Enter or a click opens one as a navigation the user starts.

**Inline completion,** the rest of an address filled in and selected as the user types, comes only from addresses **the user typed** before (the history's "typed" count), never from links followed or titles, so a channel cannot make the field complete to an address of its choosing. Chrome's history completion likewise favours addresses the user typed.

## 3. Bookmarks

### 3.1 What a bookmark is

A **bookmark** has a name and an address; a **folder** has a name and holds bookmarks and folders, in an order the user sets. Two folders are always there and cannot be removed, as in Chrome: **Bookmarks bar** and **Other bookmarks**.

| Bound | Value | Why |
|---|---|---|
| Bookmarks and folders together | 10 000 | Ten times today's 1 000; enough for any collection made by hand, and it bounds the file |
| Folder depth | 16 | Deeper than any hand-made tree; it bounds every walk of the tree |
| A name | 256 bytes (`protocol::MAX_TITLE`) | A channel's title fits |
| An address | 2 048 bytes | The longest address |

The same address may be bookmarked more than once, in different folders, as in every browser. A name is the user's text, cleaned as every shown text is (requirement 11.5); the default name is the channel's title and, for a page, the page number and its title: `News Example 302 WEATHER`.

### 3.2 The star

The star in the strip is filled when the address the strip shows is bookmarked anywhere.

- **Clicking an empty star,** or Command-D on macOS and Control-D elsewhere, bookmarks **the address the strip shows**, with its page when it shows one, at the end of the folder last used (Other bookmarks at first), and opens the star's popover.
- **The popover**, a panel of the strip, shows the name and the folder to change, "The whole channel", which drops the page from the address, "Remove" and "Done". Escape or a click elsewhere closes it, keeping what it shows.
- **Clicking a filled star** opens the popover for that bookmark.

Today the star keeps the channel without its page (requirement 19.5). Browsers keep the exact address; keeping the page shown lets a user keep "the weather on page 302", and "The whole channel" keeps today's choice one click away.

### 3.3 The bookmarks manager

A panel of the strip, opened by Command-Option-B on macOS and Control-Shift-O elsewhere (Chrome's keys), by the menu button and the Bookmarks menu.

- **Left:** the tree of folders. **Right:** the open folder's contents, in their order, each with its name and address.
- **A search field** over every bookmark's name and address; its results show their folder.
- **Open, open in a new tab, edit** (name and address; an address must pass the address rules), **remove**, **new folder**, **move** (by dragging, or "Move to…" with a folder chooser), and **sort by name** for a folder.
- **Removing** shows "Removed. Undo" for ten seconds, as long as the strip's notices last; Undo puts it back where it was. A folder is removed with what it holds.

As with history, the trusted core sends the manager only the folders' names and the open folder's contents, at most 100 items at a time, each with a number for that view; the interface asks for every change by those numbers.

### 3.4 The bookmarks bar

An optional row of the strip, below the address, showing the Bookmarks bar folder's contents left to right; a folder there opens a list of what it holds. Off by default, since the strip's height is the page area's loss; Command-Shift-B on macOS and Control-Shift-B elsewhere shows or hides it, and the setting is kept (`bar on`). It is part of the trusted strip: drawn from the trusted core's view, above the page area, out of every page's reach.

### 3.5 The start page

Page 100 of `neoteletext:home` lists the bookmarks, and pages 201 to 299 each open one (requirement 19.7). With folders, it lists the Bookmarks bar folder's bookmarks first and then Other bookmarks', walking each folder in order, a folder's name as a heading line; the first 99 bookmarks so walked have pages. A bookmark's line shows its name, cut to the row, instead of its address.

### 3.6 The file

`bookmarks.txt`, in the settings folder, readable by the user alone as `history.txt` is, written whole to a temporary file renamed over it at each change, as `settings.txt` is; changes are few.

```
neoteletext-bookmarks 1
folder	1	0	Bookmarks bar
folder	2	0	Other bookmarks
bookmark	3	1	News Example	https://news.example/teletext/
folder	4	2	Weather
bookmark	5	4	News Example 302 WEATHER	https://news.example/teletext/#302
```

Each line is a kind, a number, its folder's number (0 for the two that are always there), a name and, for a bookmark, an address, separated by tab characters. Lines are in each folder's order. **Read strictly:** the version line; at most 24 MiB (10 000 items of the longest line, rounded up); numbers unique; a folder's line before anything inside it, so there can be no cycle; depth at most 16; each name and address within its bounds and rules. A line that breaks a rule is dropped and logged, with anything inside it if it is a folder. A fuzz target, `bookmarks_file`, as for history.

The `bookmark` lines of `settings.txt` are no longer written there (requirements 19.1 and 19.2 amended). Whether today's are moved across once is the owner's decision (section 13, item 9).

### 3.7 Export and import

A browser lets its bookmarks be saved to a file and read from one, and every browser reads and writes the Netscape bookmark file (an HTML file). Two things stand in the way here:
- **Reading that file is reading HTML** from outside, which the trusted core must not do (it reads no outside format, `security-analysis.md` section 1); it would need a strict reader of the format's few tags, in a confined helper, fuzzed.
- **Choosing where to save or what to read** needs the platform's file chooser, which the confined interface may not open and the broker does not draw.

The proposal, for the owner (section 13, item 8): build the rest first; then export and import in `bookmarks.txt`'s own format, read by the broker as a file channel is (`sandboxing.md` section 7) and by the same strict reader, with the address typed in a field of the manager; the Netscape format only if the owner wants bookmarks from other browsers, which are web addresses and mostly not channels.

**Built on 23 October 2026 (decision 71), the owner away.** The owner's to-do list named reading another browser's file. The broker reads it by its path, as before; the trusted core looks at its first line only (`<!DOCTYPE NETSCAPE-Bookmark-file-1>`) and, finding it, starts a decoder with `--bookmarks` and gives it the file. The decoder, which opens no file and no network, reads the format's few tags (`DL` for a folder's contents, `H3` for its name, `A` with `HREF` for a bookmark; every other tag passed over, every attribute but `HREF` ignored), decodes character references, and reports each folder, its end and each bookmark, at most 10,000 and 16 folders deep, names and addresses bounded. The trusted core holds each address to its address rules (most web addresses are channels by discovery; `place:`, `javascript:` and `data:` are refused), cleans each name, and imports the rest into Imported, saying how many came in and how many were left out. The file chooser is still the owner's question: the path is typed.

## 4. Where the data lives and who sees it

| Data | Kept by | File | Sent to the interface | Sent to a channel |
|---|---|---|---|---|
| History rows | Trusted core | `history.txt` | The panel's rows on screen; suggestions; the Back and Forward lists | Never |
| Typed addresses | Trusted core | `history.txt` (the typed count) | Only an inline completion of what is being typed | Never |
| Recently closed tabs | Trusted core, memory | None | Their titles, in the panel and the menu | Never |
| Bookmarks | Trusted core | `bookmarks.txt` | The manager's folders and open folder; the bar; the popover; suggestions | Only to the start page's own decoder, as today |

Requirement 3.6 is amended to say this: the interface receives history and bookmarks only as these views of what is on screen.

## 5. Messages

All from the interface are strip input (requirement 3.9): each is checked by the trusted core, and none carries an address the trusted core acts on without its rules.

| Message | From | To | Carries |
|---|---|---|---|
| `History(open)` | Interface | Trusted core | Open or close the history panel |
| `HistoryQuery` | Interface | Trusted core | The search text, at most 256 bytes, and the first row wanted |
| `HistoryView` | Trusted core | Interface | At most 100 rows with their day headings and view numbers, whether there are more, recently closed titles |
| `HistoryOpen` | Interface | Trusted core | A row's view number, and whether in a new tab |
| `HistoryRemove` | Interface | Trusted core | Row view numbers (at most 100), or "this channel" with a row's number, or a range of section 2.5 |
| `Suggest` | Interface | Trusted core | The address field's text |
| `Suggestions` | Trusted core | Interface | At most 8 suggestions, and an inline completion |
| `TabList(forward)` / `TabListView` / `GoTo` | Both | Both | The Back or Forward list of the active tab, at most 15, and the entry chosen |
| `Reopen` | Interface | Trusted core | The most recent closed tab, or one by its view number |
| `PrivateTab` | Interface | Trusted core | Open a private tab |
| `Bookmarks(open)` / `BookmarksView` | Both | Both | The manager: the folder tree's names, and at most 100 items of the open folder or search, with view numbers |
| `BookmarkEdit` | Interface | Trusted core | A view number and a new name, address or place (folder and position); a new folder; remove; undo |
| `BookmarkOpen` | Interface | Trusted core | A view number, and whether in a new tab |
| `BarView` | Trusted core | Interface | The Bookmarks bar folder's items, at most 100, and an open folder's list |
| `StarView` | Trusted core | Interface | Whether the address is bookmarked; the popover's name and folder |

Each is bounded in `protocol`, read by a reader that refuses anything else, and fuzzed with the existing `interface_messages` and `core_messages` targets.

## 6. Menus and keys

When the menu bar of requirement 12.5 is built, the **History** menu holds Back, Forward, Home, Show All History, Reopen Closed Tab, Recently Closed (a list), Clear History…, and the 10 most recent rows; the **Bookmarks** menu holds Bookmark This Page, Bookmark Manager, Show Bookmarks Bar, and the two folders as lists of at most 100 items each. Until then the menu button's menu holds them, with New tab, New window, New private tab, New private window and the rest (section 14 item 9, `multiple-windows.md` section 5).

| Action | macOS | Elsewhere |
|---|---|---|
| History panel | Command-Y | Control-H |
| Bookmark this page | Command-D | Control-D |
| Bookmarks manager | Command-Option-B | Control-Shift-O |
| Bookmarks bar | Command-Shift-B | Control-Shift-B |
| Reopen closed tab | Command-Shift-T | Control-Shift-T |
| Private tab | Command-Shift-N | Control-Shift-N |
| Clear history | Command-Shift-Delete | Control-Shift-Delete |

The interface core is the same on every platform, so it takes each platform's keys everywhere, as for the settings (`settings-home-console-proxy.md` section 7, item 8).

## 7. Time and days

History groups by the user's local day, so the trusted core needs the local time zone and its daylight-saving rules: a day kept in UTC would split an Australian day at 10 or 11 in the morning. The standard library has no time zones, and the trusted core may hold no unsafe code (requirement 10.12), so a time-zone crate is needed; adding one is the owner's decision (section 13, item 10). The row keeps the day as it was where the user was, so travelling does not move past rows.

The confined trusted core must be able to learn the zone, which changes its confinement in one place each:
- **macOS:** Apple's baseline (`system.sb`), which the core's profile imports, already allows the time-zone files.
- **Linux:** the core's Landlock rules let it read nothing outside its settings folder (`confine/src/linux.rs`, `readable`), so `/etc/localtime` and `/usr/share/zoneinfo` must be added, read only, and `TZ` to the core's environment (`confine::environment(Role::Core)`).
- **Windows:** the zone's name comes from a system call the container allows; the zone's rules come from a copy of the time-zone database built into the program, as Windows has none in the form the crate reads.

Each is to be confirmed by running the confined core, as `the_trusted_core_starts_confined_and_reaches_its_own_code` does.

## 8. Performance

- **A visit** is one row looked up in memory and one line appended: well under a millisecond, off the drawing path.
- **Start** reads at most 16 MiB of history and 24 MiB of bookmarks; at typical sizes (6 MB and 100 KB) the measurement to make is the time to the first frame, which must not grow by more than 50 ms (requirement 20.14).
- **Search and suggestions** scan at most 50 000 rows and 10 000 bookmarks in memory per keystroke: a few milliseconds; if measurement says otherwise, an index of words is the next step.

## 9. Showing hostile text safely

Channel and page titles are the channel author's text, and bookmark names the user's. Everywhere they are shown (panel, suggestions, lists, menus, bar, popover, start page):
- **Cleaned and cut** (requirement 11.5): no hidden or direction-changing character, within its bound.
- **Never in the address's place, and never alone:** the address is always shown beside a title, printed from the parsed address by the address rules (requirement 11.3), so a title of `https://bank.example` cannot pass for where a row leads.
- **Never complete the address field** (section 2.10).

## 10. What this changes in the trusted core's confinement

Both files are in the settings folder, which the trusted core already reads and writes (decision 56). The one change is the time zone on Linux (section 7). The broker gains no request unless export and import are built (section 3.7).

## 11. Tests and checks this design asks for

The requirements of section 12 name them. In short: recording rules (what is and is not a visit, private tabs, the setting); the file's strict reading, its bounds and its fuzz target; appending and compaction, and that removal leaves the file at once; the 90-day and row bounds; the panel's views send no more than 100 rows and accept only view numbers; the ranges of clearing; suggestions and that inline completion comes only from typed addresses; the Back and Forward lists; recently closed tabs; the bookmark tree's bounds, cycle-free reading and its fuzz target; the star and popover; the manager's edits and undo; the bar; the start page with folders; hostile titles in every place they are shown; start time measured.

## 12. Proposed requirements

On adoption these are added to `requirements.md` as section 20, and the amendments made. Each has its State "Not met" until built.

### 20. History and bookmarks

**Requirement 20.1: Visits are recorded.**
- *Statement:* The trusted core must record each page a tab shows, as one row per address and local day, with the channel's and page's titles cleaned, the last visit's time, the day's visits and how many the user typed; a visit is a navigation recorded when the tab draws that page's first frame, and a pushed update, a subpage or a redraw is not one. The browser's own `neoteletext:` pages, private tabs and everything while "Remember history" is off must not be recorded.
- *Reason:* History is the record of what the user saw, and only that.
- *Verification:* Test of each kind of navigation, of a page that never draws, of updates, of the start page, of a private tab and of the setting.

**Requirement 20.2: History is kept for 90 days.**
- *Statement:* History must be kept between runs in `history.txt` in the settings folder, readable and writable by the user alone on macOS and Linux, appended at each visit and written whole to a temporary file renamed over it at start, at quit, when it holds twice as many lines as rows, and at once after any removal; rows older than 90 days, beyond 50 000, or beyond a file of 16 MiB must be removed, the oldest first.
- *Reason:* A browser's history outlives a run, within bounds that keep start fast and the record from growing forever.
- *Verification:* Tests of round trip, of each bound, of a broken last line, and of compaction.

**Requirement 20.3: The history file is read strictly.**
- *Statement:* `history.txt` must be read as section 2.3 of `history-and-bookmarks.md` defines it: a version line, at most 16 MiB, each line seven tab-separated fields each within its bound and rule; a line that breaks a rule is dropped and logged, and the rest stand.
- *Reason:* The file is outside input: another program can write it.
- *Verification:* Test of each rule; the fuzz target `history_file`.

**Requirement 20.4: The history panel.**
- *Statement:* A panel of the trusted strip, opened by Command-Y on macOS and Control-H elsewhere and from the menus, must show history newest first under a heading for each day, each row with its time, channel title, page number, page title and address, with a search of addresses and titles; choosing a row must open it as a navigation the user starts, in the active tab or a new one.
- *Reason:* The browser-style view of what was seen.
- *Verification:* Interface and trusted-core tests.

**Requirement 20.5: The interface sees only history on screen.**
- *Statement:* The trusted core must send the history panel at most 100 rows at a time, each with a number for that view, and must act on the interface's requests to open or remove rows only by those numbers.
- *Reason:* Requirement 3.6: a compromised interface finds nothing beyond what is on screen, and cannot name an address of its own through history.
- *Verification:* Test that no view holds more than 100 rows and that an unknown or stale number is refused.

**Requirement 20.6: History can be removed.**
- *Statement:* The user must be able to remove a row, selected rows, every row of a channel, and every row whose last visit is within the last hour, 24 hours, 7 days, 4 weeks or ever; removing everything must also empty recently closed tabs and the typed record; removed rows must be gone from `history.txt` before the panel shows them removed.
- *Reason:* The user owns their history.
- *Verification:* Tests of each, reading the file after each.

**Requirement 20.7: "Remember history".**
- *Statement:* A setting, on by default and kept in `settings.txt`, must stop all recording when off, and turning it off must offer to clear what is kept.
- *Reason:* Some users want no record.
- *Verification:* Test.

**Requirement 20.8: Private tabs.**
- *Statement:* A private tab, opened by Command-Shift-N on macOS and Control-Shift-N elsewhere, must record no history, no typed address and no recently closed entry, and must be marked "Private" in the strip and in its tab.
- *Reason:* Browser-style browsing that leaves no record.
- *Verification:* Test of each, and that the mark is shown.

**Requirement 20.9: Recently closed tabs.**
- *Statement:* The trusted core must keep the last 25 closed tabs, other than private ones, in memory with their whole back-and-forward lists; Command-Shift-T on macOS and Control-Shift-T elsewhere must reopen the most recent where it was.
- *Reason:* Closing a tab by mistake must not lose it.
- *Verification:* Test.

**Requirement 20.10: Back and Forward lists.**
- *Statement:* Holding Back or Forward, or a secondary click on it, must list at most 15 of the tab's entries that way, nearest first; choosing one must move the tab there.
- *Reason:* Browser-style navigation through a tab's list.
- *Verification:* Interface and trusted-core tests.

**Requirement 20.11: Suggestions in the address field.**
- *Statement:* While the user types an address, the trusted core must offer at most 8 suggestions from bookmarks, then from history, each shown with its address as the address rules print it; an inline completion must come only from addresses the user typed before.
- *Reason:* What was seen before is quicker to reach; a channel must not choose what the field completes to.
- *Verification:* Tests of the order, the bound, and that a link followed or a title never completes the field.

**Requirement 20.12: History never reaches a channel.**
- *Statement:* No history row, title, count or typed address may be sent to any decoder, fetcher or phone keyboard.
- *Reason:* A channel must not learn what else the user has seen (requirement 7.3).
- *Verification:* Review of every message to those processes.

**Requirement 20.13: Hostile titles stay text.**
- *Statement:* Every channel title, page title and bookmark name must be cleaned and cut wherever it is shown, and shown beside the address it leads to, never in the address's place.
- *Reason:* Titles are the channel author's text.
- *Verification:* Tests with hostile titles in every place they are shown.

**Requirement 20.14: History and bookmarks do not slow the start.**
- *Statement:* With 50 000 history rows and 10 000 bookmarks, the time from start to the first frame must not grow by more than 50 milliseconds over a start with none.
- *Reason:* The browser must open at once.
- *Verification:* Measurement, release build, recorded with its date.

**Requirement 20.15: Bookmarks and folders.**
- *Statement:* Bookmarks must have a name and an address, and folders a name and an order of bookmarks and folders; the folders Bookmarks bar and Other bookmarks must always exist; at most 10 000 bookmarks and folders together, 16 folders deep, a name of 256 bytes and an address of 2 048.
- *Reason:* Browser-style bookmarks, bounded.
- *Verification:* Tests of each bound.

**Requirement 20.16: The star.**
- *Statement:* The star must be filled when the strip's address is bookmarked; clicking an empty star, Command-D on macOS or Control-D elsewhere must bookmark the address the strip shows, page included, and open a popover to change its name and folder, keep the whole channel instead, or remove it.
- *Reason:* Browser-style bookmarking of what is shown.
- *Verification:* Interface and trusted-core tests.

**Requirement 20.17: The bookmarks manager.**
- *Statement:* A panel of the strip, opened by Command-Option-B on macOS and Control-Shift-O elsewhere and from the menus, must show the folder tree and a folder's contents, search names and addresses, and open, edit, remove (with Undo for ten seconds), add folders, move and sort; the trusted core must send it at most 100 items at a time with view numbers and act only by those numbers.
- *Reason:* Bookmarks must be managed, and requirement 3.6 holds for them as for history.
- *Verification:* Interface and trusted-core tests.

**Requirement 20.18: The bookmarks bar.**
- *Statement:* An optional row of the strip, off by default and shown or hidden by Command-Shift-B on macOS and Control-Shift-B elsewhere, must show the Bookmarks bar folder, drawn from the trusted core's view.
- *Reason:* Browser-style quick access, kept in the trusted strip.
- *Verification:* Interface test.

**Requirement 20.19: Bookmarks are kept.**
- *Statement:* Bookmarks must be kept in `bookmarks.txt` in the settings folder, written whole to a temporary file renamed over it at each change.
- *Reason:* As requirement 19.1, for bookmarks.
- *Verification:* Tests of round trip and of an interrupted write.

**Requirement 20.20: The bookmarks file is read strictly.**
- *Statement:* `bookmarks.txt` must be read as section 3.6 of `history-and-bookmarks.md` defines it: a version line, at most 24 MiB, unique numbers, each folder before its contents so no cycle can be read, depth at most 16, each field within its bound and rule; a line that breaks a rule is dropped and logged with anything inside it.
- *Reason:* The file is outside input.
- *Verification:* Test of each rule; the fuzz target `bookmarks_file`.

**Requirement 20.21: The session is always kept.**
- *Statement:* The trusted core must keep the window's place, size and maximised state and every tab but private ones, each with its back-and-forward list (at most 50 entries, nearest its place), its place and its mute, and which tab is active, in `session.txt` in the settings folder, readable by the user alone, written whole after every change of the tabs and within a second of the window's last move or resize.
- *Reason:* The owner's instruction: the system must always be able to restore the current state of windows and tabs, whatever ends it.
- *Verification:* Tests: each kind of change writes the file; private tabs are never in it; a write stopped before its rename leaves the last one.

**Requirement 20.22: The session is restored at start.**
- *Statement:* With "Restore previous session" on, the browser must start with the kept session's window, tabs, lists and active tab, the command line's addresses opening as further tabs; with it off or with no session, as before.
- *Reason:* The owner's instruction: restoring is what happens when the application starts.
- *Verification:* Tests of the restore, with and without the setting and with command-line addresses; a demonstration on each desktop.

**Requirement 20.23: The session file is read strictly.**
- *Statement:* `session.txt` must be read as `history-and-bookmarks.md` section 15.4 defines it, every field within its bound and rule; what breaks a rule is dropped and logged.
- *Reason:* The file is outside input.
- *Verification:* Test of each rule; the fuzz target `session_file`.

**Requirement 20.24: Restoring is a setting.**
- *Statement:* The settings must include "Restore previous session", on by default, kept in `settings.txt` and changed in the settings panel.
- *Reason:* The owner's instruction.
- *Verification:* Test.

### Amendments

- **3.6:** "beyond what is on screen" made exact: the interface receives history and bookmarks only as the views of `history-and-bookmarks.md` section 4.
- **12.5:** the History and Bookmarks menus hold what section 6 lists.
- **12.6:** State: history has the panel of requirement 20.4.
- **19.1 and 19.2:** bookmarks leave `settings.txt` for `bookmarks.txt`; the `bookmark` line is no longer read or written there; `history` and `bar` lines are added.
- **19.5:** withdrawn, replaced by requirements 20.15, 20.16 and 20.19. **Was:** "The star must add or remove the open channel's address, at most 1 000 kept, in the order added, between runs."
- **19.7:** the start page lists bookmarks by name in the order of section 3.5.

## 13. For the owner: decision 57

*Decided on 22 October 2026 while the owner was away, as the owner had instructed:* items 1 to 9 and 11 as recommended; item 10 otherwise, with no time-zone crate (section 14, item 1).


Adopting this design is decision 57. Each item has a recommendation; any can be changed.

1. **History is one row per page per day**, kept 90 days, at most 50 000 rows and 16 MiB. *Recommended*, as Chrome's history page shows it.
2. **History and bookmarks are panels of the trusted strip**, not teletext pages like the start page. *Recommended:* a search field, check boxes and dragging need the strip's controls, a teletext page shows about 20 rows of 40 characters, and a page would pass the history through a decoder.
3. **Suggestions in the address field, with inline completion only from typed addresses.** *Recommended.*
4. **Private tabs.** *Recommended:* browser-style, and small here, since cookies already live only as long as a fetcher.
5. **Recently closed tabs, 25, in memory only.** *Recommended;* restoring the last session is its own design.
6. **The star bookmarks the page shown**, with "The whole channel" in its popover, instead of today's channel only. *Recommended*, as every browser keeps the exact address.
7. **Folders, the two fixed folders, and a bookmarks bar off by default.** *Recommended.*
8. **Export and import later**, first in the browser's own format read by the broker, the Netscape HTML format only if wanted (section 3.7). *Recommended.*
9. **Today's bookmarks in `settings.txt`:** move them once into `bookmarks.txt` at the first start, or not (there has been no release, so the owner's own file is the only one with any). *Recommended:* no moving code; the owner's bookmarks are re-added by hand or by a one-off edit of the file.
10. **A time-zone crate for the trusted core** (section 7). *Recommended:* `jiff`, MIT or Unlicense, pure Rust, which reads the system's time-zone database on macOS and Linux and carries its own copy for Windows; a library is the owner's choice (`DEVELOPMENT.md` section 7). Its exact version and its Windows behaviour are to be checked before the build.
11. **"Remember history" on by default.** *Recommended*, as in every browser.

## 14. What building changed

Built on 22 October 2026. Building changed these, each for the owner to confirm with decision 57:

1. **The local time comes from the broker, with no time-zone crate** (item 10). The broker, which is not confined, answers a new request with the time and the local zone's offset, read by the operating system's own rules: `localtime_r` on macOS and Linux, `GetTimeZoneInformation` on Windows, in `confine`'s modules permitted unsafe code (requirement 10.12). The trusted core's confinement is unchanged: it reads no time-zone file, so section 7's change to the Linux profile was not needed. glibc's `localtime_r` reads the zone only at its first call; since 23 October 2026 `tzset` is called first, which reads `TZ`, or `/etc/localtime` again when it changed, so a change of zone while the browser runs is seen at once on every desktop (`a_change_of_zone_is_seen_at_once`, which fails on Linux without it).
2. **A row keeps its zone's offset** (an eighth field of `history.txt`), so its time of day reads as it was where the user was.
3. **What a visit is.** A navigation the trusted core makes: an address typed, Home, a bookmark, a history row, a suggestion, a link, Back and Forward and their lists, a reopened tab, a new tab. A page chosen on the wall by its number or a click is the interface's own choice of what to show, not a navigation, so it is not a visit. A visit is recorded when the tab's channel draws its first frame after the navigation; another page of a channel already drawing is recorded at once.
4. **A new tab from a row, a bookmark, the bar or a suggestion is Shift-click**; since 23 October 2026 (decision 71) also Command-click (Control-click elsewhere) and the middle button, which open nothing else: on a folder, a check box or a button they do nothing.
5. **Moving a bookmark is by dragging** onto a folder in the tree, or onto an item of the open folder to go before it; since 23 October 2026 (decision 71) also by a row's "Move…", which lists the folders the tree lists, but the item itself, and moves it to the end of the one chosen.
6. **The star's popover lists at most the first 100 folders**, in order, to choose from.
7. **Export writes `bookmarks-export-<day>.txt` beside `bookmarks.txt`**, never over another, and says where. **Import reads a file in this browser's format by its path**, typed in the manager, through the broker's existing file read (no new power for the core, decision 58), into a folder "Imported" under Other bookmarks. Since 23 October 2026 (decision 71) another browser's Netscape bookmark file is read too, by a decoder started for it (section 3.7); it is not written.
8. **"Remember history"** is a setting of the settings panel, "History: Remember, Forget". Turned off, the history panel says history is not remembered and offers Clear history; nothing is removed by turning it off. *Removed by decision 65:* every page of an ordinary tab is recorded and a private tab records none; a `remember` line in `settings.txt` is dropped and said (`core/src/settings.rs`).
9. **The menu button opens a menu**: New tab, New private tab, Reopen closed tab, History, Bookmarks, the bookmarks bar, Bookmark this page, the phone keyboard (which the button opened directly before), the developer console and Settings. Its shortcuts are spelled out ("Shift+Cmd+N", "Ctrl+Shift+N"), and the folder marks and the star beside a suggestion are drawn: the browser's fonts have no key symbols, triangles or stars.
10. **While a panel, popover or list of the strip is open, it takes the keys**, not the page's channel. Before, on a channel that takes keys, Return in the star's popover went to the channel and the popover stayed open (`the_strips_panels_take_the_keys_from_the_channel`).
11. **Opening a panel closes the others**, and the star's popover as Done.
12. **Command-H opens History everywhere**, as Control-H does away from macOS; on macOS the system may take Command-H to hide the window first, and Command-Y is History's key there.
13. **Undo** is kept by the trusted core until the next removal; the manager offers it for ten seconds.
14. **The star's default name** is the channel's title, with the page's number and title when the address names a page.
15. **History expires** on the first visit, or the first opening of the panel, of each local day, rather than at midnight.
16. **Bookmarks in `settings.txt` are not moved by code** (item 9). On the owner's Mac there were none to move: on 22 October 2026 the settings folder held the earlier prototype's `settings.txt`, in another format, with no `bookmark` line.
17. **A bar folder's list** shows its bookmarks to open; folders inside it are listed but not opened from the bar.

## 15. Restoring the window and its tabs

Added on 22 October 2026 on the owner's instruction: "the system must always be able to restore the current state of windows and tabs, and this is what should happen when the application starts too. Settings must include to enable/disable restore previous session" (decision 59).

### 15.1 What is kept

The **session**: the state of the window and every tab in it, as it is now.

| Kept | What | Bound |
|---|---|---|
| The window | Its position and inner size in logical points, and whether it is maximised | Each coordinate within ±32 768, each size from 1 to 32 768 |
| Each tab, in order | Its back-and-forward list (each entry's address, whether a page started it, and its title), its place in that list, and its mute | At most 64 tabs (`protocol::MAX_TABS`); at most 50 entries a tab, those nearest its place |
| The active tab | Its place among the tabs | |

**Private tabs are never kept** (requirement 20.8): a session restored has none of them, and closing the browser with only private tabs open keeps no tab. Since decision 72 every window is kept, each with its place, its tabs and its active tab (`multiple-windows.md`).

### 15.2 Always current

The trusted core writes the session to `session.txt` in the settings folder, readable by the user alone, whole to a temporary file renamed over it (as `settings.txt`), **after every change of the tabs**: a navigation, Back or Forward, a tab opened, closed, chosen, muted or reopened. A change of the window's place or size is written once it has been still for a second, so a drag is one write. So whatever ends the browser, a crash or a power cut among them, the last state written is the state restored. A write is made only when the session's text changed.

### 15.3 At start

With **"Restore previous session"** on (the default; `restore on` in `settings.txt`; "Tabs at start: Restore, Home" in the settings panel), the browser starts with the kept session: the window where and as large as it was, the same tabs in the same order with their back-and-forward lists, the same tab active. Addresses given on the command line open as further tabs, the last of them active. With it off, or with no session kept, the browser starts as before: the command line's addresses, or the home address. The session is still kept with the setting off, so turning it on restores the browser as it was last.

A restored tab fetches its channel afresh, as a navigation does; it is not a visit until its page draws, as any navigation (requirement 20.1). A window position that no longer falls on a display (a display unplugged) is not used, and the system places the window.

### 15.4 The file

```
neoteletext-session 3
window	120	80	1440	900	0
active	1
tab	0	1
entry	0	1792656300	https://news.example/teletext/	news.example
entry	1	1792656360	https://news.example/teletext/#302	WEATHER
tab	1	0
entry	0	1792656000	neoteletext:home	NeoTeletext
closed	0	0
entry	0	1792655000	https://sport.example/	sport.example
```

*Version 3, decision 72:* a `window` line begins each window, with its place and size, or alone when the interface never reported them; its `active` line and its tabs follow it. A `closed` line may carry a third field, the number of the window it closed with: the closed tabs of one window reopen together. At most 16 windows and 64 tabs among them. A version 2 file is not read.

*Version 2, decision 64:* each entry carries the time the tab last came to it, in seconds since 1970 (UTC), so that clearing history by time reaches the tabs' lists; `closed` lines, after the tabs, hold the recently closed tabs, most recent first, at most 25. A version 1 file is not read.

Tab-separated: `window` with x, y, width, height and maximised (0 or 1); `active` with the active tab's place; each `tab` with its mute and its place in its list, followed by its `entry` lines, each with whether a page started it, its address and its title. **Read strictly**, as `history.txt` is: the version line; at most 8 MiB (64 tabs of 50 entries of the longest line, rounded up); every field within its bound and rule, every address one the address rules accept, every title clean; a tab with no entry, or a place beyond its entries, is dropped with its entries; an active place beyond the tabs is the first. A fuzz target, `session_file`. Another program can write the file; it can already change the user's files.

### 15.5 Security

The session file holds addresses, as history does: it is kept on the user's disk only, readable by the user alone, sent nowhere, and never holds a private tab. Restoring a session opens its addresses as navigations the user started, as the bookmarks do; each passes the address rules again, and a local-network address still needs its permission (requirement 5.10). The window's geometry is the interface's report, checked within its bounds by the trusted core; a compromised interface could make the window open elsewhere next time, nothing more.

### 15.6 What building settled

1. **The window opens with the trusted core's first message.** The interface waits for it, and the trusted core sends the kept window first, before it opens any tab, so the window is never seen to jump. With no session restored, the first message is the settings, as before.
2. **A maximised window keeps the place and size it returns to**, with the maximised mark, as other browsers do; restored, it opens there and is maximised.
3. **Where the system does not say where the window is** (Wayland), the interface reports nothing and the session keeps no window line; the system places the window.
4. **The session is written once at start**, so a start with restoring off replaces the last session with the new one: "Restore previous session" off means the browser starts at Home, not that the last session is kept aside. Turning it on later restores the browser as it was when it was last closed.
5. **A tab's back-and-forward entries keep the title** drawn for them; an entry not yet drawn has an empty title until it is.

## 16. Chrome's rules (decision 64)

The owner asked on 23 October 2026 that history follow Chrome. Each rule below was read from Chrome's source that day (`chromium/src`, main); where ours differed it now follows Chrome, and where Chrome has nothing to model ours stays, listed for the owner.

| Rule | Chrome, and where | Ours before | Now |
|---|---|---|---|
| How long history is kept | 90 days (`HistoryBackend::kExpireDaysThreshold`) | 90 days | Same |
| What a visit is | Each committed main-frame navigation, not an error page (`HistoryTabHelper::DidFinishNavigation`, `RenderFrameImpl`: `should_update_history = !HasUnreachableURL()`) | Each navigation, when its page first draws; a page that never draws is not one | Same in effect: a channel's page drawing is its commit |
| A reload | A visit row, the last visit moved, the visit count and typed count not raised (`HistoryBackend::AddPageVisit`, `IsTypedIncrement`) | Counted as a visit until decision 63 | The day's row's last visit moves; nothing counted |
| The address shown, entered again or linked to | A reload: no new entry (`chrome.history` "reload: … pressing Enter in the address bar"; `ShouldTreatNavigationAsReload`) | A new entry and a visit | A reload |
| Back and Forward | A visit, counted, never typed (`PageTransitionIsNewNavigation`) | The same | Same |
| What history shows | One row per address per day, the latest visit (`REMOVE_DUPLICATES_PER_DAY`) | One row per address per day | Same |
| A row's menu | "More from this site" (`host:` search), "Remove from history", "Remove bookmark" when bookmarked (`history_list.html.ts`) | A remove button; "Forget this channel" for selected rows | The same three |
| Clearing ranges | Last 15 minutes, hour, 24 hours, 7 days, 4 weeks, all time (`browsing_data::TimePeriod`) | Without 15 minutes | With it |
| Removing history and the tabs | The entries of that time, or of addresses wholly removed, leave every tab's list but its current entry, and the closed tabs; a closed tab whose current entry goes is dropped; incognito untouched (`browsing_data::RemoveNavigationEntries`, `TabRestoreServiceHelper::DeleteFromTab`) | Not touched | As Chrome; each entry now keeps the time the tab came to it |
| Recently closed tabs | 25 (`TabRestoreServiceHelper::kMaxEntries`), kept between runs (`TabRestoreServiceImpl::LoadTabsFromLastSession`) | 25, in memory | 25, kept in `session.txt` |
| A tab's list | At most 50 entries, the oldest dropped (`blink::kMaxSessionHistoryEntries`) | Unbounded in memory, 50 kept | 50 |
| The Back and Forward menus | 12 entries, then up to 5 chapter stops by registrable domain, then "Show full history" (`BackForwardMenuModel::kMaxHistoryItems`, `kMaxChapterStops`) | 15 entries | 12 and the chapter stops, by host: decision 62 dropped the Public Suffix List |
| Suggestions | Up to 8 on a desktop (`AutocompleteResult::GetMaxMatches`), scored by typed count, visits and recency; inline completion from typed addresses | Up to 8: bookmarks, then history by visits and recency; inline from typed addresses | Unchanged; Chrome's scoring is not modelled |
| Turning history off | No setting (only an enterprise policy) | "Remember history" | Removed (decision 65) |
| Removing a whole site | No such command on the history page | "Forget this channel" | Removed (decision 65); "More from this site", then select and remove |
| The file's size | No bound but age | 50 000 rows, 16 MiB | Kept: every file the trusted core reads is bounded |
| Private tabs | Incognito records nothing and has no closed tabs of its own (`TabRestoreServiceFactory`: the original profile only) | The same | Same |

**Where Firefox differs**, read from its source the same day, for the record: it stores a reload as a visit of its own type but skips it when the address was visited in the last 6 minutes (`History.cpp`, `RECENTLY_VISITED_URIS_MAX_AGE`), and counts none (`nsPlacesTriggers.h`, `VISIT_COUNT_INC`); and it records no visit for Back or Forward at all (`nsDocShell::ShouldUpdateGlobalHistory`, `LOAD_CMD_HISTORY`), where Chrome records and counts one. On the owner's instruction Chrome's rules are the ones followed; for a reload the two agree on what is counted (decision 63).

**The host for the registrable domain.** Chrome's chapter stops change at each registrable domain, which needs the Public Suffix List; this browser has none (decision 62), so they change at each host. Why, and what that costs, is `cookies.md` section 1.1. *Decided by decision 65,* the owner having left it to the implementer: the host stays. Two hosts of one organisation make two chapters rather than one, a small difference in one menu; shipping the list for it alone would add data to keep current, and an old copy would give wrong answers, for no gain in safety.

