# NeoTeletext: licensing

NeoTeletext follows the teletext engine's licensing rule, set out in `LICENSING.md` of the neoteletext repository: using third-party material must never compromise the MIT licence. Anyone who takes our code must be able to use, change and ship it, open or closed, under the MIT licence alone. This document applies that rule to the browser and records what the browser uses.

This is the project's reading of the licences, not legal advice.

## 1. The browser's own licence

**MIT,** as the engine, decided by the owner on 20 October 2026 (decision 41 of `architecture.md`): the `LICENSE` file at the repository's root, and the `license` field of the workspace's crates.

## 2. What is acceptable

| Kind | Licences | Why our licence is not compromised | Conditions |
|---|---|---|---|
| **Code compiled into the browser** | MIT, Apache-2.0 (also with the LLVM exception), BSD-2-Clause, BSD-3-Clause, Zlib, ISC, CC0-1.0, and Unicode-3.0 for ICU4X's text data (`native/deny.toml`) | Permissive: they ask for a notice and nothing more | Keep their notices in any binary distribution |
| **A choice of licences** | For example `byteorder` (Unlicense or MIT), `r-efi` (MIT, Apache-2.0 or LGPL-2.1-or-later) | We take it under a permissive choice | None beyond that choice's own |
| **Fonts shipped with the browser** | SIL Open Font License 1.1 (Roboto, Open Sans, in `native/draw-vello/fonts/` with their licence texts) | The OFL binds the font, not the program that draws it | Ship the OFL text with each font; never sell a font on its own |
| **System libraries loaded at run time and not shipped** | LGPL-2.1-or-later (glibc, ALSA's `libasound`), GPL with the GCC Runtime Library Exception (`libgcc_s`), and each operating system's own libraries | Linked dynamically from the user's own system, so the user can always replace them, which is all the LGPL asks | Never link them statically and never bundle them (section 3) |

The owner accepted the system's LGPL `libasound` for sound on Linux on 19 October 2026 (decision 40).

`landlock` (MIT or Apache-2.0), `seccompiler` (Apache-2.0 or BSD-3-Clause) and `libc` (MIT or Apache-2.0), approved on 22 October 2026 for the Linux sandboxes (decision 52), are Linux-only dependencies of `confine`.

`keyring` 4.2.0 (MIT or Apache-2.0), chosen on 22 October 2026 while the owner was away for the cookie file's key (decision 62, `cookies.md` section 13 item 7, to confirm), brings the Keychain store on macOS, the Credential Manager store on Windows, and on Linux the Secret Service over `zbus`, all MIT or Apache-2.0; `ring`, already in the tree under rustls, gives the cipher. The broker alone calls them.

`arboard`, approved on 21 October 2026 for the clipboard (decision 49), uses two crates on Windows only under BSL-1.0, the Boost Software License: `clipboard-win` and `error-code`. It is permissive, OSI-approved and FSF-free, and asks less than MIT: its notice need not be carried in a binary distribution. `native/deny.toml` allows it for those two crates alone.

## 3. What is not acceptable

- **GPL and AGPL code.** Linking it into the browser would put the whole browser under the GPL. CREDITS.md names one GPL project, fullPage.js, for an idea only; no code was taken.
- **LGPL code compiled in or linked statically,** for example a fully static Linux build that includes `libasound`.
- **LGPL libraries bundled with a release** unless the release also carries the LGPL text and offers that library's source. A `.deb` or `.rpm` lists `libasound2` as a dependency instead. An AppImage or Flatpak must leave `libasound` out of the bundle or meet both conditions.
- **File-level copyleft code compiled in** (MPL-2.0, EPL-2.0) without the owner's decision.
- **Licences that are not open source** (Business Source License, Server Side Public License, Commons Clause, non-commercial terms), and **code with no licence.**

## 4. How it is enforced

- `cargo-deny` checks every crate against `native/deny.toml` on every full gate (`scripts/check.sh full`). The allow list is the engine's plus Unicode-3.0.
- Every library is a library choice that needs the owner's approval (requirement 14.3), whatever its licence.
- System libraries are outside cargo-deny's view. A new one is recorded in section 5 before it is used.

## 5. What is in use

Measured on 20 October 2026 from `cargo metadata --all-features` and from the built programs.

- **Crates:** about 300, all under the licences of section 2, and on Windows `clipboard-win` and `error-code` under BSL-1.0 (section 2). Most are MIT or Apache-2.0. Besides those are the 22 ICU4X crates (Unicode-3.0), which `parley` and `winit` use for text, and `tiny-skia` (BSD-3-Clause). The engine's crates are MIT.
- **The installer and updater's crates** (23 October 2026, decision 69 of that work; `cargo deny` in both workspaces): in the browser's workspace `miniz_oxide` (MIT, Zlib or Apache-2.0) and `rustix` (Apache-2.0 WITH LLVM-exception, Apache-2.0 or MIT), named directly; in the updater's own workspace, `native/tuf`, `tough` (MIT or Apache-2.0) and what it brings, among them `tokio` (MIT), `serde_json` (MIT or Apache-2.0), `jiff` (MIT or Unlicense, taken under MIT), `aws-lc-rs` (ISC and Apache-2.0 or ISC) and `aws-lc-sys`, the AWS-LC library built from source (ISC, Apache-2.0, MIT and BSD-3-Clause, every part permitted); and `rcgen` (MIT or Apache-2.0), in the release tool only, for test certificates.
- **Fonts:** Roboto and Open Sans (SIL OFL 1.1).
- **Linux system libraries,** linked dynamically and not shipped:
  - glibc (`libc`, `libm`): LGPL-2.1-or-later
  - `libasound`, for sound, linked by the sound helper alone (decision 55): LGPL-2.1-or-later
  - `libgcc_s`: GPL-3.0 with the GCC Runtime Library Exception
  - `libfontconfig`, through `fontique`, to find system fonts, loaded at run time when present (`fontconfig-dlopen`): MIT-style
  - `libfreetype`: the FreeType License, which it offers as an alternative to GPL-2.0
  - `libexpat`: MIT
  - `libpng`: the libpng licence
  - `libz`: Zlib
  - `libbz2`: BSD-style
  - `libbrotli`: MIT

  X11 and Wayland libraries (MIT) are opened at run time by `winit`.
- **Windows:** Windows's own libraries and Microsoft's Visual C++ runtime (`VCRUNTIME140.dll`), whose licence allows any program to use it and to redistribute it.
- **macOS:** the system frameworks and libraries (AppKit, Core Audio, `libSystem`, `libiconv` and others), part of macOS and never shipped.
