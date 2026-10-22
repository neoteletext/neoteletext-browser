# NeoTeletext: security analysis of history and bookmarks

| | |
|---|---|
| **Status** | Written 22 October 2026 with [`history-and-bookmarks.md`](history-and-bookmarks.md), whose design takes its findings; adopted and built with it the same day (decision 57). Section 5's changes are made in `security-analysis.md`. Building changed two answers here: the time zone comes from the broker, so the core reads no time-zone file (row "Time zone"); and while a panel of the strip is open it, not a page's channel, takes the keys. |
| **Method** | That of [`security-analysis.md`](security-analysis.md): what is protected, who attacks, each boundary crossed, the answers, and the risk that remains, ranked. |

## 1. What is protected

- **The user's history:** which channels and pages they read, and when. It is the most private thing this browser will keep: until now it kept no record of reading at all (`settings-home-console-proxy.md` section 1.1, "no history").
- **The user's bookmarks**, and that each leads where it says.
- **The truth of the strip:** the address field, its suggestions, and the panels drawn as part of it.
- **The trusted core's readers** of the two new files.

## 2. Attackers

A channel's author, who writes channel and page titles and chooses which pages exist; a compromised decoder, which reports page titles; a compromised interface process; another program of the same user, which can read and write the settings folder; someone with later access to the computer, for whom history is the target.

## 3. Boundaries and answers

| Boundary | Threat | Answer |
|---|---|---|
| Channel → titles → history | A hostile title: hidden or direction-changing characters, a title that reads as an address (`https://bank.example`), one that is very long, one that forges a day heading | Titles are cleaned and cut before they are kept (requirement 11.5), and kept cleaned; wherever shown, the address is printed beside the title from the parsed address, never replaced by it (requirement 20.13); titles never complete the address field (requirement 20.11). |
| Channel → history | A channel floods history: many pages, many visits, to push the user's real history out | Only navigations to a page that draws are visits, and a page changing under the user is not one (requirement 20.1); a channel holds at most 800 pages, and a page is one row a day however often visited; the bounds remove the oldest first. A channel whose pages the user walks through all day makes at most 800 rows a day; 50 000 rows hold 62 such days. |
| Channel → suggestions | A channel makes the address field complete to its address, so Enter goes there | Inline completion only from addresses the user typed (requirement 20.11). A suggestion below the field is shown with its own address and needs choosing. |
| Decoder → trusted core | A compromised decoder reports titles for pages the user never saw, or many | Titles are bounded by `protocol::MAX_PAGE_TITLE` and read by the fuzzed reader (`decoder_messages`); a row is made only by the trusted core's own navigation and the frame it drew, not by a decoder's report. A forged title is hostile text, as above. |
| Interface → trusted core | A compromised interface pages through the history panel and the bookmarks manager to read everything, then navigates a tab to an address that carries it out | Views are at most 100 rows, and requests act only by view numbers (requirements 20.5 and 20.17), so it cannot invent rows or addresses through them. It **can** still ask for the panel and scroll through it, reading every row, and it can already navigate a tab to any address the address rules accept. This is decision 33's accepted exposure, that the interface draws the strip and sends strip input, with more to find; open decision 3 (the strip in a smaller process) is the remedy. Ranked first below. |
| Interface → trusted core | A compromised interface removes history or bookmarks, or adds a bookmark to an attacker's channel | Removal by view numbers only; every change is strip input, checked as such (requirement 3.9). A bookmark added this way is visible in the manager and the star. The same exposure as above. |
| Trusted core → channels | History or a typed address leaks to a channel through a decoder, fetcher or keyboard | None is sent to them (requirement 20.12). The start page's decoder receives bookmarks' names and addresses, as it receives addresses today; the start page is the browser's own channel and its decoder has no network. |
| Files → trusted core | `history.txt` or `bookmarks.txt` written by another program: huge, binary, cyclic folders, endless depth, a line breaking a rule | Read strictly with bounds (16 MiB and 24 MiB, seven fields, unique numbers, a folder before its contents, depth 16); anything else dropped and logged; fuzzed (`history_file`, `bookmarks_file`) (requirements 20.3 and 20.20). A program that can write them can already change the user's files; it is not defended against. |
| Trusted core → files | A crash leaves half a file; a removal stays readable in the file | `bookmarks.txt` is written whole and renamed; `history.txt` is appended and compacted, and compacted at once after any removal, before the panel says the rows are gone (requirement 20.6); a broken last line is dropped on reading. |
| The disk | Someone with later access, or another user of the computer, reads the history | On macOS and Linux both files are created readable and writable by the user alone (mode 0600), since a home folder may be readable by others; on Windows `%APPDATA%` is the user's own. They are not encrypted, as no major browser encrypts its history. Removal rewrites the file; it cannot promise that a solid-state disk or a backup holds no earlier copy, which no browser can. Private tabs leave no row (requirement 20.8); "Remember history" was withdrawn by decision 65, as Chrome has none. |
| Private tab → history | A private tab leaves a trace | No row, typed address or recently closed entry (requirement 20.8); suggestions read history but write nothing. The operating system's own records (swap, crash reports) are outside the browser. |
| Bookmarks → start page | A name or address breaks out of its line of the start page's TTI | As today: cleaned, no control characters, each cut to a row (`settings-home-console-proxy-security.md` section 3); now also names, tested with hostile names. |
| Time zone → trusted core | The time-zone files are outside input | The trusted core reads none: the broker, unconfined, reads the offset with the operating system's own functions and sends two numbers, the offset checked to be one a zone can have (`history-and-bookmarks.md` section 14, item 1). A wrong offset puts rows on the wrong day, nothing more. |
| Interface → trusted core: the window | A compromised interface reports a window place to keep | Read by the protocol's reader within its bounds (each coordinate within ±32 768, each size 1 to 32 768), kept, and sent back at the next start; a place on no display is not used. The worst is a window that opens somewhere odd next time. |
| Files → trusted core: the session | `session.txt` written by another program: huge, binary, endless lists, addresses the rules refuse, hostile titles | Read strictly with bounds (16 MiB since decision 64, 64 tabs and 25 closed tabs, 50 entries a tab, an entry's time from 1970, each field within its rule, titles clean); a broken tab dropped whole; fuzzed (`session_file`) (requirement 20.23). Each address restored passes the address rules again and opens as a navigation the user started, as a bookmark does; a local-network address still needs its permission (requirement 5.10). |
| The disk: the session | The open tabs' addresses on the disk | Mode 0600 as history; never a private tab (requirement 20.21). A browser closed with only private tabs open keeps none. Since decision 64 it also holds the recently closed tabs, as Chrome keeps them, which are what was read: removing history removes them as it removes rows, and a private tab never becomes one. |
| Import (not proposed for the first build) | A hostile bookmark file from another browser | Not built. If built: the browser's own format only, read by the broker as a file channel is and by the same strict reader; the Netscape HTML format only by a confined helper, fuzzed (`history-and-bookmarks.md` section 3.7). |

## 4. Risks that remain, ranked

1. **A compromised interface reads all history and bookmarks through the panels, and carries them out by navigating a tab** (low likelihood: it needs a fault in the interface first; high consequence for privacy). The same root as decision 33's accepted strip exposure; history makes it worth more. Remedy: open decision 3. A partial control, if wanted: the trusted core could limit how fast views are sent, which slows reading without stopping it.
2. **History and the session on the disk** (likelihood depends on the computer; medium consequence): as in every browser. Private tabs and the setting are the user's controls; the session holds only what is open.
3. **A channel's title misleads in a list** (low, low): always beside its real address, cleaned.
4. **The files changed by another program of the user** (low, low): outside the browser's protection, as any of the user's files is.

## 5. What changes in `security-analysis.md` when this is built

- **Section 2, what is protected:** history and bookmarks.
- **Section 5.5 and 5.6:** the views and the requests of section 3 above.
- **Section 12, privacy:** history is kept for 90 days, on the user's own disk only, never sent anywhere; the setting and private tabs.
- **Section 16:** risk 1 above, beside the existing rank for a fault in the interface.

## 6. Checks that keep these properties

The tests of requirements 20.1 to 20.24, and in particular: no view over 100 rows; unknown and stale view numbers refused; inline completion never from a link or a title; the file after each removal; hostile titles in every place shown; the three fuzz targets in the full gate (`history_file`, `bookmarks_file`, `session_file`); no private tab in the session; a review that no message to a decoder, fetcher or keyboard carries history.
