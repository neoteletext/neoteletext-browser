# Sandboxing: security analysis

The analysis of [`sandboxing.md`](sandboxing.md), as `DEVELOPMENT.md` section 2.3 asks of a feature that adds inputs, outputs or dependencies. It extends [`security-analysis.md`](security-analysis.md); its findings are in that document's sections 5.1, 5.3, 5.5, 5.7, 5.8, 13 and 16.

## 1. What the sandboxes protect

The property of decision 20: a compromised helper, by a fault in a reader of untrusted data or in a dependency, gains nothing beyond its pipe. Concretely, a compromised decoder must not read the user's files, reach the network or start a program; a compromised fetcher must not read the user's files or start a program, and reaches only the network, which it had; a compromised interface must not reach the network or files beyond fonts and preferences, and must not start a program; a compromised sound helper reaches the sound output and nothing else; a compromised trusted core must not reach the network, the user's files beyond its settings and what a file channel could name, or start any program but the browser's own helpers, each confined by the broker (decisions 55 and 56).

## 2. Threats to the sandboxes themselves

| Threat | Where | Control |
|---|---|---|
| A layer is missing on the user's machine | Linux | Every layer is required (decision 58): without the namespace, Landlock 5 enforced whole, or a filter for the architecture, the process ends with its reason and the browser does not run. Tested: `a_helper_without_its_namespace_does_not_run`, `a_file_to_read_keeps_landlock_whole`. |
| A new C library release uses a system call the filter does not expect | Linux | A deny-list: new calls pass; only named dangerous classes are refused. The cost is a weaker filter than an allow-list; the gain is a browser that keeps running. |
| The filter is applied after a thread exists, leaving that thread unconfined (the engine's `neottx` has this gap) | Linux | `confine::apply` is the first call of each helper's `main`. |
| `clone3`, whose flags the filter cannot read, starts a process | Linux | Answered "not implemented", so the C library falls back to `clone`, which the filter inspects for `CLONE_THREAD`. |
| An `AF_UNIX` socket reaches a service the helper should not (D-Bus, the display) | Linux | A decoder may open no socket at all. A fetcher and the interface may; what they reach is then the service's own access control. Landlock's TCP rule stops the fetcher's family set from being widened by a mistake in the filter. |
| A helper reads a path Landlock lets through | Linux | Read-only rules on system directories and, for the interface, fonts and preferences; nothing under the user's documents; nothing writable but `/dev/snd` for the sound helper and the settings folder for the trusted core. |
| The AppContainer cannot be made or started (for one, without the user's profile folders in the environment) | Windows | The process is not started, and the browser says why (decision 58); there is no weaker fallback and no switch to one. Tested: `a_helper_whose_container_cannot_be_made_is_not_started`, which failed on the code that fell back. |
| A handle the parent holds open is inherited | Windows | The handle list attribute: the two pipes, and standard error when there is one, and nothing else. The broker duplicates a helper's pipe handles into the core's process one by one, with the same access, and closes its own. |
| A compromised trusted core asks the broker for a program, role or file it should not have | All | It names one of five helpers, never a path or a role; the broker reads a file channel by the channel rules, only teletext (or an import's bookmarks file), never beneath a dot-entry of the home folder, applications' data or the browser's own folders, and never through a link, opened by descriptor (decision 88); every request is bounded and fuzzed (`sandboxing.md` section 7, `security-analysis.md` section 5.8). |
| The trusted core's container fails to start | Windows | No core and no browser (decision 58). |
| A DLL the broker needs breaks the core's start under the Win32k filter | Windows | `shell32.dll` is delay-loaded in `neoteletext.exe` (`core/build.rs`); `the_trusted_core_starts_confined_and_reaches_its_own_code` starts the core confined on every platform. |
| The helper outlives a crashed trusted core | Windows | The job's kill-on-close; the core's end closes the job. On Unix the helper ends at its pipe's end. |
| The restricted token still reads the user's files | Windows | Low integrity denies writes, not reads; reads are denied by the AppContainer, which the decoder and fetcher have and the interface, which needs the desktop, does not. The interface's file reads are the remaining gap on Windows. |
| `sandbox-exec` is deprecated and goes away | macOS | Still present on macOS 15; packaging must keep each process under a profile of its own (requirement 3.8). |
| The environment carries the user's secrets or tokens to a helper | All | `env_clear` with an allow-list: nothing for a decoder, fetcher or keyboard beyond what Windows needs to start a process; the window system's variables for the interface. |
| The report lies, because a compromised helper prints it | All | The report is for the user's information; nothing decides on it. |

## 3. What each platform's sandbox does not stop

- **Linux:** a fault in the kernel; a Unix-socket service that grants more than it should.
- **Windows:** the interface's reads of the user's files at low integrity; a fault in the kernel or in a service the token may still reach.
- **macOS:** whatever `system.sb` allows every process, which Apple maintains.
- **All:** a fault in the broker's parser, the one process not confined (`security-analysis.md` section 5.8); a compromised trusted core still holds the browser's state and draws nothing but can mislead through what it sends the interface (`security-analysis.md` rank 4).

## 4. Dependencies added

| Crate | Licence | Why it is trusted |
|---|---|---|
| `landlock` 0.4.7 | MIT or Apache-2.0 | By Mickaël Salaün, the Landlock kernel maintainer; approved in the engine (E.33) |
| `seccompiler` 0.5.0 | Apache-2.0 or BSD-3-Clause | The rust-vmm project's, run in production hypervisors; approved in the engine |
| `libc` 0.2.190 | MIT or Apache-2.0 | Already in the build through the standard library's dependents |

Windows adds no crate; `windows-sys` 0.52 is already a dependency of `web-opener` and `winit`.

## 5. Remaining risk

Low on macOS and Linux for the helpers and the trusted core; low for the broker, small and parsing only the core's bounded requests; **medium for the interface on Windows** (file reads).
