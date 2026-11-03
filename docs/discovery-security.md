# NeoTeletext: security analysis of discovery

| | |
|---|---|
| **Design** | [`discovery.md`](discovery.md), designed 3 November 2026 (TODO-2073); not built. |
| **Written** | 3 November 2026, before the build, as `DEVELOPMENT.md` sections 2.3 and 14 ask of a feature that reads outside input. Its findings are applied to the design (section 4). |
| **Rule** | Decision 4 and requirement 1.2: security first; decision 58: no weakening and no fallback to a weaker mode. |
| **Builds on** | [`security-analysis.md`](security-analysis.md) sections 5.1, 5.2, 5.5, 7 and 12; the engine's analysis of the same feature, `docs/web-discovery-security.md` in its git history (removed with the engine's browser on 18 October 2026, commit `94346b0`), whose threats this analysis takes over and brings to the present design. |

## 1. What is protected

| Asset | Why it matters |
|---|---|
| The trusted core | It holds the profile, cookies, history and bookmarks, and decides every navigation. It must never read HTML. |
| The viewer's choice of where to go | A server must not send the viewer to a channel the viewer did not see and choose (REQ-694). |
| The local network | The browser must not become a way to reach routers and devices behind it (requirement 5.10). |
| Encryption | A viewer who typed or reached `https` must not be moved to `http` without seeing it. |
| Each channel's session | Cookies belong to channels (REQ-1883, decision 62); a web page must neither plant nor read them. |
| The viewer's privacy | Which web sites the viewer looks for channels on. |

## 2. Attackers

Any web server the viewer names, or one it redirects to; anyone who can write into a site's page, such as a comment form; someone on the network path of a plain `http` request; a fetcher compromised through the response it reads; a compromised interface process.

## 3. Threats and their answers

### 3.1 Hostile markup attacks the code that reads it

A server sends crafted headers or HTML: endless attributes, unterminated quotes, comments that never close, nested scripts, megabytes of padding, UTF-16, zero bytes.

- **Answer:** the reader is the engine's (requirement 2.1), not a parser of HTML: a byte loop over at most 8,192 bytes that knows comments, raw-text elements, attribute quoting and the `link` element, and stops at the first thing it cannot read exactly. It allocates nothing unbounded, its crate denies panics, it has no unsafe code, and it is a fuzz target. It runs in a fetcher started for this one request, confined as every fetcher is (`sandboxing.md`). The trusted core and the interface never receive HTML: only addresses, which the trusted core reads again.
- **Remaining risk:** a fault in the reader is contained by the fetcher's sandbox; section 3.9 covers what a fetcher taken over could then do.

### 3.2 The reader and a web browser read the page differently

The danger is someone who can inject text into a site's page planting a pointer that a web browser would never see, to send the site's teletext viewers elsewhere.

- **Answer:** the reader stops at `<body` and `</head`, so text injected into the body is never read. It skips comments and the content of `script`, `style`, `noscript`, `template`, `textarea`, `title` and `xmp`, as HTML does, reads attribute values with HTML's quoting, takes the first of two attributes of the same name, and decodes no character reference: a target holding `&` is refused, so an encoded address cannot hide what it is.
- **Remaining risk:** a site that lets strangers write into its head before byte 8,192 already has a script-injection fault on the web; a planted pointer is one more effect of it, and its target still passes section 3.3 and is only shown (section 3.4).

### 3.3 A pointer leads somewhere dangerous

A pointer names a router's address, a `file:` or `javascript:` address, an `http` address from an `https` page, a page file, or something that is not a channel at all.

- **Answer:** the trusted core reads every target again with the address rules a link's target meets (requirement 11.3, REQ-1693): `https` or `http` alone, an address of the URL map, a channel's folder and no page file. It refuses `http` when the final address was `https`. A literal local address is refused at once, and a name is checked against the local network when it is connected to, as every address is (requirement 5.10). A refused target is not shown.
- **Remaining risk:** none beyond a typed address's, which a result cannot exceed.

### 3.4 A result moves the viewer without the viewer seeing where

- **Answer:** discovery never moves a tab (REQ-694). Results are shown in the strip, the browser's own area, with each address whole and its host emphasised, never in the page area a channel draws (section 7 of `security-analysis.md`). Only the viewer's choice opens one, as a page-initiated navigation (REQ-725, REQ-1670), and the address field then shows the real address, as for any link (DEC-1664's reasons). The panel never takes the focus by itself and takes Enter only once the viewer has moved into it (as REQ-811 does for the web prompt), so a result timed to arrive as the viewer presses Enter opens nothing.
- **Remaining risk:** a site can call any channel its own, just as it can link to anything on the web; the browser shows where the viewer goes and does not judge whose edition it is. Look-alike host names are not flagged, as for links (`security-analysis.md` section 16).

### 3.5 Redirects take discovery to another origin

`example.org` redirects to `example.net`, which announces a channel.

- **Answer:** redirects are followed as the engine's agent follows them, at most five, each checked for the local network and for a fall from `https` to `http`. The results panel names the address the viewer typed and, when it differs, the final address that announced the results.
- **Remaining risk:** the request reaches `example.net` before the viewer sees that name; it carries no cookie, no `Referer` and nothing of the viewer, and the redirect was `example.org`'s own choice, as in any web browser.

### 3.6 A page makes the browser contact a web address

- **Answer:** discovery runs only for an address the viewer typed in the address field, and the trusted core knows that, since the field's text reaches it as strip input of its own kind (`Pending::typed`); a link, a bookmark, a redirect, a page's target, a descriptor and a file never start it. A result never starts discovery again.
- **Remaining risk:** none.

### 3.7 Discovery plants or reads a channel's session

- **Answer:** the discovery fetcher is started with no channel, asks the trusted core for no cookie, and every `Set-Cookie` in its answers is ignored; nothing of it reaches a jar.
- **Remaining risk:** none.

### 3.8 Denial of service

A server sends endless headers, an endless body, a body one byte a minute, a redirect loop, thousands of `<link>` elements, or a pointer whose channel points back.

- **Answer:** the engine's header limit; at most 8,192 bytes of body, then the connection is closed; the engine's time limits; at most five redirects; at most 16 results from headers and 16 from elements, 32 in the message, eight shown; one request, one fetcher, ended with it; discovery never runs on a result.
- **Remaining risk:** a site costs at most six small requests and 8,192 bytes for each address the viewer types.

### 3.9 The trusted core trusts the fetcher's report

A fetcher taken over through the reader could report any address, or many messages.

- **Answer:** the trusted core accepts one `Discovered` message, from the discovery fetcher it started, for the tab and the typed address it started it for; another is that fetcher's fault, which ends it (REQ-683's rule). The message is bounded on both sides and fuzzed. Each address is read again by the address rules (section 3.3), and nothing is navigated without the viewer's choice (section 3.4).
- **Remaining risk:** a compromised discovery fetcher can show the viewer a list of channel addresses of its choosing, which any web page the viewer typed could do by announcing them. It holds the network for the life of the one request, as every fetcher does (`security-analysis.md` section 5.1).

### 3.10 A compromised interface

- **Answer:** the interface receives cleaned addresses alone and sends back a choice by its number in the list the trusted core holds; the trusted core navigates to its own copy, never to text the interface sends, as with bookmarks on the start page (REQ-685). A compromised interface can already navigate a tab to any address (decision 33's accepted exposure), so discovery gives it nothing more.
- **Remaining risk:** none beyond decision 33's.

### 3.11 Privacy

- **Answer:** the request carries the engine's fixed `User-Agent`, no cookie and no `Referer`, and runs only when the viewer asks.
- **Remaining risk:** the site learns that a NeoTeletext viewer asked for its address, which is what the viewer asked to do.

### 3.12 The address rules forbid the request itself

DEC-1722 item 1, confirmed by the owner, refuses every `http` and `https` address outside the URL map before any fetcher starts. Discovery's request is for exactly such an address. Making it requires an exception the owner has not granted.

- **Answer:** none can be given here: it is the owner's decision. DEC-2374 proposes the narrowest exception: a typed address alone, one request in the discovery role, by a fetcher with no channel and no cookie, read by the link-pointer reader alone, nothing of the address kept as a channel or in history. Until it is confirmed, discovery is not built.

## 4. Findings applied to the design

1. **A fetcher of its own for each discovery, with no channel and no cookie** (sections 3.1, 3.7): not the tab's channel fetcher. Applied in `discovery.md` section 4.1.
2. **The typed-only rule is the trusted core's**, from the strip input's own kind, never from anything a fetcher or the interface says (section 3.6). Applied in section 3.
3. **Results go in the strip, never the page area, and the panel never takes the focus by itself** (section 3.4). Applied in section 4.5.
4. **The trusted core reads every target again**, refuses `http` from `https`, and shows nothing it refused (section 3.3). Applied in section 4.4.
5. **One message, from the fetcher started for it, bounded and fuzzed**; a second ends the fetcher (section 3.9). Applied in sections 4.3 and 4.4.
6. **A choice is by number, navigated to the trusted core's own copy** (section 3.10). Applied in section 4.5.
7. **The request needs the owner's exception to DEC-1722** (section 3.12): DEC-2374, waiting for the owner.
8. **The reader is the engine's**, and does not exist yet (the engine's TODO-2376): the browser writes none of its own (requirement 2.1).

## 5. Changes to `security-analysis.md`

When discovery is built, section 5.1 gains the discovery fetcher's request and section 5.2 the `Discovered` message, with the answers of sections 3.1 and 3.9 here. Nothing changes there while discovery is not built.

## 6. Review

Written on 3 November 2026, before any of discovery is built. To be reviewed again, and section 5 applied, when it is built.
