# NeoTeletext: security analysis of cookies

| | |
|---|---|
| **Design** | [`cookies.md`](cookies.md), proposed and built 22 October 2026 (decision 62); what building settled is its section 15. |
| **Rule** | Decision 58: no weakening or downgrading of security, and no fallback to a weaker mode. |
| **Builds on** | [`security-analysis.md`](security-analysis.md) sections 5 and 12; the engine's REQ-WEB-006, REQ-WEB-026 and REQ-SEC-013. |

## 1. What is protected

- **Cookies as credentials:** a session cookie is a signed-in user; anyone who holds it is that user to the channel.
- **Cookies as identifiers:** a long-lived cookie lets a channel recognise the user across visits.
- **The user's control:** what is kept, for how long, and the user's ability to remove it.

## 2. Attackers

A channel's server, hostile or compromised; another channel, on the same origin or another; someone on the network path of a plain-HTTP channel; a fetcher compromised through its channel's responses; the interface compromised through a fault in its own code; another program running as the user; someone with the disk or a backup later.

## 3. Boundaries and answers

| Boundary | Threat | Answer |
|---|---|---|
| Channel → its own jar | A server sets cookies to fill memory, track, or overwrite another channel's | The engine's rules: sizes, 400 days, prefixes, host-only, channel-only (REQ-WEB-006, REQ-WEB-026); 180 a channel and 3 000 in all, least recently used first. A channel can reach only its own jar. |
| Channel → another channel on the origin | A neighbour on a shared host sets `Path=/` to read or replace the other's cookies, or sends `Clear-Site-Data` to sign the other out | Each channel's jar is its own, a `Path` outside the folder is narrowed to it (REQ-WEB-026), and `Clear-Site-Data` empties only the sender's jar (`cookies.md` section 8). |
| Network → plain-HTTP channel | An attacker on the path reads or sets cookies of a plain-HTTP channel, or replaces a `Secure` one | Plain HTTP is allowed (the engine's decision E.32), so its own cookies are as exposed as its pages; `Secure` cookies are never sent over it or replaced from it, and the `http` and `https` channels of a host are separate jars, so an `http` channel cannot touch the `https` one's cookies at all. `Clear-Site-Data` is ignored over plain HTTP. |
| Fetcher → trusted core | A compromised fetcher asks for other channels' cookies, or feeds hostile `Set-Cookie` lines | It asks only for its own channel's requests, which the trusted core checks against the channel it started the fetcher for; the trusted core answers only that request's header (requirement 21.1). Its `Set-Cookie` lines are bounded and parsed by the engine's strict parser in the trusted core, as if the server had sent them, and fuzzed (`cookie_messages`). It sees the cookies of the requests it makes, which a fetcher must. |
| Trusted core → fetcher | The trusted core's answers leak cookies of another channel, or Strict cookies to a cross-channel navigation | `header_for` with the request's kind (REQ-WEB-006's sending rules); tested per kind. |
| Interface → trusted core | A compromised interface reads cookies through the panel, deletes them, blocks channels, or turns persistence on | The panel's views carry no name or value (requirement 21.6). Deleting and blocking by view numbers are strip input, as today (decision 33's accepted exposure: a compromised interface can already navigate and clear history). It cannot read a cookie by any message. |
| Trusted core → interface | A cookie leaks into a view, a console entry or a notice | No message to the interface carries a name or value (requirement 3.6); a review and a test over every view kind. |
| The disk | A backup, another user, or a stolen disk reads `cookies.bin` | Session cookies are never written; the rest only sealed under a key in the platform's secret store, mode 0600 (requirement 21.4). Without the key nothing is written: no clear-text fallback (decision 58). |
| Another program as the user | Malware reads the key from the secret store as the browser does, then the file | Not prevented, as in every browser; Chrome's App-Bound Encryption on Windows needs an elevated service and is reported bypassed. What it can steal is the persistent cookies, never the session cookies, which are not on the disk. Device Bound Session Credentials (the engine's REQ-WEB-017) are the remedy that matters, when built. |
| File → trusted core | A replaced `cookies.bin`, sealed with the stolen key, holds hostile entries | Read strictly, each cookie passed through the engine's rules again as if newly set, bounded, fuzzed (`cookie_file`, requirement 21.5). It can plant cookies, as such a program already could by other means. |
| Broker → trusted core | The key | One request, answered with the key or a refusal; the key stays in the trusted core and is wiped at exit. The broker is unconfined already and holds nothing more by this. |
| Private tabs | A private session leaves cookies | Never written, separate jars, emptied with the last private tab (requirement 21.3). |

## 4. Risks that remain, ranked

1. **Malware running as the user steals persistent cookies** (medium likelihood on an infected machine; high consequence for accounts that use them). Shared with every browser; limited here by never writing session cookies. Remedy: Device Bound Session Credentials.
2. **A compromised interface clears or blocks cookies** (low; low): annoyance, not theft. The same root as decision 33.
3. **Plain-HTTP channels' cookies on the network** (depends on the channel; medium): the engine's decision E.32 allows plain HTTP; the padlock and "Not secure" say so.
4. **A secret store the browser cannot reach** (likely on a minimal Linux; low consequence): cookies last for the run, and the panel says why.

## 5. What changes in `security-analysis.md` when this is built

- **Section 2:** cookies kept between runs, encrypted.
- **Section 5.4, the fetcher:** holds no cookie; asks per request.
- **Section 12, privacy:** what is kept, encrypted, and the user's controls.
- **Section 16:** risk 1 above.

## 6. Checks that keep these properties

The tests of requirements 21.1 to 21.11, and in particular: the fetcher without a jar; no name or value in any interface message; nothing written in the clear when the key is refused; no session or private cookie in the file; `Clear-Site-Data` over `https` from the channel alone; the three fuzz targets in the full gate.
