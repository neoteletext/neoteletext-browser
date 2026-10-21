# Sandboxing on every platform

| | |
|---|---|
| **Status** | Built 22 October 2026 on the owner's instruction ("do both"), after the options of 21 October 2026; decision 52 below records it, for the owner to confirm. |
| **Requirements** | 3.3, 3.4, 3.6, 3.7 (each process confined, from "deny everything"), 3.8 (the macOS gap), 10.12 (where unsafe code may be). |
| **Security analysis** | [`sandboxing-security.md`](sandboxing-security.md); [`security-analysis.md`](security-analysis.md) section 13 holds the per-platform summary. |
| **Code** | `native/confine`: `spawn`, the trusted core's side, and `apply`, a helper's side; `macos.rs`, `linux.rs`, `windows.rs`. |

## 1. What is wanted

Every helper of the trusted core, on every platform, runs with only what it is for: a decoder with its pipes alone; a fetcher and the phone keyboard with the network and the name resolver; the interface with the window system, fonts, text input, the pasteboard, the sound output and the Dock, and no network. The sandbox must hold on any machine the browser is installed on, without an installer step, root, or a kernel the user cannot be expected to have, and it must say what it applied.

The helpers only ever receive: everything they work on arrives over their pipe from the trusted core, which is why the sandboxes here deny and never broker. That is the property that keeps them small.

## 2. The options, by platform

**Linux** has four mechanisms a user-level program may use and one of them is on every machine.

| Mechanism | Kernel | On every machine? | Gives |
|---|---|---|---|
| seccomp-bpf | 3.5 (2012) | Yes: every distribution, WSL2, Docker's default profile, Flatpak | Refuses whole classes of system calls |
| Landlock | 5.13 (2021); TCP rules from 6.7 | No: RHEL 8, Ubuntu 20.04, Debian 11 lack it; some builds leave it out | Which directories may be read, and no TCP |
| User and network namespaces | 3.8 | No, and less each year: Ubuntu 24.04 restricts unprivileged user namespaces, Flatpak and containers forbid them | An empty network: nothing to connect to |
| A setuid helper | — | Needs a root-installed binary | Namespaces without unprivileged user namespaces |
| AppArmor or SELinux profiles | — | Root to install, distribution-specific | Strong, not portable |

Chromium and Firefox arrived at the same layering: the syscall filter is mandatory because it never fails on a user's machine; the namespace layer is best effort because it is the one that does. Both allow-list system calls, which each new C library release has broken; the engine's `neottx` deny-lists, which a browser with no maintainer on call needs. Both apply the sandbox before any thread exists, which the engine's module does not.

**Windows** has one model since Vista, which Chromium, Edge, Office and Adobe Reader use: the token is the sandbox. A job object (the helper dies with the core and may start nothing), a restricted token at low integrity (no writes to the user's files or registry), process mitigation policies (DEP, ASLR, CFG, no dynamic code, no child processes, the Win32k filter for a process without a window) and, from Windows 8, an AppContainer with named capabilities (none for the decoder; the internet client for the fetcher). All of it is applied by the parent at process creation, which suits the trusted core, and all of it is in `windows-sys`, already a dependency.

**macOS** was done on 21 October 2026 with `sandbox-exec` profiles from "deny everything"; `security-analysis.md` section 13 records how. macOS refuses a nested `sandbox-exec`, so the trusted core itself waits for the App Sandbox at packaging (requirement 3.8).

### 2.1 Library or not

Linux is raw system calls either way. With the crates `landlock` (by the Landlock kernel maintainer) and `seccompiler` (the rust-vmm project's filter builder), both already approved in the engine, the browser's own unsafe code on Linux is two `libc` calls. Hand-written, it is a new crate with unsafe system calls and a hand-assembled BPF program, where a mistake kills helpers silently or lets a call through. The crates were chosen. Windows needs no new library, only a third crate permitted unsafe code.

## 3. What was built

One crate, `confine`, so the trusted core keeps no platform code:

- `confine::spawn(Spawn { program, arguments, role, environment, local_network })` starts a helper confined as the platform's parent side allows, with its pipes, and an environment of exactly what the core passes (nothing for a decoder or fetcher; the window system's variables for the interface). It returns a `Process` with a `Report` of what confined it.
- `confine::apply(role)` is the first call in each helper's `main`, before any thread: on Linux it applies, in order, no-new-privileges, an empty network namespace for the roles without the network (best effort), Landlock with the directories the role may read (best effort, the newest the kernel has), and the seccomp deny-list (required; without it the helper ends and the core says so, except on an architecture no filter exists for, which is reported). On macOS and Windows it reports that the core confined it.
- Each helper prints what confined it, once, at start, and the core prints what it applied; the phone keyboard prints nothing of it, because that process prints one thing only (REQ-INT-137).

The seccomp deny-list: starting programs (`execve`, `execveat`, `fork`, `vfork`, `clone` without `CLONE_THREAD`, `clone3` answered "not implemented" so the C library falls back to `clone`), tracing and reaching into processes (`ptrace`, `process_vm_*`, `kcmp`), mounting and namespaces after our own (`mount`, `umount2`, `pivot_root`, `chroot`, `unshare`, `setns`), kernel interfaces a helper has no use for (`io_uring_*`, `bpf`, `keyctl`, `add_key`, `request_key`, `userfaultfd`, `perf_event_open`, `kexec_*`, `reboot`, `swapon`, `swapoff`, module loading, `acct`, clock and host-name setting, `open_by_handle_at`, `fanotify_init`, and on x86-64 `iopl` and `ioperm`), and sockets: none for a decoder; for a fetcher only `AF_INET`, `AF_INET6`, `AF_NETLINK` (the resolver asks which address families exist) and `AF_UNIX` (the name service cache); for the interface only `AF_UNIX` (the display, the sound server, the input method); `listen` and `accept4` for none.

Windows: the decoder in a less privileged AppContainer with no capabilities, the fetcher and keyboard in an AppContainer with the internet client capability and, when the user has allowed local addresses, the private network one; every helper in a job object with kill-on-close and a limit of one process, with every mitigation policy above, no child processes, and for the decoder and fetcher the Win32k filter and the UI restrictions (no desktop, clipboard or handles); the interface with a restricted token at low integrity instead of a container, since it needs the desktop. Only the two pipe handles are inherited. An AppContainer can read only what grants read to `ALL APPLICATION PACKAGES`: an installation under Program Files does, a folder of the user's does not; when the container start fails, the helper is started with the restricted token alone and the report says so.

## 4. Measured

- **macOS**, 21 October 2026: the four helpers under their profiles; the browser fetches, decodes, draws and opens the sound device; idle costs unchanged.
- **Linux**, 22 October 2026, Ubuntu 26.04 on `foundintelquiet4060` (kernel 7.0, Landlock in the LSM list, unprivileged user namespaces allowed): the crate's probe tests pass (a decoder can open no file, start no program, connect nowhere and write nothing; a fetcher can start no program and write nothing, and has the network); the browser under Xvfb with the terminal channel reports the decoder "network namespace, Landlock, seccomp", the fetcher "Landlock, seccomp", the interface "network namespace, Landlock, seccomp", fetches and draws (47 frames in 20 s). Machines without Landlock or user namespaces run with seccomp alone and say so; none was to hand.
- **Windows**, 22 October 2026, Windows 11 in the virtual machine on `foundintelquiet4060`: the workspace builds and every test passes there; in the logged-in desktop session the browser runs with every layer on: the interface at low integrity in its job with the mitigation policies drew the channel's pages and opened the sound device, the decoder ran in a less privileged AppContainer. Started from an `ssh` session instead (session 0, no window station), the interface fails at `CoInitialize` with "access denied" at low integrity and the sound device is refused at medium: a test of the Windows sandbox must run in the desktop session (a one-off interactive scheduled task does it; `DEVELOPMENT.md` section 12.1). The network role reaches its relay from inside the sandbox: `the_keyboard_reaches_its_relay_confined` (`core/tests/confined_keyboard.rs`, needs the internet) passed in the virtual machine in both sessions, in the AppContainer and, with `NEOTELETEXT_CONFINE_NO_CONTAINER` forcing the fallback, under the restricted token at low integrity; and on macOS under the profile.

## 5. Decision 52 (for the owner to confirm)

Decided on 22 October 2026, on the owner's instruction, after the options of 21 October 2026.
1. Every helper is confined on every platform by that platform's own means, from "deny everything", with the layered, degrade-and-report design the two browsers converged on: the one layer every machine has is required (seccomp on Linux; the token and job on Windows; the `sandbox-exec` profile on macOS), the stronger optional layers are applied when the machine allows and reported.
2. On Linux the crates `landlock` and `seccompiler`, with `libc`, as the engine's own `neottx` uses (its decision E.33); the deny-list rather than an allow-list of system calls, so no C library or kernel release can stop a helper from running.
3. On Windows the Win32 calls in `windows-sys`, already a dependency; `confine` is the third crate permitted unsafe code (requirement 10.12 amended), on its Linux and Windows modules only, each block with its reason.
4. A helper confines itself first thing, before any thread, where the platform makes a process confine itself (Linux); the trusted core confines it at its start where the parent does (macOS, Windows).
5. The trusted core on macOS stays unconfined until the App Sandbox at packaging (requirement 3.8); macOS refuses a nested `sandbox-exec`.

*Reason:* security first (decision 4); a sandbox that depends on user namespaces fails on the machines users have, as Chromium's and Electron's did on Ubuntu 24.04; the crates keep the BPF program, where a mistake hides, with the people who run it in production.

- **Linux, sound and Wayland**, 22 October 2026, on the same host: with the sound server reachable (`XDG_RUNTIME_DIR` set), the confined interface opened the device through PipeWire; under a headless Wayland compositor (`sway`) the browser ran with every helper confined, 47 frames in 18 s, fetch and push connected. The crate's tests there pass (6, with the environment and probe tests).

## 6. What building changed

1. **The user namespace maps the user to themselves.** Unmapped, a process in a new user namespace is "nobody" to every other process, and PipeWire, which checks the peer's credentials, dropped the interface's connection; Wayland and X11 did not care. After `unshare`, the helper writes `deny` to `setgroups` and its own uid and gid to `uid_map` and `gid_map`, as `user_namespaces(7)` allows an unprivileged process to do.
2. **An AppContainer needs the user's profile variables** (`USERPROFILE`, `LOCALAPPDATA`, `APPDATA` and their kin) in the child's environment, or `CreateProcess` fails with "environment option not found"; the core passes them to every Windows helper.
3. **A Windows helper with a window must be started in the desktop session.** From an `ssh` session (session 0, no window station) the interface dies at `CoInitialize` at low integrity; the test runs through an interactive scheduled task.
4. Found on the way: `rsync` does not see a re-vendored crate, because `cargo vendor` stamps every file with the same date, so the Windows export is sent with `--checksum` and the virtual machine's vendor folder removed before its copy (`DEVELOPMENT.md` section 12.1).
