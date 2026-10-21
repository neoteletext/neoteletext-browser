# Sandboxing: security analysis

The analysis of [`sandboxing.md`](sandboxing.md), as `DEVELOPMENT.md` section 2.3 asks of a feature that adds inputs, outputs or dependencies. It extends [`security-analysis.md`](security-analysis.md); its findings are in that document's sections 5.1, 5.3, 5.5, 13 and 16.

## 1. What the sandboxes protect

The property of decision 20: a compromised helper, by a fault in a reader of untrusted data or in a dependency, gains nothing beyond its pipe. Concretely, a compromised decoder must not read the user's files, reach the network or start a program; a compromised fetcher must not read the user's files or start a program, and reaches only the network, which it had; a compromised interface must not reach the network or files beyond fonts and preferences, and must not start a program.

## 2. Threats to the sandboxes themselves

| Threat | Where | Control |
|---|---|---|
| The layer that is relied on is missing on the user's machine | Linux | seccomp is required and the helper refuses to run without it; Landlock and the namespace are reported when missing, and the report is printed at start. An architecture with no filter is reported, not silent. |
| A new C library release uses a system call the filter does not expect | Linux | A deny-list: new calls pass; only named dangerous classes are refused. The cost is a weaker filter than an allow-list; the gain is a browser that keeps running. |
| The filter is applied after a thread exists, leaving that thread unconfined (the engine's `neottx` has this gap) | Linux | `confine::apply` is the first call of each helper's `main`. |
| `clone3`, whose flags the filter cannot read, starts a process | Linux | Answered "not implemented", so the C library falls back to `clone`, which the filter inspects for `CLONE_THREAD`. |
| An `AF_UNIX` socket reaches a service the helper should not (D-Bus, the display) | Linux | A decoder may open no socket at all. A fetcher and the interface may; what they reach is then the service's own access control. Landlock's TCP rule stops the fetcher's family set from being widened by a mistake in the filter. |
| A helper reads a path Landlock lets through | Linux | Read-only rules on system directories and, for the interface, fonts and preferences; nothing under the user's documents; nothing writable but `/dev/snd` for the interface. |
| The AppContainer cannot start because the program folder does not grant read to `ALL APPLICATION PACKAGES` (a user-folder installation) | Windows | The helper is started with the restricted token alone and the report says so; packaging sets the ACL (`TECH-DEBT.md`). The fallback is weaker, not open. |
| A handle the trusted core holds open is inherited | Windows | The handle list attribute: the two pipes, and standard error when there is one, and nothing else. |
| The helper outlives a crashed trusted core | Windows | The job's kill-on-close; the core's end closes the job. On Unix the helper ends at its pipe's end. |
| The restricted token still reads the user's files | Windows | Low integrity denies writes, not reads; reads are denied by the AppContainer, which the decoder and fetcher have and the interface, which needs the desktop, does not. The interface's file reads are the remaining gap on Windows. |
| `sandbox-exec` is deprecated and goes away | macOS | The App Sandbox at packaging is required anyway (requirement 3.8); the profiles are the interim. |
| The environment carries the user's secrets or tokens to a helper | All | `env_clear` with an allow-list: nothing for a decoder, fetcher or keyboard beyond what Windows needs to start a process; the window system's variables for the interface. |
| The report lies, because a compromised helper prints it | All | The report is for the user's information; nothing decides on it. |

## 3. What each platform's sandbox does not stop

- **Linux:** a fault in the kernel; a Unix-socket service that grants more than it should; and, on a kernel without Landlock, the decoder reading world-readable files (it still cannot send them: no sockets, no programs, no writes).
- **Windows:** the interface's reads of the user's files at low integrity; the fallback without an AppContainer on a user-folder installation; a fault in the kernel or in a service the token may still reach.
- **macOS:** the trusted core, unconfined until packaging; whatever `system.sb` allows every process, which Apple maintains.
- **All:** a fault in the trusted core, which the sandboxes do not confine (security-analysis.md rank 4).

## 4. Dependencies added

| Crate | Licence | Why it is trusted |
|---|---|---|
| `landlock` 0.4.7 | MIT or Apache-2.0 | By Mickaël Salaün, the Landlock kernel maintainer; approved in the engine (E.33) |
| `seccompiler` 0.5.0 | Apache-2.0 or BSD-3-Clause | The rust-vmm project's, run in production hypervisors; approved in the engine |
| `libc` 0.2.190 | MIT or Apache-2.0 | Already in the build through the standard library's dependents |

Windows adds no crate; `windows-sys` 0.52 is already a dependency of `web-opener` and `winit`.

## 5. Remaining risk

Low on macOS and Linux for the helpers; **medium for the interface on Windows** (file reads) and for the trusted core everywhere, until packaging.
