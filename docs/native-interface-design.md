# NeoTeletext: native interface design

| | |
|---|---|
| **Status** | **Adopted 16 October 2026** (decision 33 of [`architecture.md`](architecture.md)): the owner chose the native architecture built here as the one going forward. Proposed 15 October 2026. This document is now the record of how it was chosen and what was built when; [`architecture.md`](architecture.md), [`requirements.md`](requirements.md) and [`security-analysis.md`](security-analysis.md) describe it as it stands. References below to the earlier documents point to their copies in `prototypes/docs/`. |
| **Purpose** | To specify an interface drawn natively in Rust, with no web engine, that runs on macOS, Windows, Linux, iOS and Android and, with features cut, on microcontrollers; and to review it against `prototypes/docs/architecture.md`, `prototypes/docs/requirements.md`, `prototypes/docs/security-analysis.md` and `prototypes/docs/proofs.md` so that nothing important is lost. |
| **Evidence** | Nine interface prototypes built on 15 October 2026, all under `prototypes/spikes/` (section 2), removed with the prototypes on 21 October 2026 (decision 50) and kept in git at the tag `prototypes-final`, where every `prototypes/` path here is. Measurements are from macOS 15.6 on Apple silicon at a display scale of 1 unless stated. |

**Abbreviations.** None are used. Crate names such as `vello_cpu` are literal identifiers.

## Contents

1. Summary
2. What was evaluated
3. Targets and constraints
4. The design
5. Process model and security
6. How a page reaches the screen
7. Platforms, including microcontrollers
8. Libraries and licences
9. Decisions asked of the owner
10. Risks and what is not yet proven
11. Review against the existing documents
12. Next steps

---

## 1. Summary

NeoTeletext's interface would be drawn by NeoTeletext's own Rust code, with no web engine:

- **Drawing on the processor is the minimum.** Every target must work with no graphics processor. `vello_cpu` draws on the processor; a graphics-processor renderer of the same family is an optional upgrade where it helps.
- **One interface core, several drawing back ends.** The state, layout, animation, hit-testing and input handling are written once. They draw through one small drawing interface, with back ends for desktops and phones (`vello_cpu`, optionally Vello on the graphics processor) and for microcontrollers (`embedded-graphics`).
- **Widgets come from Masonry, or are ours.** Masonry, Linebender's widget layer, provides the text input, buttons, sliders, scrolling, screen-reader support and input methods the shell needs, and in its unreleased source can draw with `vello_cpu`. If it proves unsuitable, the same widgets are built on Taffy (layout), Parley (text and editing) and AccessKit (screen readers).
- **No web engine means most of the web-engine security work is no longer needed.** Decisions 27 to 32, and the two approved but unrecorded decisions on the Linux window host's system call filter and the Wayland security context, exist only to contain a web engine. The process separation of decision 18 for fetching, decoding and the trusted core stays.

The cost: NeoTeletext owns its interface code, including the parts browsers give for free (text editing, accessibility, input methods), and depends on young libraries.

---

## 2. What was evaluated

Every prototype shows the same tab as the React prototype: a header with the thumbnail width slider, its readout and the flat, tilt and fan transform picker, and a wall of teletext pages from the stand-in engine. Sizes are release builds with symbols stripped.

| Prototype | Folder | Size | Needs a graphics processor | Text | Notes |
|---|---|---|---|---|---|
| React in the system webview (Tauri 2) | `prototypes/src`, `prototypes/src-tauri` | 11 megabytes (application bundle) | No | The browser's | The current design; brings decisions 27 to 32 |
| Tauri 3 with the Chromium Embedded Framework | branch `tauri3` | about 334 megabytes | No | The browser's | Caller attribution failed (`prototypes/docs/architecture.md` section 16); no phones |
| Chromium Embedded Framework directly | Linux test machine only | 1.5 gigabytes unstripped | No | The browser's | The bindings' own example; our application not completed |
| Bevy | `prototypes/spikes/bevy-wall` | 54.0 megabytes | Yes | No editable text field | Redrew every frame until set to desktop mode |
| Dioxus native (Blitz) | `prototypes/spikes/dioxus-wall` | 19.7 megabytes | Yes, by default | Real text; no range input; two-dimensional transforms only | Pointer positions only in window coordinates |
| egui | `prototypes/spikes/egui-wall` | 10.3 megabytes | Yes | The best text field of the native options | Scroll bar needed its space kept to stop a redraw loop |
| macroquad | `prototypes/spikes/macroquad-wall` | 1.0 megabyte | Yes | Basic | Its blocking event loop must be told which input wakes a frame |
| tiny-skia, no framework | `prototypes/spikes/skia-wall` | 0.9 megabytes | **No** | Bitmap fonts only | 3 to 4 milliseconds a frame |
| **`vello_cpu` and Parley, no framework** | `prototypes/spikes/vello-wall` | **3.3 megabytes** | **No** | **Shaped Unicode text in system fonts** | **1.9 milliseconds a frame** single-threaded; its multithreading kept a core busy while idle and is off |

Every native prototype idled at no processor use once configured to redraw only on input or change. None was tried on Windows, Linux, iOS or Android, and only the owner has tried their controls.

---

## 3. Targets and constraints

| Constraint | Source | Effect |
|---|---|---|
| Security is paramount | Decision 4 | Every choice below is weighed first for what a compromise gains |
| macOS, Windows and Linux; then iOS and Android | Decision 24, requirements 15.1 and 15.2 | One code base for all five |
| Microcontrollers, with features cut | Owner, 15 October 2026 | Rules out every web engine and every framework tried; only a design with a separate drawing back end reaches a microcontroller |
| Must work with no graphics processor; a graphics processor is a welcome extra | Owner, 15 October 2026 | Rules out Bevy, macroquad, egui and Dioxus native as they stand |
| Totally open licences | Owner, 15 October 2026 | Permissive licences only (section 8); rules out Slint |
| A richly animated interface for many pages at once | Decisions 3 and 7 | Kept as a goal, now met by our own drawing instead of Cascading Style Sheets |

---

## 4. The design

### 4.0 The engine is a separate project

The teletext engine, `neoteletext`, is maintained in its own repository (`~/devel/neoteletext-engine`), not in NeoTeletext's. This design does not move it, copy it or change it:

- **NeoTeletext depends on the engine's crates by a pinned version**, never on a copy of their source. The interface core, the drawing back ends, the interface process and the trusted core are NeoTeletext's own code; decoding, rendering to palette indices, the page store, fetching rules and every limit (decision 26) are the engine's.
- **Changes the engine needs are made in the engine's repository**, by its own process, and recorded in `neoteletext/docs/neoteletext-browser-open-questions.md` (`prototypes/docs/architecture.md` section 5.1). NeoTeletext never patches them locally.
- **The engine's core is in use** (section 12.2): `neoteletext-core`, `neoteletext-formats` and `neoteletext-image`, at commit `f341e94`, in the engine's recommended `modern-classic` profile. The engine's browser shell (`neoteletext-gui`, `neoteletext-browser`) and its application crate are not used. `standin`, the earlier stand-in for the engine, remains only for the earlier prototypes.
- **The microcontroller build** uses the same engine crates that are already `no_std` (section 7), so the engine stays one code base for every target.

### 4.1 Layers

```
 interface core (ours)
   state of tabs and walls · layout · animation · hit-testing · input events
   draws only through the drawing interface below
        │
        ├── widgets: Masonry, or our own on Taffy, Parley and AccessKit (section 4.3)
        │
 drawing interface (ours, small: rectangles, rounded rectangles, gradients,
                    images under an affine transform, text runs)
        │
        ├── vello_cpu + Parley ───────── softbuffer ── winit     every desktop and phone (the minimum)
        ├── Vello on the graphics processor ────────── winit     optional, where it helps
        └── embedded-graphics + bitmap fonts                     microcontrollers, features cut
```

- **The interface core** is the tab and wall logic proven in `prototypes/spikes/vello-wall/src/tab.rs`: layout of the header and wall, the slider's hysteresis and dwell (from the React prototype's `thumbnailSize.ts`), the transitions with the prototype's timing curves (`tween.rs`). Pages are only ever drawn flat: the owner removed the prototype's tilt and fan on 15 October 2026.
- **The drawing interface** is the set of calls the prototypes needed. Pages are images scaled into upright rectangles.
- **Redrawing is on demand:** only on input, a change of state, or while something animates.

### 4.2 Text

- **Parley** lays out and shapes text, with full Unicode and bidirectional support, and finds system fonts through Fontique.
- **Shipped fonts** replace system fonts before release, so that text is the same everywhere and its licence is known (section 8). Teletext's own fonts suit page captions.
- **Untrusted text** (channel names, row 0, link labels) is cleaned before display (requirement 11.5) and shaped in the interface process only (section 5).

### 4.3 Widgets

The shell needs: the address field (requirement 11.3), buttons, the thumbnail slider, scrolling, the tab strip with drag to reorder (requirement 12.1), lists for bookmarks, history and settings (requirement 12.6), the developer tools panel (requirement 12.7), and fields for teletext forms (requirement 7.10).

| Option | What it gives | Against it |
|---|---|---|
| **A. Masonry** (Linebender; Apache-2.0) | `TextInput`, `TextArea`, `Button`, `Checkbox`, `Slider`, `ProgressBar`, `Spinner`, `Label`, `Prose`, `Image`, `Flex`, `Grid`, `ZStack`, `Portal`, `ScrollBar`, `VirtualScroll`, `Split`, and in its source also `Canvas`, `Svg`, `RadioButtons`, `Switch`, `StepInput`, `CollapsePanel`; screen readers through AccessKit; input methods through `ui-events` | Alpha. Drawing with `vello_cpu` exists only in unreleased source (Linebender, 2026 first quarter: Masonry moved to the `imaging` rendering abstraction); the published 0.4.0 uses Vello on the graphics processor with `wgpu` 26. No statement on iOS or Android. Not for microcontrollers. |
| **B. Our own widgets** | Exactly what NeoTeletext needs, all reviewable: Taffy for layout, Parley's `PlainEditor` for the address field, AccessKit for screen readers, `arboard` for the clipboard, input methods from winit (proven in the Linux display host, `prototypes/docs/proofs.md` section 6.16) | Text editing, accessibility and input methods done well are real, permanent work |

**Recommended:** prove option A first (section 12); fall back to option B where it falls short. Either way the microcontroller build uses its own minimal widgets on `embedded-graphics`.

### 4.4 Input

- Pointer, wheel, keys, input methods and dropped files arrive through winit (on phones, touch through winit as well), normalised by `ui-events` where Masonry is used.
- **Cell clicks** (requirement 12.3) are computed from the pointer position and the rectangle the page was drawn in, which the interface core lays out itself.
- **Keys** reach the trusted core's input router unchanged (requirement 12.4).

---

## 5. Process model and security

### 5.1 What changes

With no web engine, nothing in the interface runs code that was not shipped with the application, and nothing in it parses data from the network. The interface receives only checked frames from the trusted core, cleaned and bounded text, and state. The window host of decision 27, the Linux display host of decisions 29 to 32, and the confinement of decision 28 existed to contain a web engine and are no longer needed for that.

### 5.2 Where the interface runs

| Option | Desktop and Android | What a compromise of the interface would gain |
|---|---|---|
| **1. A separate interface process, recommended** | Keeps decision 18's shape: trusted core, fetcher, decoders, font helper, **interface process**, updater, launcher. The interface process draws the window, the wall and the trusted strip, holds no secrets, and can be denied the network and files on every desktop, including macOS, now that there is no WebKit (proof A's limit, `prototypes/docs/proofs.md` section 2, no longer applies). | The ability to draw a false strip and send intents and strip input, which the trusted core checks; no secrets. Reaching it needs a fault in our own Rust interface code or its libraries, fed only by our own frames and cleaned text. |
| 2. The interface inside the trusted core | One process fewer, as `neoteletext`'s egui design had it. | Everything: the drawing and text-shaping libraries would sit beside every secret. |

On iOS and on microcontrollers there is one process in any case, as `prototypes/docs/architecture.md` section 13 already records for iOS.

**The trusted strip under option 1** is drawn by the interface process. That was the position before decision 27 for the strip's integrity against page code, and here there is no page code: the strip can be imitated only by a fault in NeoTeletext's own interface process. Requirement 3.9's hostile interface harness, which replaces the interface process with an attacker's, still holds every other property.

### 5.3 Sandboxing the interface process

- **No network** on every platform (it needs none).
- **No files** beyond its own resources and shipped fonts.
- **Display access** is needed, as for any application. Under X11 any application can send input to others (`prototypes/docs/proofs.md` section 6.8); the interface process is NeoTeletext's own code, not a web engine, so this is the same exposure every desktop application has, not a containment problem.
- **A system call filter** on Linux, and the platform sandboxes of `prototypes/docs/architecture.md` section 6.5, as for the decoder.

---

## 6. How a page reaches the screen

Steps 1 to 3 and 6 of `prototypes/docs/architecture.md` section 10 are unchanged: the decoder renders palette indices; the trusted core checks every frame, keeps flash phases and never resends an unchanged frame.

**Transport to the interface process.** Two options:

- *(Replaced on 22 October 2026 by the second option, runs and changes: `architecture.md` decision 61.)* **Keep the indexed Portable Network Graphics images** of requirement 9.3 on the pipe: about 6 kilobytes a page (proof C), already written by `neoteletext-image`. The interface process decodes them with a pure-Rust decoder, reading only images our own encoder produced. **Recommended.**
- Send checked palette indices with a simple run-length encoding of our own: no image decoder in the interface, more of our own format code.

The interface turns indices and palette into colours once per frame, keeps the image until a newer frame replaces it whole (requirement 9.6), and reports visibility from its own layout (requirement 9.5), which it knows exactly. Scaling and the 4:3 pixel-shape correction (requirement 9.8) become part of the page's drawing transform, with nearest-neighbour sampling.

Decision 24 (9) already allowed "a palette-index canvas" if images limited very large walls; this design is that canvas.

On iOS and microcontrollers, with one process, frames go from decoder to interface as indices with no encoding.

---

## 7. Platforms, including microcontrollers

| Platform | Window and input | Drawing | Notes |
|---|---|---|---|
| macOS, Windows, Linux | winit | `vello_cpu`, shown with softbuffer; Vello on the graphics processor optional | macOS measured; others not yet |
| iOS, Android | winit | as above | Phone screens have up to four times the pixels of this test; frame time to be measured, and the optional graphics-processor back end may matter most here. softbuffer's and winit's phone support to be confirmed in a build. |
| Microcontrollers | the board's own display and input drivers | `embedded-graphics`, bitmap fonts | `neoteletext-core`, `-formats`, `-image`, `-slicer` and `-web` are already `no_std`; `vello_cpu` can also run without the standard library (its `libm` feature) on chips with enough memory |

**What the microcontroller build cuts** (to be decided with the owner): smooth text, screen readers, input methods, multiple tabs and windows, developer tools, and probably the network fetcher, depending on where its teletext comes from. It keeps pages, page-number and Fastext keys, flash, and a simple page wall. Its security model is that of a single-purpose device and needs its own analysis.

---

## 8. Libraries and licences

The engine's own dependencies are the engine's concern, in its own repository (section 4.0).

Every library below is under a permissive licence, as far as checked; each still needs the owner's approval as a library choice (requirement 14.3).

| Library | Role | Licence |
|---|---|---|
| `vello_cpu` | Drawing on the processor | Apache-2.0 or MIT |
| Vello (graphics processor), `vello_hybrid` | Optional faster drawing | Apache-2.0 or MIT |
| Parley, Fontique | Text layout, shaping, editing, font fallback | Apache-2.0 or MIT |
| Masonry (option A) | Widgets | Apache-2.0 |
| Taffy | Layout (option B, and possibly within Masonry) | MIT |
| AccessKit | Screen readers | Apache-2.0 or MIT |
| winit | Windows and input | Apache-2.0 |
| softbuffer | Showing processor-drawn pixels | MIT or Apache-2.0 |
| `ui-events` | Uniform input events | to confirm |
| `arboard` | Clipboard | to confirm |
| `muda` | Native menu bar (requirement 12.5) | to confirm |
| `idna`, `unicode-security` | Safe display of internationalised addresses in the strip (requirement 11.3) | to confirm |
| `embedded-graphics` | Microcontroller drawing | MIT or Apache-2.0 |
| tiny-skia | Alternative processor drawing | BSD-3-Clause |
| Roboto, Open Sans | The interface's fonts (section 12.1) | SIL Open Font License 1.1 |

**To settle for "totally open":**
- **Parley's Unicode data** comes from ICU4X, under the Unicode License, which is permissive but not MIT or Apache.
- **Fonts:** shipped fonts must be chosen under a licence the owner accepts (the SIL Open Font License is the usual one for good fonts). The teletext fonts are the project's own.
- **The default fonts of other libraries** (for example macroquad's) are not used.

---

## 9. Decisions asked of the owner

1. **Adopt a native interface with no web engine**, superseding decision 3 (the interface drawn with HyperText Markup Language and Cascading Style Sheets), decision 19 (React), and the second half of decision 1 (the Tauri shell). Decision 20 (security does not depend on the interface's code) stays, with the interface process in place of the window host.
2. **Drawing on the processor as the minimum**, with `vello_cpu` and Parley; Vello on the graphics processor optional.
3. **Widgets:** prove Masonry first, with our own widgets on Taffy, Parley and AccessKit as the fallback (section 4.3).
4. **Process model:** a separate interface process with no secrets, drawing the trusted strip (section 5.2, option 1).
5. **Retire decisions 27 to 32** (window host, Linux confinement, nested display, display host, Smithay, input methods and screen readers through the display host), and the approved but unrecorded system call filter and security context for the Linux window host, keeping their measurements in `prototypes/docs/proofs.md` as the record of why a web engine was not kept.
6. **Frames on the pipe stay indexed Portable Network Graphics images** (section 6).
7. **Microcontrollers** become a target with a cut feature set, to be defined (section 7).
8. **The libraries of section 8**, each by name.

---

## 10. Risks and what is not yet proven

| Risk or unknown | How it would be settled |
|---|---|
| Masonry draws with `vello_cpu` only in unreleased, alpha source | Proof N1 (section 12) at a pinned commit |
| `vello_cpu` 0.3 and Parley 0.12 are young; their interfaces change | Pin versions; keep our drawing interface small so a back end can be replaced |
| Frame time on phone-resolution screens with no graphics processor | Measure on iOS and Android simulators and devices |
| Windows and Linux behaviour of every prototype | Build and run there |
| Screen readers and input methods on each platform, with Masonry or our own widgets | Test with VoiceOver, Narrator, Orca, TalkBack, and an input method per platform |
| A fault in our own interface code could imitate the strip | Small code, `unsafe` forbidden, fuzzing of the interface's message readers, the hostile interface harness, recorded spoofing reviews |
| Shaping untrusted text (cleaned) with Parley | Keep it in the interface process; fuzz the interface's text path with hostile strings |
| Reduced motion (requirement 12.2) has no crate in the list | Read each platform's setting directly |
| `vello_cpu`'s multithreading spins while idle | Keep it off, or report it upstream if the owner wishes |

---

## 11. Review against the existing documents

Every recorded decision and requirement, and what this design does to it. "Kept" means unchanged; "changed" says how; "retired" means it existed only for the web engine.

### 11.1 Decisions (`prototypes/docs/architecture.md` section 2)

| Decision | Effect |
|---|---|
| 1 (engine `neoteletext`; Tauri shell replaces egui shell) | Engine kept. Shell part superseded: the shell is our own native Rust interface |
| 2 (Tauri 2) | Retired: no Tauri |
| 3 (HyperText Markup Language and Cascading Style Sheets interface) | Superseded; its goal (rich animation of many pages) kept |
| 4 (security paramount) | Kept, and this design is weighed against it (section 5) |
| 5 (pages as images, never over a local port) | Kept |
| 6 (32-colour palette, lossless) | Kept |
| 7 (many pages at once) | Kept |
| 8 (whole-frame replacement) | Kept |
| 9 to 17 (tab security model, protocols, content, updates) | Kept; untouched by the interface |
| 18 (hardened process model) | Kept in shape, with the interface process in place of the web-engine window host |
| 19 (React) | Superseded |
| 20 (interface code irrelevant to security) | Kept: requirement 3.9 and its harness apply to the interface process |
| 21 | Already superseded by decision 22 |
| 22, 23 (fetching never depends on viewing; exception for separate page files) | Kept |
| 24 | Kept, except item (9), which this design fulfils as the palette-index canvas |
| 25, 26 | Kept |
| 27 to 32 | Retired (section 9, item 5) |
| Approved, not yet recorded: system call filter and security context for the Linux window host | Retired before recording, unless the owner wants them recorded and then retired; a system call filter applies instead to the interface process (section 5.3) |

### 11.2 Design principles (`prototypes/docs/architecture.md` section 4)

| Principle | Effect |
|---|---|
| 1, 2, 5, 6, 7 | Kept |
| 3 (the web interface is untrusted; its code is irrelevant to security) | Changed: the interface process is untrusted in the same sense, with no web technology |
| 4 (the webview never receives teletext data) | Changed: the interface process never receives network bytes, only checked frames and cleaned text |

Decision 27's other finding, that a strip drawn in a separate process's window can be covered by any other window, is respected: the strip is drawn in the interface process's own window (section 5.2).

### 11.3 Requirements (`prototypes/docs/requirements.md`)

| Requirements | Effect |
|---|---|
| 1.1 to 1.3, 2.1 to 2.4 | Kept. 1.3's check gains: the interface contains no interpreter at all |
| 3.1 (separate processes) | Changed: "window host" becomes "interface process" |
| 3.2 to 3.5, 3.7, 3.8 | Kept; 3.7 gains the interface process's sandbox (section 5.3) |
| 3.6 (window host holds no secrets) | Changed: applies to the interface process, now with **no network on every platform** |
| 3.9 (security does not depend on the interface's code) | Kept; the hostile interface harness replaces the interface process |
| 3.10, 3.11 (Linux window host confinement, nested display) | Retired |
| 4.1 to 4.7 (messages) | Kept; "window host" becomes "interface process" |
| 5.x, 6.x, 7.1 to 7.8, 7.10 to 7.12, 8.x | Kept |
| 7.9 (the webview holds no channel data) | Changed: the interface process holds no channel data and no storage |
| 9.1 to 9.7, 9.9 | Kept; 9.5 and 9.6 implemented in the interface core |
| 9.8 (scaling by Cascading Style Sheets) | Changed: scaling and pixel shape in the page's drawing transform |
| 10.1 (no teletext data reaches the webview) | Changed: no network byte reaches the interface process |
| 10.2 (page text rendered by React as text nodes) | Changed: page text is only ever drawn as text runs; there is no markup to inject into |
| 10.3, 10.4, 10.5, 10.6, 10.7, 10.11 | Retired: JavaScript, Content Security Policy, webview navigation, Tauri capabilities and page-code networking do not exist |
| 10.8 (no inspector or remote debugging) | Changed: no debugging interface reachable from outside the process in release builds |
| 10.9 (dropped files go to the trusted core) | Kept |
| 10.10 (Linux WebKitGTK version check) | Retired |
| 11.1 (strip in the webview's window) | Changed: the strip is drawn by the interface process in every window, outside the page area, and its input is still a separate message kind |
| 11.2 to 11.5 | Kept; 11.3 implemented with Parley, `idna` and `unicode-security` |
| 12.1 to 12.8 | Kept; implemented with Masonry or our own widgets. 12.2's "Cascading Style Sheets transforms and view transitions" becomes our own transitions; reduced motion must be read from each platform (section 10) |
| 12.9 (accessibility) | Kept; AccessKit in the interface process on every desktop. Its Linux display-host note is retired |
| 13.x (updates) | Kept; the application becomes far smaller to download |
| 14.1, 14.3, 14.5 | Kept |
| 14.2 (strict compilation) | Kept; the TypeScript half no longer applies |
| 14.4 (runtime attribution test) | Retired with Tauri; replaced by a test that each window's messages are attributed to that window |
| 15.1, 15.2 | Kept, and extended to microcontrollers with a cut feature set |

### 11.4 Security analysis (`prototypes/docs/security-analysis.md`)

| Section | Effect |
|---|---|
| 1 (summary) | Changed: no web engine; the main remaining risks are the trusted core, strip spoofing by page images, update keys, and faults in our own interface code |
| 5.5, 5.6 (boundaries to and from the interface) | Changed: the three levels of compromise (page code, web content process, escape into the window host) collapse to one: a fault in the interface process |
| 5.7 (the trusted strip) | Changed: drawn by the interface process; no Linux special case |
| 6 (the system webview: the main concession) | Retired: the concession is no longer made |
| 7 (spoofing and the line of death) | Kept: page images can still imitate interface parts, so the strip and the recorded layout reviews stay |
| 13 (platform notes) | Changed: no webview notes; the Linux display-host note retired; microcontrollers added |
| 14 (comparison with the alternatives) | Changed: add this design |
| 16 (ranked risks) | Changed: web-engine risks (ranks 3, 4, 5, 9, 10 and 13) removed; a fault in the interface process added |
| 17 (checks) | Changed: webview configuration checks replaced by interface-process sandbox checks |

### 11.5 Proofs (`prototypes/docs/proofs.md`)

Kept as the record. Proofs A, B and the Linux proofs explain why a web engine was not kept; Proof C's frame sizes still apply to the pipe. Proofs D (hostile interface harness) and E (fetching independence) remain to be built, unchanged in purpose.

### 11.6 Things not to miss

These are easy to lose in a move away from a browser engine, and each is required somewhere:

1. **Teletext forms** (requirement 7.10) need text fields in the page area, not only in the strip.
2. **The native menu bar and shortcuts** (requirement 12.5) need a menu library (`muda`) or each platform's own.
3. **Reduced motion** (requirement 12.2) must be read from each platform.
4. **Internationalised addresses shown safely** (requirement 11.3): the browser engine did none of this for us anyway, but it now sits entirely in our own strip code.
5. **Cell clicks** (requirement 12.3): exact, because the interface lays out every page itself.
6. **The 4:3 pixel-shape correction** (requirement 9.8) moves into the drawing transform.
7. **Clipboard, drag and drop, multiple windows** (requirements 10.9 and 12.8): from `arboard` and winit.
8. **The developer tools panel** (requirement 12.7) needs text tables, from the widget layer.
9. **Localisation of the interface's own text** has never been specified in any document; it should be, now that the interface's text is ours.
10. **The cookie store moving into the trusted core**, and **macOS sandboxing of the fetcher and decoder**, are still unsolved (`prototypes/docs/architecture.md` section 5.1, requirement 3.8); this design does not change them.
11. **The ten open questions for neoteletext** (`neoteletext/docs/neoteletext-browser-open-questions.md`) are still unanswered and are independent of the interface.
12. **The display scale:** every measurement here was at a scale of 1; drawing on the processor costs about four times as much at a scale of 2.

---

## 12. Next steps

**Built on 15 October 2026, in `native/`** (measured on macOS 15.6, Apple silicon, display scale 1):

| Part | Crate | State |
|---|---|---|
| Interface core: tab state, layout, slider, transitions, hit-testing of page cells, visible pages | `interface-core` | `no_std`, draws only through its `Canvas` trait |
| Drawing on the processor | `draw-vello` | `vello_cpu` and Parley; 1280 by 820 drawn in 10.0 milliseconds (first frame, fonts loading) |
| Drawing on a microcontroller | `draw-embedded` | `embedded-graphics`, `no_std`; the same tab drawn into an 800 by 480 display in memory |
| Messages between trusted core and interface | `protocol` | Framed and bounded; each side refuses the other side's kinds; text checked for hidden characters on both sides |
| Interface process | `neoteletext-interface` | 4.7 megabytes with its two fonts; reads indexed images with the `png` crate; on macOS runs under a sandbox profile with no network and no file writes |
| Trusted core | `neoteletext` | 0.5 megabytes; starts the decoder and the interface, routes visible pages to the decoder, drops unchanged frames, encodes images, sets the strip, flashes at 0.75 hertz (neoteletext decision E.12) |
| Decoder | `neoteletext-decoder` | 0.4 megabytes; the stand-in's decoder until the engine's exists (section 4.0) |

Running, the interface used about 215 megabytes and the trusted core 18; while every page flashed, the interface used about 5% of one core and the trusted core about 2%. Killing the trusted core ended the other two processes.

### 12.1 The browser window (second build, 15 October 2026)

Built to the owner's mockups (`pagebanks.png`, `500kultur.png`, `currentzoomingui.png`), with the owner's answers of the same day: build on `native/`, labels in English, the window frame as Chrome draws it, and an overview of every page along the bottom.

| Part | What it does |
|---|---|
| Tab strip | Coloured tabs (colour chosen by the trusted core from the site), close buttons, a new-tab button. On macOS the system's window buttons sit at its left end; elsewhere the browser draws its own minimise, maximise and close buttons, the strip drags the window and its edges resize it |
| Toolbar (the trusted strip) | Back and forward (dimmed when history has nothing), a padlock or "Not secure", the address with its host in full ink, a bookmark star, a menu button. Clicking the address selects it for typing; Enter sends it to the trusted core as strip input |
| Page banks (left) | 100 to 800, each with its colour, range, name and page count; the chosen bank outlined in its colour |
| Page list (middle) | The chosen bank's pages: number, status (shown, loaded, available) and title; scrolls; Up and Down step through it; three digits typed choose a page |
| Page area (right) | The page size control (−, segmented bar, +, percentage) inside the panel, filling the row (the owner removed flat, tilt and fan). The bar is logarithmic and steeper towards its right: a pull to the left from the widest halves the page within about 14% of the bar, and the small sizes get the rest for fine control; eight presses of − or + halve or double it; the bank's pages as a grid that keeps shrinking over the slider's whole travel, down to 80 pixels a page, and a list only at the slider's left edge itself (no band of travel, no delay); the chosen page outlined |
| Overview (bottom) | All 800 pages in eight rows: empty, available, loaded (in the bank's colour) and shown (outlined); a dot that shrinks away on each page as it loads; the counts |
| Trusted core | Tabs, history, bookmarks, address rules, one decoder per channel, every channel loading one page each 0.2 seconds in its own order whatever is on screen (decision 22) |

**Text** is drawn in Roboto (Open Sans is shipped too, for comparison), both from Google Fonts under the SIL Open Font License 1.1, in small capitals: every letter a capital, letters written in lower case drawn 80% as large. The owner chose these fonts on 15 October 2026.

**Widgets** are our own (section 4.3, option B): the address field's editor is in the interface core, so it works without the standard library too; text is laid out by Parley.

**Measured** on macOS 15.6, Apple silicon, display scale 1, window 1440 by 900: a whole frame draws in about 13 milliseconds (building the scene 3 to 4.5, rendering about 3, handing it to the window about 6); while a channel loads, the interface uses about 25% of one core, drawing the overview's dots ten times a second, and the trusted core about 1%. Once the channel has loaded, the interface uses about 6% (redrawing for flash, which every stand-in page has) and about 200 megabytes of memory.

### 12.2 The teletext engine integrated (15 October 2026)

The decoder process is now the engine. What each process does with it:

| Process | Engine crates | What it does |
|---|---|---|
| Decoder (`neoteletext-decoder`) | `neoteletext-core`, `neoteletext-formats`, `neoteletext-image` (palette) | Detects the format of a channel's bytes and reads them into packets (T42 recordings, TTI page files, transport streams, sliced records); decodes them into the engine's page store; reports each display page as it is first received completely, with a title from the first text row of the engine's grid (its character sets, mosaics left out); composes shown pages at the engine's default presentation level and renders them in its resolved colour mode, with Bedstead, at its own flash timing (decision E.48), into 480 by 500 palette indices with a palette of at most 32 colours. No network and no file writes: on macOS it runs under the same sandbox profile as the interface. |
| Trusted core (`neoteletext`) | `neoteletext-image` (PNG writer), `neoteletext-core` (flash timing constants) | Reads a channel from a file or a folder of page files, never parsing it, and hands it to the channel's decoder in pieces of 64 packets every 0.2 seconds, whatever is on screen (decision 22); starts one decoder per channel; tells the decoder which pages are shown and prompts it once per flash unit; checks and encodes each frame. |

**Pinning.** The dependency is a `git` source at the engine's commit `f341e94` in its local repository, because that commit is not yet published; Cargo reads the commit, never the engine's working tree, where another session works. When the commit is published, the source becomes the engine's own repository address at the same commit.

**Addresses.** A channel is opened by a `file://` address or an absolute path, given on the command line (`neoteletext <address>...`) or typed into the strip. The strip shows "File" for these. Web addresses are accepted but open an empty channel, because the fetcher is not built.

**Messages between trusted core and decoder** (`protocol::decoder`): to the decoder, data, end of data, the pages shown, a tick and close; from it, a page arrived (number and title, which the trusted core cleans) and a rendered page. Each direction refuses the other's kinds; every length, page number and palette index is checked on arrival.

**Measured** with the Teefax recording of 10 October 2026 (715 kilobytes) and the engine's 56 authored Level 2.5 and 3.5 test pages: 247 pages of the recording arrived in about 55 seconds at that pace; after one minute the decoder of the shown channel had used about 7 seconds of processor time (it renders the shown pages twelve times a second for flash), the other decoder almost none; the decoders used 12 to 21 megabytes and the interface about 120.

**Since then, at the owner's request:** the page list shows only page and title, in a narrower column; a click on a page in the page area zooms it to fill the area, and Escape or its close button returns to the wall (a click on a zoomed page is a click on its cells). The zoom is a shared-element transition, as Material's container transform and the web's FLIP technique: the page moves and grows from its place on the wall to its zoomed place over 300 milliseconds on Material 3's standard curve, `cubic-bezier(0.2, 0, 0, 1)`, while the wall dims behind it; closing runs the same path back to the page's current place, and Escape part-way reverses from where the page is, without a jump. The microcontroller back end draws the movement without the dimming. Over a page that a click would zoom, the pointer is a hand, and the page pops out: 6% larger about its centre over 150 milliseconds, ease-out, with a shadow and a light frame, drawn over its neighbours so the layout does not move; it settles back the same way when the pointer leaves. The slider's right end is the zoomed state: sliding to it zooms the chosen page (or the first on screen), leaving it zooms out, and zooming by a click moves the thumb there, Escape putting it back. Zoomed, the wheel (one page per 20 pixels of scrolling, a notch being 21) and the Up and Down keys move to the next or previous page of the bank: the pages slide as a vertical strip over 220 milliseconds on the standard curve and come to rest snapped exactly in place; a step during a slide carries on from where it has reached, and the list and overview follow.

### 12.3 Web channels and push (15 October 2026)

A channel is a folder on a web server, as the engine's `docs/channels.md` defines it: `channel.json` names the push feed, `contents.txt` lists every page with its title under section headings, and each page is `NNN.tti`. NeoTeletext now opens such channels.

| Part | What it does |
|---|---|
| Fetcher (`neoteletext-fetcher`) | One per web channel, started by the trusted core. The engine's web module (`neoteletext-web`, the `push` feature) opens the channel (`channel.json`, then the bundle `index.t42` if there is one), reads `contents.txt`, fetches every page it lists in its order at the engine's rate, and holds the push connection, subscribed to all pages, over server-sent events. The program itself only moves bytes between the agent and plain TCP sockets. No TLS yet, so `https` channels are reported unreachable. On macOS it may use the network but write no files. |
| Trusted core | Starts a fetcher for each web channel; takes the contents file's titles and section names for the banks; passes page bytes, fetched or pushed, to the channel's decoder unread. A server on this computer or the local network is reached only when named with `--allow-local` (the engine's REQ-SEC-006). |
| Decoder | Reports a page that arrives for the first time, and a page whose new version differs from the last (a digest of its rows), as updated. |
| Interface | Pages listed but not yet arrived show as available. An updated page: in the overview, an amber ring bursts from its cell and an amber dot stays for a minute; in the page list, its row flashes amber and shows "Updated 12s"; in the page area, its frame glows amber, fading over 3 seconds, and its caption shows the same age. |
| Site tool (`neoteletext-site`, not shipped) | `build` makes "Halcyon Teletext", a fictional channel of all 800 pages, from sample recordings decoded by the engine, keeping each page in its original hundred where it can: a front page and section indexes of its own, `contents.txt` and `channel.json`. Ten pages are live (101 and 102 news, 150 newsflash, 201 and 202 markets, 302 and 303 sport results, 401 weather, 410 travel, 601 television), each with four versions. `serve` serves the folder and pushes one live page every 1.5 seconds as a server-sent event named `page`, rotating its versions and stamping row 23 "LIVE · UPDATED" with the time. The samples are broadcast recordings, so the built site stays in the ignored `native/target/sites` and is never committed. |

To run it: `neoteletext-site build target/sites <recordings>...`, `neoteletext-site serve target/sites`, then `neoteletext --allow-local 127.0.0.1 http://127.0.0.1:8070/halcyon/`. Measured: 218 of the 800 pages arrived in the first 45 seconds, at the engine's one request every 0.2 seconds; pushes arrived and showed as updates throughout.

**Not yet built:** TLS for `https` channels, and WebSocket push; following page links and Fastext from a click on a cell; sub-pages beyond the latest; the engine's contents files and bank names; reveal and the engine's viewer for page entry; rendering only when a flashing page changes, instead of twelve times a second.

### 12.4 The wheel and the overview (16 October 2026)

- **A mouse wheel's notch moves one zoomed page.** Measured with a probe window on macOS 15.6: a slow notch arrives as a single event of 0.1 lines (`scrollingDeltaY`; its raw line count is 1), and quick notches are sped up, 0.1, 0.9 and 2.8 within 80 milliseconds. At 21 pixels a line and 20 pixels a page, a slow notch moved 2.1 pixels, and about ten were needed for one page. Feeds that show one item at a time, as the fullPage.js library does, count only the wheel's direction and ignore it until the step's animation ends. Now any notch moves one page, and at most one page per 220-millisecond slide; trackpads, which report pixels, keep the distance rule.
- **The overview folds into the footer** with a chevron at its top right. Folded, it is one heading high, draws only its title and the chevron, records no new-page dots and asks for no redraws; opened, it is drawn from the channel as it is, the page list having kept it current.
- **A page in the overview pops up at its native size,** 480 by 500, above its cell, with the pointer a hand over every cell whose page has arrived. The page is added to the pages reported visible, so its decoder renders it whichever bank it is in. It never covers the tab strip or the trusted strip: on a short window it shrinks to fit below them.

**Not yet built (window):** redrawing only the parts of the window that changed (the overview's dots now redraw everything); the menu's contents; showing why an address was refused; paste and copy in the address field (needs a clipboard library); the text being composed by an input method; hover previews; screen readers; a compact layout for small microcontroller displays; a build for a microcontroller target (the target is not installed on this machine); Linux, Windows, iOS and Android; the system call filter for the interface process on Linux.


1. **Proof N1: Masonry with `vello_cpu`.** The same tab, with Masonry at a pinned source commit drawing through `vello_cpu`; measure size, idle use and frame time, and try `TextInput` as the address field with an input method and a screen reader.
2. **Proof N2: one core, two back ends.** The tab's interface core drawing through `vello_cpu` in a window and through `embedded-graphics` into an in-memory display built without the standard library.
3. **Proof N3: platforms.** The `vello_cpu` version built and run on Linux and Windows, and in iOS and Android simulators, with frame times at phone resolutions and at a display scale of 2.
4. **On the owner's decisions** (section 9): update `prototypes/docs/architecture.md`, `prototypes/docs/requirements.md` and `prototypes/docs/security-analysis.md` as section 11 lists.

### 12.5 Efficiency, then sandboxes on every platform (21 and 22 October 2026)

An analysis of the processes' interaction, memory, latency, threads and polling (21 October 2026) led to decision 53: the sound device stopped while silent, frames only for what is drawn, one flush per event, the trusted core's computed deadline and its outbox writer threads, the fetcher's threaded connects, flash wakes from used rates only, and the engine's E.103. A security review of the same day led to the macOS profiles from "deny everything" and, on the owner's instruction the next day, to decision 52 and the `confine` crate for Linux and Windows (`sandboxing.md`). Both were built and measured before being recorded, as `DEVELOPMENT.md` section 2.2 allows.

### 12.6 The window's controls (23 October 2026)

On the owner's requests of 23 October 2026 (decision 63): Reload joined the strip between Forward and Home, with Command-R and Control-R; a press of the middle button on a tab closes it; cookie management moved from a panel over the window into the developer console, as its Cookies tab; and every control drawn as an icon, a symbol or text that may be cut short gained a tip. The tips are listed each frame from where the controls were drawn, one place per part of the window (`interface-core/src/browser/tips_ui.rs`), so a control added later is given its tip where it is drawn. The same day the window was found not to reach the screen on macOS 15.6 under the interface's sandbox profile, which lacked the window manager's service (`sandboxing.md` section 6).
