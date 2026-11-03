# NeoTeletext: discovery, from a web address to a channel

| | |
|---|---|
| **Status** | Designed 3 November 2026 (TODO-2073); not built. Its security analysis, [`discovery-security.md`](discovery-security.md), was written first and its findings are applied here. Two things stand before the build: the owner's answer on DEC-2374, because DEC-1722 item 1 leaves discovery no address to run on, and the engine's link-pointer reader, which does not exist (the engine's TODO-2376). |
| **Requirements** | REQ-693 (requirement 5.5, discovery reads only link pointers) and REQ-694 (requirement 5.6, results are shown, and a chosen one is an ordinary navigation). |
| **Decisions** | Decision 14 (DEC-913) keeps discovery; DEC-1664 withdraws the site-change screen REQ-694 first named; DEC-1722 makes every `http` and `https` address the browser opens an address of the URL map (REQ-1693); DEC-2374 proposes how discovery and DEC-1722 meet. |
| **Builds on** | The engine's user agent and its readers (requirement 2.1); the address rules in `native/core/src/address.rs`; the strip as the browser's trusted area (`architecture.md` section 6.1); the confinement of the fetcher (`sandboxing.md`). |

## 1. What discovery is for

A publisher with a web site and a teletext channel can point from one to the other. A viewer who types the web site's address, `example.org`, should be able to reach the channel without knowing its address. On the web the usual way is a pointer in the page's head or headers, as a feed is announced:

```
Link: <https://example.org/teletext/>; rel="alternate"; type="application/vnd.neoteletext+json"
<link rel="alternate" type="application/vnd.neoteletext+json" href="https://example.org/teletext/">
```

Discovery reads those pointers and nothing else of the web page, shows the viewer what it found, and lets the viewer choose. It is row 5 of the order in which a reader opens an address, as the protocol's DOC-2036 gives it: "any other address that answers 200 with a web page".

## 2. What it is not

- It is not a web engine. Nothing of the page is shown, run or followed but the pointers (requirement 1.3).
- It never moves a tab by itself (REQ-694). A result is shown with its whole address; only the viewer's choice opens it.
- It never runs for a link, a bookmark, a redirect, a page's target, a descriptor or a file. Only an address the viewer typed in the address field can start it (REQ-693).
- It never runs twice in a row: a result is a channel's address, which opens as a channel or fails as one, and never starts discovery again.

## 3. When it runs

1. The viewer types an `http` or `https` address in the address field, or a bare host that the field reads as one.
2. The address rules (`address::parse_typed`) refuse it as not being a channel's address, after the field's own repair of a missing final slash (DEC-1722 item 2).
3. Instead of only refusing it, the trusted core starts discovery for it, in the tab in front, and says so in the strip ("Looking for a channel at example.org").

Today step 3 cannot happen: DEC-1722 item 1, confirmed by the owner on 1 November 2026, says such an address is refused "with a sentence for the viewer, and no tab opens and no fetcher starts". DEC-2374 proposes the narrow exception discovery needs: a typed address alone, one request, read by the discovery role alone, with nothing of the address kept as a channel. Until the owner answers, nothing of discovery is built, because a trigger is the first thing every other part depends on.

## 4. How it runs

### 4.1 The request

The fetcher makes one `GET` of the typed address with the engine's user agent, in the resource role "discovery" (`architecture.md` section 2, resource role), which the engine reads with its discovery reader alone:

- `Accept: text/html` and the engine's fixed `User-Agent`; no `Cookie`, no `Referer`, and no `Set-Cookie` kept;
- redirects followed by the engine's rules, at most five, each checked for the local network (requirement 5.10) and for a fall from `https` to `http`;
- the response's headers within the engine's header limit, and at most the first 8,192 bytes of its body (REQ-693), after which the connection is closed and not reused;
- every other status, and every media type but `text/html`, ends discovery with "No channel found at example.org".

The fetcher is a fresh one, started for this request with no channel, as a tab's fetcher is started for a channel, confined the same way, and ended when the request ends. It holds no cookie, so a hostile page cannot reach a channel's session through it.

### 4.2 The reader

The engine's link-pointer reader (the engine's TODO-2376, which this browser waits for) reads, in that order:

1. each `Link` header field, by RFC 8288's grammar, keeping a link whose `rel` holds `alternate` and whose `type` is the channel's media type;
2. each `<link>` element in the first 8,192 bytes, until `<body` or `</head`, skipping comments and the content of `script`, `style`, `noscript`, `template`, `textarea`, `title` and `xmp`, with HTML's attribute quoting, the first of two attributes of the same name, and no character references decoded: a target holding `&` is refused.

It is bounded (at most 16 results from headers and 16 from elements, each target at most the engine's `MAX_URL_LENGTH`), allocates nothing it has not bounded, panics on nothing, and is a fuzz target. Its limits are engine constants, used here by name (requirement 2.4).

### 4.3 What reaches the trusted core

The fetcher sends one typed message, `FromFetcher::Discovered`, holding the address it asked for, the final address after redirects, and the targets the reader returned, each resolved against the final address, each at most `MAX_ADDRESS` bytes, at most 32 in all. Nothing else of the response reaches the trusted core. The message is in the fetcher's protocol module of its own, with its kind, its reader and writer, its bounds checked on both sides, and a fuzz target, so that it does not touch the fetching code.

### 4.4 What the trusted core does

1. It accepts the message only from the discovery fetcher it started for a typed address of the tab in front, and only once; any other is a fault of that fetcher, which is ended.
2. It reads each target again with the address rules (`address::parse`), the same as a link's (requirement 11.3), and keeps only channel addresses: an address of the URL map with no page file, `https` or `http`, and never `http` when the final address was `https`. A target the rules refuse is dropped and not shown, and the channel's Log says how many were dropped.
3. It drops duplicates, keeps at most eight, in the order found, and sends them to the interface for the strip as a cleaned list (requirement 11.5).
4. It moves no tab, opens no fetcher for a result and keeps nothing of discovery when the tab navigates, closes or the viewer types again.

### 4.5 What the viewer sees and does

The strip, the browser's trusted area outside the page, shows a panel under the address field: "Channels found at example.org", then each result as its whole address, its host emphasised as the address field shows it, with Enter, a click or a number key choosing one. Escape or a click elsewhere closes it. With no result: "No channel found at example.org". The panel never takes the keyboard focus by itself and accepts Enter only once the viewer has moved the focus into it, as the web prompt does (REQ-811), so that a result arriving as the viewer presses Enter for something else opens nothing.

Choosing a result navigates the tab at once, as a page-initiated navigation (REQ-725, REQ-1670), with Back returning to where the tab was. The tab's history records the channel, not the web address.

## 5. Order of the build

1. The owner's answer on DEC-2374, and the requirements it settles revised to match (REQ-693, REQ-694, REQ-1898 item 2).
2. The engine's reader, with its fuzz target, and the pin moved here (`DEVELOPMENT.md` section 11).
3. `FromFetcher::Discovered` and the discovery fetcher, in a module of their own.
4. The trusted core's part, section 4.4.
5. The strip's panel, section 4.5.
6. The tests of section 6, each failing on the code before.

## 6. Tests

- Hostile markup: comments, raw-text elements, unterminated quotes, a `>` inside a quoted value, duplicate attributes, UTF-16 and zero bytes, megabytes of padding: only well-formed pointers in the head are found (the engine's tests and fuzz target, and one end-to-end here).
- A pointer past byte 8,192 is not seen; a body longer than that closes the connection.
- A target the address rules refuse (`file:`, `javascript:`, a local address, a page file, `http` from `https`) is not shown.
- Receiving results moves no tab.
- Choosing a result navigates at once, recorded as started by the page, and Back returns.
- An address that did not come from the address field (a link, a bookmark, a redirect, a page's target) starts no discovery.
- A second `Discovered` message, or one from a channel's fetcher, ends that fetcher and shows nothing.
- No `Cookie` is sent and no `Set-Cookie` is kept.

## 7. What building changed

Nothing yet: not built.
