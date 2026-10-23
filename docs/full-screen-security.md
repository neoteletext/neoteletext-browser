# NeoTeletext: full screen, security analysis

The design is `full-screen.md` (decision 85). This analysis follows `security-analysis.md`'s form. It is written before building, and is to be checked against what is built.

## 1. What changes

Full screen hides the trusted strip, which requirement 11.4 required to be visible in every layout because "a hidden indicator protects nothing". The page fills the display, so a page image is the only thing seen.

## 2. Threats

| Threat | How | Why it does not succeed |
|---|---|---|
| **A page draws a false strip** | A teletext page can draw, in its 40 by 25 cells, something like an address field or a padlock along its top rows | Nothing in full screen can take the user anywhere outside the channel without full screen ending first: every navigation out of the channel needs the strip's site-change screen, which ends full screen (section 5 of the design), and the real strip is one movement away. A drawn strip cannot be typed into or pressed: a click on a page reaches the trusted core as a cell, never as strip input (requirement 12.3). |
| **A channel that takes input collects typing** | It draws a false address field and hopes the user types an address or a password into it | Keys go to a channel only once its input session is open, and in full screen the trusted badge "Keys go to *host*" shows in the margin, outside the page, whenever they do (design section 3); the strip's own badge rule (`interaction-security.md`) holds in the margin. |
| **A page keeps the user in full screen** | It draws "press Escape" text, or a channel that takes keys captures Escape | The full-screen key (Control-Command-F, F11) is the browser's own and never reaches a channel; the system's own exit always works; the leaving notice is drawn by the interface, in the margin, not over the page. |
| **A page imitates the leaving notice or the typing badge** | Draws them inside the page | The interface draws its notice and badge in the margin of screen colour beside the page, where no page image is ever drawn; a page can draw only inside its own image. |
| **A compromised interface** | Hides the strip or draws a false one | It could already (requirement 3.9's exception); full screen gives it nothing new. |

## 3. Requirement 11.4

The amendment keeps 11.4's purpose: the strip may be hidden only in the page-only full screen, where nothing that leaves the channel can happen without the strip, and it reappears on the pointer at the top edge and with any key the browser keeps. Every other layout still shows it.

## 4. Remaining risk

Low, the same as a television's teletext: what is seen is the channel's page, and leaving the channel needs the real strip.
