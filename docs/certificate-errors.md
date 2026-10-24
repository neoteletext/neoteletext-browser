# NeoTeletext: the certificate-error screen

| | |
|---|---|
| **Asked** | By the owner on 23 October 2026: "do Certificate-error screen ensure specs and reqs". |
| **Decision** | 83 (`architecture.md` section 2). |
| **Meets** | Requirement 7.13, new; requirement 7.11 (a failed certificate is a hard stop, decision 24) is unchanged. |
| **Security analysis** | `certificate-errors-security.md`. |

## 1. What was wrong

A channel whose certificate failed was refused, as decision 24 requires: the fetcher's connection ended in the handshake and nothing offered a way past it. But nothing said so. The tab stayed empty, the strip looked as it does while a channel loads, and the reason reached only the console's Log, as an engine event's debug text. A person could not tell a refused channel from a slow one, nor what to do about it.

## 2. What the screen is

When a connection of a tab's channel is refused because of its certificate, the tab's page area shows the **certificate-error screen** in place of the channel's pages, and the strip marks the channel refused.

- **The heading:** "This channel's certificate was refused".
- **The host,** as the strip shows it: the trusted core's own copy of the channel's address, never a name from the certificate, which the server chose.
- **What failed,** in one sentence chosen by the trusted core from the kind of failure (section 3).
- **What it means:** "NeoTeletext does not open a channel whose certificate fails. There is no way past this screen."
- **What may help,** for the failures a person can mend: an expired or not-yet-valid certificate can be the computer's clock, so "If this computer's date or time is wrong, correct it and try again."
- **Three buttons:** **Try again** (Reload: the channel opened afresh, requirement 11.6), **Go back** (shown when the tab has history to go back to) and **Close tab**. There is no button, key or setting that opens the channel anyway (decision 24).

The strip's security mark, the padlock or "Not secure" beside the host, reads **"Certificate refused"** in the strip's warning colour while the screen shows.

The screen is drawn by the interface process from the strip's state, which the trusted core sends; no page image is drawn over it or beside it in the page area, and nothing the channel sent appears in it.

## 3. The kinds of failure

The fetcher learns the kind from its TLS library's error (`rustls`, decision 41) and reports only the kind. The trusted core words it.

| Kind | From `rustls` | What the screen says |
|---|---|---|
| Expired | `CertificateError::Expired`, `ExpiredContext` | Its certificate has expired. |
| Not yet valid | `NotValidYet`, `NotValidYetContext` | Its certificate is not valid yet. |
| Unknown issuer | `UnknownIssuer` | Its certificate was not issued by an authority this browser trusts; a certificate the server made for itself is refused so. |
| Another host | `NotValidForName`, `NotValidForNameContext` | Its certificate is for another site. |
| Revoked | `Revoked` | Its certificate has been withdrawn by the authority that issued it. |
| Invalid | any other `InvalidCertificate` | Its certificate is not valid. |
| Handshake | any other error of the secure handshake | The secure connection to it could not be made. |

A connection that fails before the handshake, or a proxy that refuses (requirement 19.10), is not a certificate failure and does not show the screen. A handshake that runs out of the connection's one deadline (finding F-63) is reported as Handshake, as a broken one is; telling the two apart is not built.

**What is not checked** (finding F-95): the roots are Mozilla's, compiled in (`webpki-roots`); the system's own store is not read, so a root the user or their organisation added is not trusted, and nothing asks whether a certificate was revoked (no revocation list, no OCSP, no stapled answer). `Revoked` is kept for the day revocation is checked; as built it cannot occur. A certificate revoked but not expired is accepted.

## 4. When it goes

The screen goes when the channel's contents next arrive (a connection succeeded), when the tab goes to another address, or when the tab closes. **Try again** opens the channel afresh; if the certificate fails again, the screen shows again.

Every connection of the channel counts, its fetch and its push alike, since both are to the channel's own host. Repeated refusals while the agent retries change nothing on screen.

## 5. Messages

- **From the fetcher:** `FromFetcher::CertificateRefused(CertificateProblem)`, one byte; the fetcher names no host, since it serves one channel and the trusted core knows which.
- **To the interface:** the strip's state gains `refused: Option<CertificateProblem>`, one byte after its notice. The interface draws the screen and the strip's mark from it, and the button presses are the strip input that already exists (Reload, Back, Close tab).
- **To the console:** the Log gets one warning line, in the trusted core's words, each time the screen first shows.

## 6. Requirements

Requirement 7.13.

## 7. What building settled

Built on 23 October 2026.

1. **The kind comes from the fetcher's transport,** which sees the TLS library's error before the engine's agent turns every failure into one; the transport keeps the latest refusal and the fetcher reports it, as it reports a proxy's failure.
2. **The host is never taken from the certificate.** A certificate's names are the server's to choose; the screen names the host the user asked for.
3. **The refusal belongs to the channel, not the tab:** it is kept with the channel's state in the trusted core and sent with the strip of whichever window shows it.
