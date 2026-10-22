# NeoTeletext: settings, the home page, the developer console and proxies

| | |
|---|---|
| **Status** | Adopted 21 October 2026 on the owner's instruction to design, specify, number the requirements of and build each of these (decision 51). Requirements: section 19 of [`requirements.md`](requirements.md), with requirements 11.2, 12.6 and 12.7 amended. Security: [`settings-home-console-proxy-security.md`](settings-home-console-proxy-security.md). Section 7 lists what building changed. |
| **Engine** | Proxy encoding and the agent's report of each exchange are the engine's (its decision E.102, REQ-WEB-032 and REQ-WEB-033), so `neottx` has them too. |

## 1. Settings

The browser keeps a few settings between runs, and the bookmarks with them. The trusted core owns them; the interface only shows them and asks for changes (requirements 3.6 and 12.6).

### 1.1 What is kept

| Setting | Values | Default | Where it is changed |
|---|---|---|---|
| Home | An address, or `neoteletext:home` | `neoteletext:home` | The settings panel; "Use this page" |
| Sound | On or off | On | The settings panel |
| Volume | 0 to 100 | 100 | The settings panel |
| Proxy | None, an HTTP proxy or a SOCKS5 proxy, with its host and port | None | The settings panel (a security setting) |
| Local addresses allowed | Up to 16 addresses on this computer or the local network | None | The settings panel (a security setting) |
| Bookmarks | Up to 1 000 addresses | None | The star in the strip |

Nothing else is kept: no history, no cookies (the engine keeps cookies only for the life of a fetcher, its REQ-SEC-013), no window position. *Amended 22 October 2026:* history and bookmarks are kept (decision 57), and the window and its tabs as the session (decision 59), each in a file of its own (`history-and-bookmarks.md`).

### 1.2 The file

One text file, `settings.txt`, in the platform's place for an application's settings:

- macOS: `~/Library/Application Support/NeoTeletext/`
- Windows: `%APPDATA%\NeoTeletext\`
- Linux and other Unix systems: `$XDG_CONFIG_HOME/neoteletext/`, or `~/.config/neoteletext/` when that is not set.

Each line is a name, one space, and a value: `home neoteletext:home`, `sound on`, `volume 80`, `proxy socks5 127.0.0.1 1080`, `allow-local 192.168.1.20`, `bookmark https://news.example/teletext/`. The first line is `neoteletext-settings 1`, the format's version. The file is at most 256 KB and a line at most its name, a space and a value of 2 048 bytes, the longest address; a value is checked as the setting's own rule says (an address by the trusted core's address rules of requirement 11.3, a number within its range). A line that breaks a rule, or a name not known, is dropped and written to the log; the rest stand. The file is written whole to a temporary file beside it and renamed over it, so a crash never leaves half a file.

### 1.3 The settings panel

A panel of the trusted strip (requirement 11.1), opened by a gear button at the right of the strip, by Command-comma on macOS and by Control-comma elsewhere, and closed by Escape or the button again. It is drawn by the interface from the trusted core's settings view, in the strip's own colours, over the page area as the phone keyboard's panel is; no page can draw it or send it input, and its input reaches the trusted core as strip input. It holds:

- **Home:** the address, a field that takes an address on Enter, and "Use this page", which takes the open channel's address.
- **Sound:** a switch, and the volume with minus and plus buttons in steps of 10.
- **Proxy:** None, HTTP or SOCKS5, and a host and port field.
- **Local addresses:** the list, each with a remove button, and a field to add one.

The trusted core checks every change; a refused one is said in the panel ("Not an address on this computer or the local network"), and nothing is kept. The interface receives the security settings only as the cleaned text the panel shows, as it receives the strip's "Not secure": it acts on none of them, and a change it asks for is checked by the trusted core as any strip input is (requirement 3.6, amended by decision 51).

## 2. The home page

### 2.1 The Home button

A Home button, a house, sits in the strip after Forward (requirement 11.2). It, Command-Shift-H on macOS and Alt-Home elsewhere open the home address in the active tab, as a navigation the user starts (requirement 8.2). A new tab opens the home address.

### 2.2 The start page

The default home is `neoteletext:home`, a channel the trusted core serves itself as teletext, with no network: its pages are TTI the trusted core writes and gives to a decoder as a channel's pages.

- **Page 100**, NeoTeletext: how to open a channel ("Type a channel's address above"), the keys, and the bookmarks, each with a page number from 201: `201 news.example/teletext`.
- **Pages 201 to 299**: one a bookmark, which says where it leads and opens it.

Choosing a bookmark, by its number or by clicking it, opens it in the tab as the user's own navigation: the start page is the browser's, so its links are not a page's and need no site-change screen (requirement 8.4 does not apply to it). Its text is built from the bookmarks' addresses, cleaned as every address is (requirement 11.5). Any other `neoteletext:` address is refused.

## 3. The developer console

### 3.1 What it shows

A panel docked at the bottom of the window (requirement 12.7), opened by Command-Option-I on macOS and Control-Shift-I elsewhere, for the active tab's channel, with tabs:

- **Network:** each exchange the fetcher made, newest last: the time, the method, the address, the status, the bytes received and how long it took; and the push connection's events: connected, each message's kind and size, lost and why.
- **Packets:** each packet received on the tab, newest last: the time, where it came from (fetched or pushed), its magazine and row, the page number of a header, and its 40 data bytes as text and as hexadecimal.
- **Pages:** the pages the channel's decoder holds, with their titles and when each last changed.
- **Notices:** the engine's and the trusted core's notices for the channel: refusals, warnings, protocol errors.
- **Cookies** (decision 63, 23 October 2026): the cookies panel of `cookies.md` section 4.1, for every site and channel rather than the active tab's alone: counts, sizes, last use, delete and block, never a name or value. Pause and Clear are the other tabs'; the settings panel's "Manage…" opens the console here.

Each of the first four tabs keeps its newest 1 000 entries a channel. The panel can pause, which stops it scrolling, and clear. It never shows a cookie, a form's values, a key typed into a page or the pairing secret: the interface never receives those (requirement 3.6).

### 3.2 Where it comes from

- The **fetcher** reports each exchange (the engine's REQ-WEB-033) and each push event to the trusted core.
- The **trusted core** already passes every byte the fetcher received to the decoder; it reads each 42-byte packet's address with the engine's Hamming decoder as it goes, and keeps the packet entries.
- The **decoder** already reports the pages it holds.
- The trusted core keeps each tab's entries and sends the open tab's to the interface while the console is open, as text it has cleaned, at most 200 entries a message.

Everything shown is the channel's own data, shown as text, never interpreted.

## 4. Proxies

### 4.1 What is supported

- **HTTP proxies,** by `CONNECT` (RFC 9110 section 9.3.6): every connection, plain or TLS, is a tunnel through the proxy, so TLS stays end to end and the proxy sees only the host and port.
- **SOCKS5 proxies** (RFC 1928), with no authentication, sending the host's name for the proxy to resolve (address type 3).

Proxies that need credentials are not supported: the browser would have to hold a password. Proxy settings of the operating system are not read; the proxy is the browser's own setting.

### 4.2 What goes through it

Every connection the fetcher makes: descriptors, contents files, pages, push. The phone keyboard's relay connections, and `neottx`'s, go through the same encoding (`neottx --proxy`).

### 4.3 The local network

With a proxy, the proxy resolves names, so the browser does not learn the addresses a name resolves to. The local-network rule (requirement 5.10, the engine's REQ-SEC-006) is then kept on what the browser can see: an address written as a local IP address, `localhost` and names under `.local` and `.internal` are refused unless allowed, before anything is sent to the proxy. A public name that resolves, at the proxy, to an address on the proxy's own network reaches it; that network is the proxy's to protect (residual risk 2 of the security analysis). The proxy's own address may be local: it is the user's choice.

### 4.4 Errors

A proxy that refuses, answers with anything but success, or does not answer within the engine's connection timeout fails that connection, with "The proxy refused the connection to *host*" in the strip's notice; nothing goes direct instead.

## 5. Messages

| Message | From | To | Carries |
|---|---|---|---|
| `Settings` | Trusted core | Interface | The settings view: home, sound, volume, proxy, local addresses, the last refusal |
| `ChangeSetting` | Interface (strip input) | Trusted core | One change, checked again by the trusted core |
| `Home` | Interface (strip input) | Trusted core | Open the home address |
| `Console(open)` | Interface | Trusted core | Start or stop sending the console |
| `ConsoleEntries` | Trusted core | Interface | A tab's new entries, cleaned text |
| `Exchange`, `PushEvent` | Fetcher | Trusted core | One exchange or push event, bounded |

The fetcher takes its proxy as a command-line argument when it is started, as it takes the channel's address: no message changes where it connects (requirement 4.7).

## 6. Requirements

Requirements 19.1 to 19.12 of `requirements.md`.

## 7. What building changed

Built on 21 October 2026. Building changed these:

1. **A line's bound.** A line of 2 048 bytes could not hold the longest address the address rules accept, so such a bookmark would have been dropped at the next start (the test `the_longest_address_is_kept` failed). A line is now at most its name, a space and a value of 2 048 bytes; requirement 19.2 is amended.
2. **The strip's notice** is a field of the strip's state, text the trusted core writes; it takes the address's place for ten seconds. A proxy's failure reaches the trusted core as `ProxyFailed`, a message that carries nothing, and the trusted core says it in its own words with the channel's host as the strip shows it, so a fetcher cannot write the strip. The fetcher calls any failure on the way to the proxy the proxy's: its name not found, no answer, a refusal.
3. **Push events** are not a message of their own (section 5's `PushEvent`): the trusted core reads them from the fetcher's existing `Pushing` and pushed `Page` messages. The Network tab shows "push: connected", "push: lost" and each pushed message's form and size.
4. **The Pages tab** is a log of each page's arrival, with its title, and of each change, rather than a table of the decoder's store.
5. **The Packets tab** lists the packets a fetcher received as T42, fetched or pushed. A file channel's data and TTI pages are not packets; the trusted core does not read a file channel's bytes, so they are not listed.
6. **The console** docks in the overview's place while it is open. Clear empties the interface's view; the trusted core keeps its own entries.
7. **The settings panel** is centred over the window below the strip, not over the page area, which is too small for it at the smallest window. The gear sits to the left of the mute-all and menu buttons. Volume steps by 10. A proxy's kind, host and port go together with Apply or Enter; None goes at once.
8. **Shortcuts.** The interface core is the same on every platform, so it takes each platform's shortcut everywhere: Home is Command-Shift-H (Control-Shift-H away from macOS) and Alt-Home; the console is Command-Option-I and Command-Shift-I (Control-Shift-I away from macOS). Shortcuts are read from the key without its modifiers, as Option changes the character a key types on macOS.
9. **Local addresses** allowed in the settings reach fetchers started after the change, as the proxy does (requirement 19.12); `--allow-local` on the command line still adds to them.
10. **The start page** lists 15 bookmarks on page 100, with a line naming the pages of the rest; pages 201 to 299 hold the first 99 bookmarks, and later ones are kept but have no page. Its page list shows each bookmark's address as the title. It is written when the tab opens it, so a bookmark added while it is shown appears the next time it opens. Characters the English G0 set draws as other symbols (`#`, `[`, `\`, `]`, `^`, `_`, `` ` ``, `{`, `|`, `}`, `~`) show as `?`. The strip names it "NeoTeletext" where a channel shows its padlock or "Not secure". Going back from a bookmark it opened returns to page 100, not to the bookmark's page, which would open it again.
11. **The phone keyboard** process takes `--proxy` when it starts, as the fetcher does.
12. **The view types** of the settings and the console live in the interface core, as the strip's do, and the protocol encodes them.
