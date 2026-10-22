# Pending tasks for Andrew

What only Andrew can do: accounts, money, keys, people, machines, and checks that need a hand on a real window or phone. Each item says what it unblocks. Remove an item here when it is done.

## Accounts and certificates (unblock signed releases, `docs/installation-and-updates.md` sections 4 and 10)

1. **Join the Apple Developer Program** (paid yearly), then create a **Developer ID Application** certificate on this Mac. It signs and notarises the macOS bundle. Without it macOS 15 will not open the browser for an ordinary user. Checked 22 October 2026: this Mac has no code-signing identity.
2. **Get a Windows code-signing certificate** whose key sits in a hardware module (the CA/Browser Forum rule since June 2023): a hardware token from a certificate authority, or a cloud signing service such as Microsoft's. It signs every Windows executable and the installer.
3. **Choose the update host** for `update.neoteletext.com` (decisions 17 and 25). It must serve static files over TLS and host nothing else, and it need not be trusted.

## Keys and people (unblock The Update Framework repository, section 9)

4. **Buy five hardware security keys**:
   - three for the root role;
   - one for the targets role;
   - one for the Linux release key.
5. **Name the three people who each hold one root key** (requirement 13.9), one of them keeping theirs in a safe. Two of the three are needed to sign a new root.
6. **Hold the first key ceremony.** It creates the root, targets and Linux release keys, signs the first root, and is recorded in `docs/releases.md`.

## Machines

7. **Pick the machine that re-signs the timestamp daily** (section 9.3). It holds the timestamp key only, and if it misses seven days, every user sees "Updates unavailable".

## Checks that need a hand

8. **Real phones against qrkeyboard.com** (protocol 3), as `DEPLOY.md` step 5 of qrkeyboard asks.
9. **The settings panel by hand**: press each control and listen to the volume. Posted clicks do not reach the window.
10. **The phone keyboard through a proxy**: its relay reached through the proxy setting.
11. **Typing into a channel and paste on macOS under the sandboxes**: the text-input services and the pasteboard, which the profile allows but no test drove.

12. **Listen on Windows and Linux with real speakers.** The sound helper played confined on both (`docs/sandboxing.md` section 7.5), but into the virtual machine's device and the Linux host's null sink: nobody heard it.

## Decisions

13. **Confirm decisions 52 and 53** (`docs/architecture.md` section 2): every helper sandboxed on every platform, and the efficiency work. Both are built and marked "owner to confirm".
14. **Confirm decision 57**, history and bookmarks, built while you were away: the 17 departures are in `docs/history-and-bookmarks.md` section 14.
15. **Decision 58 on Linux:** a default Ubuntu from 24.04 forbids the browser's network namespace unless an AppArmor profile allows it, which needs root, so the browser now refuses to run there; a per-user install (decision 54) cannot add one. Choose: a system-wide package that installs the profile, or a stated requirement (`docs/sandboxing.md` section 8.3).
16. **Decisions 10 and 44 under decision 58:** plain `http` channels and plain `ws://` push are allowed, marked "Not secure". They are not downgrades (nothing falls back to them), but they are weaker modes a user can choose. Say whether they stand.

## Housekeeping

17. **The prototypes backup**, `~/devel/neoteletext-browser.prototypes.bak-2026-10-21-141115` (33 GB): delete it when you no longer want it.
18. **teletext-rag** (branch `redesign`) has no remote: say where it should be pushed.
19. **Push** the browser, the engine's `implementation` branch and `neoteletext-common` when you want them on GitHub.
