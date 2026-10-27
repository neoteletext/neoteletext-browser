# Driving the browser from a test harness on macOS

How the NeoTeletext browser was launched, found, brought to the front, given keys and clicks, captured and closed by a program, on 26 October 2026, to test the kitchensink channel on foundintelquiet4060. It records what works, what was tried and failed, and why, so the next harness starts from here.

Measured on macOS 15 (Darwin 24.6), Apple Silicon, with the release build in `native/target/release/`.

## 1. Why it is not simple

| Obstacle | Cause |
|---|---|
| The Accessibility API sees no window | The window belongs to `neoteletext-interface`, which runs under `sandbox-exec` (decision 52). The sandbox closes the Accessibility API to it: every request answers `kAXErrorAPIDisabled` (-25211), while the same tool reads iTerm2 and Chrome. So AppleScript's System Events, Accessibility-based tools such as browser-use's macos-harness, and `AXRaise` cannot reach it. |
| One process cannot activate another | Since macOS 14, activation is cooperative. `NSRunningApplication.activate` returns true, but the window does not become the key window. |
| Keys reach a channel only while the window has the focus | INPUT.md section 9.1: the tab must be active, the page the channel's, and the window focused. Keys posted to the process while it is in the background reach the browser (typed digits choose pages), but never the channel. |
| The page list keeps the keys it is given | After typing a page number the focus is on the page list, which keeps the arrows and Page keys until something else is clicked (decision 105). |
| A keychain prompt at every start | The cookie file's key is in the login Keychain (decision 62). A rebuilt program is signed afresh, and the Keychain asks again ("neoteletext wants to use your confidential information stored in "NeoTeletext cookie file" in your keychain"). The prompt, SecurityAgent, takes the front. |
| "Frontmost" can be stale | `NSWorkspace.frontmostApplication` is updated by notifications a command-line tool's idle run loop never processes. It reported SecurityAgent while iTerm2 was in front. |
| A click can land on another window | Other apps' windows may cover the browser's. A synthetic click goes to whatever is on top at that point, and a capture by window ID still shows the browser's window, hiding the problem. |
| Local addresses are refused | The fetcher reaches this computer or the local network only when named with `--allow-local`, and it takes an IP address, not an address and port (`192.168.0.166:8098` made each fetcher stop and restart). |

## 2. What works

### 2.1 Launch

```
neoteletext --no-keychain --allow-local 192.168.0.166 http://192.168.0.166:8098/kitchensink/
```

- **`--no-keychain`** (decision 113) keeps cookies for the run only. The trusted core never asks the broker for the cookie key, so no keychain prompt appears; the cookie file is neither read nor written.
- **`--allow-local`** takes the server's IP address.
- **Record the processes.** The harness starts the launcher with `posix_spawn` and records every process under it: the trusted core, the interface, the sound helper, and a fetcher and a decoder for each tab, ten in all. The list is kept in a file, so they can be ended by PID even if the launcher goes first.

### 2.2 Find the window

`CGWindowListCopyWindowInfo`, filtered to layer 0, gives each window's owner PID, window ID and frame. The browser's window is the largest one whose owner is the recorded `neoteletext-interface` process. No Accessibility is needed.

### 2.3 Bring it to the front with the focus

Two steps, then a check:

1. **Raise:** `GetProcessForPID` and `SetFrontProcessWithOptions(…, kSetFrontProcessFrontWindowOnly)`, the deprecated Carbon calls. They put the browser's window on top of every other window, but leave the browser inactive. Swift marks them unavailable, so the harness is Objective-C, with the deprecation warning suppressed at that one call and a comment saying why.
2. **Activate:** one real left click, posted through `CGEventPost(kCGHIDEventTap, …)`, on an empty part of the tab strip, 20 points below the window's top in its right half where there are no tabs. The harness first checks that the browser's own window is the topmost one at that point. A click does what `activate` no longer can, and on the empty strip it does nothing else.
3. **Verify:** the frontmost process, asked of the system with `GetFrontProcess` (it agrees with `lsappinfo front`), must be the interface, and the topmost window at the window's centre must be the browser's. Up to three attempts.

If **SecurityAgent** is in front, the harness stops and says so: a system security prompt is the user's to answer, never a program's.

### 2.4 Keys and clicks

- **Keys:** `CGEventCreateKeyboardEvent`, posted to the system's event tap, only after the focus is verified. Afterwards the harness checks the browser still has the focus. Text is sent a character at a time, with its Unicode string set on the event. The coloured keys are F1 to F4, which the kit's navigation takes as red, green, yellow and cyan.
- **Clicks:** in points from the window's top-left, refused when another window covers the point.
- **Proof:** the strip reads "Keys go to *host*" while keys go to the channel. The server's `/stats.json` counts `keys`, `gotos` and `own_published` for the channel.

### 2.5 Screenshots

`screencapture -x -o -l WINDOW_ID FILE` captures the window even when it is covered. That is why it can't show whether a click will reach the window.

### 2.6 Close

1. The browser's own Quit, Command-Q, sent while it has the focus, so it saves its session and ends its helpers itself.
2. Whatever is left after 8 seconds is sent `SIGTERM`, then `SIGKILL`, **by PID**. Each PID is checked first to still run one of the browser's programs (`proc_pidpath` under `native/target/release/`), so a reused number is never signalled. The launcher on its own leaves its helpers running when sent `SIGTERM`, a known fault, so every recorded process is signalled.
3. The harness checks that none is left. Never `pkill`.

## 3. The tool: `ntctl`

All of section 2 is one Objective-C program, built with `clang -fobjc-arc -Wall -Wextra -Werror -framework AppKit -framework ApplicationServices ntctl.m -o ntctl`:

| Command | Does |
|---|---|
| `ntctl launch ARGS…` | Starts the browser with ARGS, records its processes, waits up to 30 s for the window; prints the launcher, interface and window |
| `ntctl status` | The frontmost process, the recorded processes, the window and whether it has the focus |
| `ntctl focus` | Section 2.3 |
| `ntctl keys KEY…` | Focuses, then sends each key: `return tab space escape backspace up down left right home end pageup pagedown f1`–`f4`, `cmd+LETTER`, `text:WORDS`, `wait:MILLISECONDS` |
| `ntctl click X Y` | Focuses, then clicks at points from the window's top-left |
| `ntctl shot FILE` | Section 2.5 |
| `ntctl close` | Section 2.6 |

A session with it:

```
ntctl launch --no-keychain --allow-local 192.168.0.166 http://192.168.0.166:8098/kitchensink/
ntctl keys text:301 wait:1500 down down down return    # choose an article, Enter reads it
ntctl shot 813.png
ntctl keys text:601 f4                                  # answer the quiz with cyan
ntctl close
```

It needs the Accessibility permission for posting events, which the terminal it runs in already has. It is in the session's scratchpad (`drive/ntctl.m`), not yet in the repository.

## 4. What was tried and did not work

| Tried | What happened |
|---|---|
| Keys posted to the interface's PID (`CGEvent.postToPid`) with the window in the background | Digits chose pages in the browser, but no key reached the channel: the window had no focus |
| `osascript … set frontmost of process … to true` | No effect: System Events uses Accessibility |
| `NSRunningApplication.activate` | Returned true; the window stayed inactive (cooperative activation) |
| Accessibility (`AXUIElementCreateApplication`, `AXRaise`, `AXFocused`) | `kAXErrorAPIDisabled` for the sandboxed interface |
| A click at a point inside the window's frame | Reached iTerm2, which was in front at that point |
| `NSWorkspace.frontmostApplication` to check focus | Stale in a command-line tool |
| `--allow-local 192.168.0.166:8098` | "not an IP address"; the fetchers restarted in a loop |

## 5. Limits

- **The browser restores its last session** at each launch, so tabs accumulate across runs.
- **The focus can be taken away** between the check and the keys, by another app or by the user. `ntctl keys` checks again after sending and reports a loss, but cannot prevent it. A dedicated login session or a virtual machine would remove that.
- **`SetFrontProcessWithOptions` is deprecated.** It was measured to work on macOS 15; a later macOS may remove it. The click step still works on its own whenever the strip is uncovered.
- **A test hook in the browser** (identify, focus, report focus, quit, over a private channel) would be the most robust route, as both the research and Codex concluded. It would be a change to a security-first browser, so it is the owner's decision and is not built.

## 6. Sources

- [Apple forums: sandboxed apps and the Accessibility API](https://developer.apple.com/forums/thread/756130)
- [Apple forums: Accessibility failing under a sandbox](https://developer.apple.com/forums/thread/687965)
- [Apple forums: bringing another app to the front](https://developer.apple.com/forums/thread/793253)
- [Apple forums: `activateWithOptions` on Sonoma](https://developer.apple.com/forums/thread/739524)
- [Apple forums: the change to `activateWithOptions` since Big Sur](https://developer.apple.com/forums/thread/668913)
- [Apple forums: `NSApplicationActivateAllWindows`](https://developer.apple.com/forums/thread/724143)
- [Apple forums: focusing other apps' windows](https://developer.apple.com/forums/thread/106828)
- [Stack Overflow: bringing another app's window to the front](https://stackoverflow.com/questions/76443344/is-there-a-way-to-bring-a-window-of-another-macos-application-to-the-front-witho)
- [Michael Tsai: activating applications via AppleScript](https://mjtsai.com/blog/2022/05/31/activating-applications-via-applescript/)
- [Chromium review on `SetFrontProcessWithOptions`](https://codereview.chromium.org/1278073002)
- [Mozilla bug 2070908: windows not made key while the app is inactive](https://bugzilla.mozilla.org/show_bug.cgi?id=2070908)
- [Wine: Sonoma's activation changes](https://list.winehq.org/hyperkitty/list/wine-gitlab@list.winehq.org/thread/K4XSASJC5FYH262CAN453KE4ZHNRCKDX/)
- [browser-use/macos-harness](https://github.com/browser-use/macos-harness)
- The Codex CLI (0.163.0), asked read-only, 26 October 2026
