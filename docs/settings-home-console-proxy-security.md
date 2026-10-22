# NeoTeletext: security analysis of settings, the home page, the developer console and proxies

| | |
|---|---|
| **Status** | Written 21 October 2026 with [`settings-home-console-proxy.md`](settings-home-console-proxy.md), whose design takes its findings (decision 51). |
| **Method** | That of [`security-analysis.md`](security-analysis.md): what is protected, who attacks, each boundary crossed, the answers, and the risk that remains, ranked. |

## 1. What is protected

The user's settings and bookmarks; where the browser's connections go; the user's local network; the truth of the strip and the console; the trusted core's readers.

## 2. Attackers

A channel's author; a network attacker; a hostile proxy; a compromised fetcher, decoder or interface; another program of the same user that can write the settings file.

## 3. Boundaries and answers

| Boundary | Threat | Answer |
|---|---|---|
| Settings file → trusted core | A malformed or hostile file: huge, deep, binary, a line breaking a rule | Read by a bounded reader (256 KB, a line of a name and a value of at most 2 048 bytes, a version line), each value checked by its own rule; anything else dropped and logged; fuzzed (`settings_file`). A program that can write the file can already change the user's files; it is not defended against. |
| Trusted core → settings file | A crash leaves half a file | Written to a temporary file and renamed over the old one. |
| Interface → trusted core | A compromised interface asks for a hostile setting: a proxy of the attacker's, every local address allowed | Each change is checked as strip input (requirement 3.9): only values the rules accept are kept. A change of a security setting is said in the strip's notice for ten seconds and written to the log. That the interface draws the strip, and so could ask for a change the user did not make, is open decision 3, unchanged; a hostile proxy still meets end-to-end TLS. |
| Trusted core → interface | The interface learns a security setting | It receives the settings only as the text the panel shows, and acts on none of them (requirement 3.6, amended). No cookie, form value, typed key or pairing secret reaches the console. |
| Bookmarks → the start page | An address breaks out of its line of TTI, or forges a page | The start page is written by the trusted core from addresses already parsed by the address rules and cleaned (requirement 11.5): no control character, so no line can end early; each is cut to a row. Its links are the trusted core's own table, not the page's text. Tested with hostile addresses. |
| Channel → console | Hostile text or bytes in a page, a packet, a notice | Shown as text only, cleaned, cut to a row; packet bytes shown as hexadecimal beside the cleaned text; never interpreted. |
| Cookies → console | The console's Cookies tab (decision 63) puts cookie management where diagnosis is | The tab draws the cookies panel's views unchanged (`cookies-security.md` section 3): origins, channels' folders, counts, sizes and last use, never a name or value, and asks by view number. Moving it into the console adds no message and no field. |
| Fetcher → trusted core | A compromised fetcher floods or forges exchange reports | Each report is bounded (an address of at most 2 048 bytes, a method of 8, numbers in range) and read by a reader that refuses anything else, fuzzed (`fetcher_messages`); at most 1 000 entries are kept a tab. A forged report is only text in the console. A proxy's failure is a message that carries nothing; the trusted core words the strip's notice itself, so a fetcher cannot write the strip. |
| Browser → proxy | The proxy learns the hosts and ports; a plain HTTP channel's content | The user chose the proxy. TLS runs end to end through the tunnel; plain HTTP was already readable on the network (requirement 5.9's "Not secure"). |
| Proxy → browser | A hostile answer: a huge head, no answer, a success with bytes after it | The answer's head is read with the engine's bounded head reader; anything but success fails the connection, and nothing goes direct instead; bytes after the head belong to the tunnel. Fuzzed in the engine (`proxy`). |
| The local network | With a proxy, names are resolved at the proxy, so the local-network rule cannot see where a name leads | Local IP addresses written as addresses, `localhost`, and names under `.local` and `.internal` are refused unless allowed, before the proxy is asked. |

## 4. Risks that remain, ranked

1. **A compromised interface asks for a hostile proxy** (low likelihood, medium consequence): open decision 3. The change is said in the strip and logged; TLS still protects encrypted channels.
2. **A public name that resolves to an address on the proxy's network** (low likelihood, depends on the proxy): that network is the proxy's to protect.
3. **The settings file changed by another program of the user** (low, low): outside the browser's protection, as any of the user's files is.
