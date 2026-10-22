# NeoTeletext

A desktop browser for teletext channels on the web, built on the neoteletext engine (`~/devel/neoteletext-engine`) at a pinned commit. The code is in `native/`. The earlier Tauri design and its prototypes were removed on 21 October 2026 (decision 50) and are kept in git at the tag `prototypes-final`; a path under `prototypes/` in any document is a path there.

**The development process is `~/devel/neoteletext-common/DEVELOPMENT.md`**, shared with the engine: how work proceeds, tracking, requirements, decisions, code rules, dependencies, tests, gates, commits, hosts and safety. This file says only what is particular to this repository; where they differ, that document holds.

## Key documents

- `docs/requirements.md`: every requirement, numbered by section ("Requirement 6.3"), each with Statement, Reason, Source, Verification and a dated State. The only normative text.
- `docs/architecture.md`: the design. Section 2 is the owner's decisions (Decision n, with a "Now" column); section 5 the engine and its pinned commit; section 15 what is built; section 17 the glossary.
- `docs/security-analysis.md`, and one security analysis per feature beside its design: `hyperlinks.md` with `hyperlinks-security.md`, `interaction.md` with `interaction-security.md`, `sandboxing.md` with `sandboxing-security.md`, `history-and-bookmarks.md` with `history-and-bookmarks-security.md`, `cookies.md` with `cookies-security.md`.
- `docs/licensing.md` and `CREDITS.md`.
- `docs/native-interface-design.md`: how the native interface was chosen and what was built when.

## Rules particular to this repository

- **Security first** (decision 4, requirement 1.2): the browser runs as separate processes (trusted core, decoder, fetcher, interface, keyboard), and security never depends on the interface's code (decision 20). A change touching requirement sections 3 to 8, 10, 11 or 13 is reviewed against `security-analysis.md`.
- **The engine does the teletext** (requirement 2.1): recognition, decoding, rendering and web-protocol reading are the engine's; every limit is an engine constant used by name (requirement 2.4). What the engine lacks is added there, by its own process, and the pin moved here (`DEVELOPMENT.md` section 11).
- **No script engine, web engine or interpreter** in the tree (requirement 1.3).
- **Unsafe code** only in `dock-menu` (`src/mac.rs`) and `web-opener` (`src/windows.rs`) (requirement 10.12).
- **No broadcast recordings or sites built from them** are committed; they stay in ignored folders such as `native/target/sites` (requirement 14.6).

## Commands

- Tools: `scripts/tools.sh` installs the pinned tools into the ignored `tools/`; the check script sources it and refuses other versions.
- Edit loop: `scripts/try.sh` (Clippy on every save) or `scripts/try.sh <crate> [filter]`.
- Gates: `scripts/check.sh commit` (pre-commit), `scripts/check.sh full` (pre-push; its Linux stage runs on `NEOTELETEXT_LINUX_HOST`, `foundintelquiet4060` by default).
- The reference channel: `cargo run -p neoteletext-site -- terminal [port] [--relay URL] [--podcast FILE.wav]`.
- A local qrkeyboard relay for tests: `NEOTELETEXT_QRKEYBOARD_RELAY` names one; qrkeyboard.com runs protocol 3 since 21 October 2026.
- Diagnostics on standard error: `NEOTELETEXT_AUDIO_TRACE=1` reports the sound operations and the device's peak level; `NEOTELETEXT_FRAME_TRACE=1` reports every frame drawn and its cost, and every page frame and update that arrives. Each helper reports what confined it at start (`confine`); every layer is required, and a process that cannot have one does not run (decision 58).
- Windows: built and tested by hand in the virtual machine (`DEVELOPMENT.md` section 12.1).
