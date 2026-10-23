# NeoTeletext: the certificate-error screen, security analysis

The design is `certificate-errors.md` (decision 83, requirement 7.13). This analysis follows `security-analysis.md`'s form.

## 1. What is new

- One message kind from the fetcher to the trusted core: `CertificateRefused`, one byte naming a kind of failure.
- One byte more in the strip's state, from the trusted core to the interface.
- A screen the interface draws in a tab's page area, with three buttons that send strip input that already exists.

Nothing new can open a channel, and nothing proceeds past a refused certificate: decision 24's hard stop is unchanged.

## 2. Threats

| Who | Can | Cannot |
|---|---|---|
| A server, or a network attacker presenting a certificate | Make the fetcher refuse the connection, as before, and so show the screen for that channel | Put any text of its own on the screen: the host is the trusted core's copy of the address the user asked for, and the sentence is chosen from a fixed list by kind; a certificate's names and fields are never shown |
| A compromised fetcher | Claim a refusal for its own channel, which shows the screen there; it could already stop that channel loading | Name another channel or host; claim a refusal for another tab; make a channel open: the message carries one byte, read strictly and refused if unknown |
| A compromised interface | Draw the screen, or hide it, as it can draw anything in the window (requirement 3.9's exception) | Proceed past a refusal: there is no message that does; the buttons are Reload, Back and Close tab |
| A page | Nothing: the screen is drawn in the page area only while no page image is, and a page cannot draw the strip's mark | — |

## 3. Bounds and parsing

`CertificateRefused` is one byte, one of seven values; anything else, or any other length, is a broken message and the fetcher is stopped (requirement 4.3). The strip's new byte is bounded the same way on the interface's side. Both readers are covered by the fuzz targets `fetcher_messages` and `core_messages`.

## 4. Remaining risk

Low. The screen adds explanation, not capability: it shows a refusal the browser already made and offers only what the strip already offers.
