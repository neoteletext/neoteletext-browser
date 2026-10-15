# T42 Browser: native interface design

| | |
|---|---|
| **Status** | **Proposed, not decided.** 15 October 2026. Nothing here changes a recorded decision until the owner decides it; section 9 lists the decisions asked for. |
| **Purpose** | To specify an interface drawn natively in Rust, with no web engine, that runs on macOS, Windows, Linux, iOS and Android and, with features cut, on microcontrollers; and to review it against [`architecture.md`](architecture.md), [`requirements.md`](requirements.md), [`security-analysis.md`](security-analysis.md) and [`proofs.md`](proofs.md) so that nothing important is lost. |
| **Evidence** | Nine interface prototypes built on 15 October 2026, all under `spikes/` (section 2). Measurements are from macOS 15.6 on Apple silicon at a display scale of 1 unless stated. |

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

T42 Browser's interface would be drawn by T42 Browser's own Rust code, with no web engine:

- **Drawing on the processor is the minimum.** Every target must work with no graphics processor. `vello_cpu` draws on the processor; a graphics-processor renderer of the same family is an optional upgrade where it helps.
- **One interface core, several drawing back ends.** The state, layout, animation, hit-testing and input handling are written once. They draw through one small drawing interface, with back ends for desktops and phones (`vello_cpu`, optionally Vello on the graphics processor) and for microcontrollers (`embedded-graphics`).
- **Widgets come from Masonry, or are ours.** Masonry, Linebender's widget layer, provides the text input, buttons, sliders, scrolling, screen-reader support and input methods the shell needs, and in its unreleased source can draw with `vello_cpu`. If it proves unsuitable, the same widgets are built on Taffy (layout), Parley (text and editing) and AccessKit (screen readers).
- **No web engine means most of the web-engine security work is no longer needed.** Decisions 27 to 32, and the two approved but unrecorded decisions on the Linux window host's system call filter and the Wayland security context, exist only to contain a web engine. The process separation of decision 18 for fetching, decoding and the trusted core stays.

The cost: T42 Browser owns its interface code, including the parts browsers give for free (text editing, accessibility, input methods), and depends on young libraries.

---

## 2. What was evaluated

Every prototype shows the same tab as the React prototype: a header with the thumbnail width slider, its readout and the flat, tilt and fan transform picker, and a wall of teletext pages from the stand-in engine. Sizes are release builds with symbols stripped.

| Prototype | Folder | Size | Needs a graphics processor | Text | Notes |
|---|---|---|---|---|---|
| React in the system webview (Tauri 2) | `src`, `src-tauri` | 11 megabytes (application bundle) | No | The browser's | The current design; brings decisions 27 to 32 |
| Tauri 3 with the Chromium Embedded Framework | branch `tauri3` | about 334 megabytes | No | The browser's | Caller attribution failed (`architecture.md` section 16); no phones |
| Chromium Embedded Framework directly | Linux test machine only | 1.5 gigabytes unstripped | No | The browser's | The bindings' own example; our application not completed |
| Bevy | `spikes/bevy-wall` | 54.0 megabytes | Yes | No editable text field | Redrew every frame until set to desktop mode |
| Dioxus native (Blitz) | `spikes/dioxus-wall` | 19.7 megabytes | Yes, by default | Real text; no range input; two-dimensional transforms only | Pointer positions only in window coordinates |
| egui | `spikes/egui-wall` | 10.3 megabytes | Yes | The best text field of the native options | Scroll bar needed its space kept to stop a redraw loop |
| macroquad | `spikes/macroquad-wall` | 1.0 megabyte | Yes | Basic | Its blocking event loop must be told which input wakes a frame |
| tiny-skia, no framework | `spikes/skia-wall` | 0.9 megabytes | **No** | Bitmap fonts only | 3 to 4 milliseconds a frame |
| **`vello_cpu` and Parley, no framework** | `spikes/vello-wall` | **3.3 megabytes** | **No** | **Shaped Unicode text in system fonts** | **1.9 milliseconds a frame** single-threaded; its multithreading kept a core busy while idle and is off |

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

- **The interface core** is the tab and wall logic proven in `spikes/vello-wall/src/tab.rs`: layout of the header and wall, the slider's hysteresis and dwell (from the React prototype's `thumbnailSize.ts`), the transitions with the prototype's timing curves (`tween.rs`), and the flat, fan and tilt transforms with the perspective Cascading Style Sheets uses (`pose.rs`).
- **The drawing interface** is the set of calls the prototypes needed. Pages are images drawn under an affine transform; the tilt's perspective is drawn as strips, each exact for its own corners, because both `vello_cpu` and tiny-skia transform images affinely only.
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
| **B. Our own widgets** | Exactly what T42 Browser needs, all reviewable: Taffy for layout, Parley's `PlainEditor` for the address field, AccessKit for screen readers, `arboard` for the clipboard, input methods from winit (proven in the Linux display host, `proofs.md` section 6.16) | Text editing, accessibility and input methods done well are real, permanent work |

**Recommended:** prove option A first (section 12); fall back to option B where it falls short. Either way the microcontroller build uses its own minimal widgets on `embedded-graphics`.

### 4.4 Input

- Pointer, wheel, keys, input methods and dropped files arrive through winit (on phones, touch through winit as well), normalised by `ui-events` where Masonry is used.
- **Cell clicks** (requirement 12.3) are computed from the pointer position mapped back through the page's own transform, which the interface core owns, so the mapping is exact under every transform.
- **Keys** reach the trusted core's input router unchanged (requirement 12.4).

---

## 5. Process model and security

### 5.1 What changes

With no web engine, nothing in the interface runs code that was not shipped with the application, and nothing in it parses data from the network. The interface receives only checked frames from the trusted core, cleaned and bounded text, and state. The window host of decision 27, the Linux display host of decisions 29 to 32, and the confinement of decision 28 existed to contain a web engine and are no longer needed for that.

### 5.2 Where the interface runs

| Option | Desktop and Android | What a compromise of the interface would gain |
|---|---|---|
| **1. A separate interface process, recommended** | Keeps decision 18's shape: trusted core, fetcher, decoders, font helper, **interface process**, updater, launcher. The interface process draws the window, the wall and the trusted strip, holds no secrets, and can be denied the network and files on every desktop, including macOS, now that there is no WebKit (proof A's limit, `proofs.md` section 2, no longer applies). | The ability to draw a false strip and send intents and strip input, which the trusted core checks; no secrets. Reaching it needs a fault in our own Rust interface code or its libraries, fed only by our own frames and cleaned text. |
| 2. The interface inside the trusted core | One process fewer, as `neoteletext`'s egui design had it. | Everything: the drawing and text-shaping libraries would sit beside every secret. |

On iOS and on microcontrollers there is one process in any case, as `architecture.md` section 13 already records for iOS.

**The trusted strip under option 1** is drawn by the interface process. That was the position before decision 27 for the strip's integrity against page code, and here there is no page code: the strip can be imitated only by a fault in T42 Browser's own interface process. Requirement 3.9's hostile interface harness, which replaces the interface process with an attacker's, still holds every other property.

### 5.3 Sandboxing the interface process

- **No network** on every platform (it needs none).
- **No files** beyond its own resources and shipped fonts.
- **Display access** is needed, as for any application. Under X11 any application can send input to others (`proofs.md` section 6.8); the interface process is T42 Browser's own code, not a web engine, so this is the same exposure every desktop application has, not a containment problem.
- **A system call filter** on Linux, and the platform sandboxes of `architecture.md` section 6.5, as for the decoder.

---

## 6. How a page reaches the screen

Steps 1 to 3 and 6 of `architecture.md` section 10 are unchanged: the decoder renders palette indices; the trusted core checks every frame, keeps flash phases and never resends an unchanged frame.

**Transport to the interface process.** Two options:

- **Keep the indexed Portable Network Graphics images** of requirement 9.3 on the pipe: about 6 kilobytes a page (proof C), already written by `neoteletext-image`. The interface process decodes them with a pure-Rust decoder, reading only images our own encoder produced. **Recommended.**
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

**What the microcontroller build cuts** (to be decided with the owner): perspective tilt and fan, smooth text, screen readers, input methods, multiple tabs and windows, developer tools, and probably the network fetcher, depending on where its teletext comes from. It keeps pages, page-number and Fastext keys, flash, and a simple page wall. Its security model is that of a single-purpose device and needs its own analysis.

---

## 8. Libraries and licences

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
5. **Retire decisions 27 to 32** (window host, Linux confinement, nested display, display host, Smithay, input methods and screen readers through the display host), and the approved but unrecorded system call filter and security context for the Linux window host, keeping their measurements in `proofs.md` as the record of why a web engine was not kept.
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

### 11.1 Decisions (`architecture.md` section 2)

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

### 11.2 Design principles (`architecture.md` section 4)

| Principle | Effect |
|---|---|
| 1, 2, 5, 6, 7 | Kept |
| 3 (the web interface is untrusted; its code is irrelevant to security) | Changed: the interface process is untrusted in the same sense, with no web technology |
| 4 (the webview never receives teletext data) | Changed: the interface process never receives network bytes, only checked frames and cleaned text |

Decision 27's other finding, that a strip drawn in a separate process's window can be covered by any other window, is respected: the strip is drawn in the interface process's own window (section 5.2).

### 11.3 Requirements (`requirements.md`)

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
| 12.1 to 12.8 | Kept; implemented with Masonry or our own widgets. 12.2's "Cascading Style Sheets transforms and view transitions" becomes our own transforms and transitions; reduced motion must be read from each platform (section 10) |
| 12.9 (accessibility) | Kept; AccessKit in the interface process on every desktop. Its Linux display-host note is retired |
| 13.x (updates) | Kept; the application becomes far smaller to download |
| 14.1, 14.3, 14.5 | Kept |
| 14.2 (strict compilation) | Kept; the TypeScript half no longer applies |
| 14.4 (runtime attribution test) | Retired with Tauri; replaced by a test that each window's messages are attributed to that window |
| 15.1, 15.2 | Kept, and extended to microcontrollers with a cut feature set |

### 11.4 Security analysis (`security-analysis.md`)

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

### 11.5 Proofs (`proofs.md`)

Kept as the record. Proofs A, B and the Linux proofs explain why a web engine was not kept; Proof C's frame sizes still apply to the pipe. Proofs D (hostile interface harness) and E (fetching independence) remain to be built, unchanged in purpose.

### 11.6 Things not to miss

These are easy to lose in a move away from a browser engine, and each is required somewhere:

1. **Teletext forms** (requirement 7.10) need text fields in the page area, not only in the strip.
2. **The native menu bar and shortcuts** (requirement 12.5) need a menu library (`muda`) or each platform's own.
3. **Reduced motion** (requirement 12.2) must be read from each platform.
4. **Internationalised addresses shown safely** (requirement 11.3): the browser engine did none of this for us anyway, but it now sits entirely in our own strip code.
5. **Cell clicks under every transform** (requirement 12.3): exact, because the interface owns the transforms; still to be tested under each.
6. **The 4:3 pixel-shape correction** (requirement 9.8) moves into the drawing transform.
7. **Clipboard, drag and drop, multiple windows** (requirements 10.9 and 12.8): from `arboard` and winit.
8. **The developer tools panel** (requirement 12.7) needs text tables, from the widget layer.
9. **Localisation of the interface's own text** has never been specified in any document; it should be, now that the interface's text is ours.
10. **The cookie store moving into the trusted core**, and **macOS sandboxing of the fetcher and decoder**, are still unsolved (`architecture.md` section 5.1, requirement 3.8); this design does not change them.
11. **The ten open questions for neoteletext** (`neoteletext/docs/t42-browser-open-questions.md`) are still unanswered and are independent of the interface.
12. **The display scale:** every measurement here was at a scale of 1; drawing on the processor costs about four times as much at a scale of 2.

---

## 12. Next steps

1. **Proof N1: Masonry with `vello_cpu`.** The same tab, with Masonry at a pinned source commit drawing through `vello_cpu`; measure size, idle use and frame time, and try `TextInput` as the address field with an input method and a screen reader.
2. **Proof N2: one core, two back ends.** The tab's interface core drawing through `vello_cpu` in a window and through `embedded-graphics` into an in-memory display built without the standard library.
3. **Proof N3: platforms.** The `vello_cpu` version built and run on Linux and Windows, and in iOS and Android simulators, with frame times at phone resolutions and at a display scale of 2.
4. **On the owner's decisions** (section 9): update `architecture.md`, `requirements.md` and `security-analysis.md` as section 11 lists.
