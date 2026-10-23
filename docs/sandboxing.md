# Sandboxing on every platform

| | |
|---|---|
| **Status** | Built 22 October 2026 on the owner's instruction ("do both"), after the options of 21 October 2026; decision 52 below records it, for the owner to confirm. The sound helper (decision 55) and the trusted core behind a broker (decision 56, section 7) built the same day on the owner's instruction ("DO THIS PROPERLY NOW NOT LATER"). Decision 58, the same day, rescinded decision 52's "degrade and report": every layer is required (section 8). |
| **Requirements** | 3.1 (the processes), 3.3, 3.4, 3.6, 3.7 (each process confined, from "deny everything"), 3.8 (the trusted core confined), 10.12 (where unsafe code may be), 16.6 (silent without a sound device). |
| **Security analysis** | [`sandboxing-security.md`](sandboxing-security.md); [`security-analysis.md`](security-analysis.md) section 13 holds the per-platform summary. |
| **Code** | `native/confine`: `spawn`, the parent's side, and `apply`, a process's own side; `macos.rs`, `linux.rs`, `windows.rs`; `broker.rs`, the broker and the core's end of it, and `unix.rs`, passing descriptors on Unix. `native/core/build.rs` (section 7.4). |

## 1. What is wanted

Every helper of the trusted core, on every platform, runs with only what it is for: a decoder with its pipes alone; a fetcher and the phone keyboard with the network and the name resolver; the interface with the window system, fonts, text input, the pasteboard and the Dock, and no network; the sound helper with the sound output alone (decision 55); and the trusted core with its settings folder alone (decision 56); the updater with the update host's one port and its staging folder (decision 73, `installation-and-updates.md` section 8.1). The sandbox must hold, whole, on every machine the browser runs on, and say what it applied; a machine that cannot give it all does not run the browser (decision 58, section 8). Decision 52 had asked instead that it run anywhere without an installer step, root or a recent kernel, with less where it must.

The helpers only ever receive: everything they work on arrives over their pipe from the trusted core, which is why their sandboxes deny and never broker. That is the property that keeps them small. The trusted core is the one process that needs things done for it, starting programs, reading a file channel and opening the web browser, and a broker does those (section 7).

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

**macOS** was done on 21 October 2026 with `sandbox-exec` profiles from "deny everything"; `security-analysis.md` section 13 records how. macOS refuses a nested `sandbox-exec`, so a confined trusted core cannot start its helpers itself; the broker (section 7) starts them, each under its own profile.

### 2.1 Library or not

Linux is raw system calls either way. With the crates `landlock` (by the Landlock kernel maintainer) and `seccompiler` (the rust-vmm project's filter builder), both already approved in the engine, the browser's own unsafe code on Linux is two `libc` calls. Hand-written, it is a new crate with unsafe system calls and a hand-assembled BPF program, where a mistake kills helpers silently or lets a call through. The crates were chosen. Windows needs no new library, only a third crate permitted unsafe code.

## 3. What was built

One crate, `confine`, so the trusted core keeps no platform code:

- `confine::spawn(Spawn { program, arguments, role, environment, local_network, writable })` starts a helper confined as the platform's parent side allows, with its pipes, and an environment of exactly what the core passes (nothing for a decoder or fetcher; the window system's variables for the interface). It returns a `Process` with a `Report` of what confined it.
- `confine::apply(role, writable)` is the first call in each helper's `main`, and in the trusted core's, before any thread: on Linux it applies, in order, no-new-privileges, an empty network namespace for the roles without the network, Landlock with the directories the role may read, and the seccomp deny-list. Since decision 58 each is required: the first that cannot be applied ends the process with its reason, and the parent says so. On macOS and Windows it reports that the broker confined it.
- Each helper prints what confined it, once, at start, and the core prints what it applied; the phone keyboard prints nothing of it, because that process prints one thing only (REQ-INT-137).

The seccomp deny-list: starting programs (`execve`, `execveat`, `fork`, `vfork`, `clone` without `CLONE_THREAD`, `clone3` answered "not implemented" so the C library falls back to `clone`), tracing and reaching into processes (`ptrace`, `process_vm_*`, `kcmp`), mounting and namespaces after our own (`mount`, `umount2`, `pivot_root`, `chroot`, `unshare`, `setns`), kernel interfaces a helper has no use for (`io_uring_*`, `bpf`, `keyctl`, `add_key`, `request_key`, `userfaultfd`, `perf_event_open`, `kexec_*`, `reboot`, `swapon`, `swapoff`, module loading, `acct`, clock and host-name setting, `open_by_handle_at`, `fanotify_init`, and on x86-64 `iopl` and `ioperm`), and sockets: none for a decoder; for a fetcher, the phone keyboard and the updater only `AF_INET`, `AF_INET6` and `AF_NETLINK` with the routing protocol alone (the resolver asks which address families exist), and no `AF_UNIX` since decision 86 (section 9.1); for the interface only `AF_UNIX` (the display and the input method); for the sound helper only `AF_UNIX` (the sound server); for the trusted core none, as for a decoder (its broker's socket is inherited, not made); `listen` and `accept4` for none; and `pidfd_getfd` for every role (decision 86).

Windows: the decoder in a less privileged AppContainer with no capabilities, the fetcher and keyboard in an AppContainer with the internet client capability and, when the user has allowed local addresses, the private network one; every helper in a job object with kill-on-close and a limit of one process, with every mitigation policy above, no child processes, and for the decoder and fetcher the Win32k filter and the UI restrictions (no desktop, clipboard or handles); the interface with a restricted token at low integrity instead of a container, since it needs the desktop; the sound helper in an AppContainer with no capabilities (the audio service admits containers); the trusted core in a less privileged AppContainer with the Win32k filter and no capabilities, its settings folder granted to its container (section 7). Only the two pipe handles are inherited. An AppContainer can read only what grants read to `ALL APPLICATION PACKAGES`: an installation under Program Files does, a folder of the user's does not. The installer grants it on `versions\` of the per-user installation (decision 78). Decision 52 started a helper with the restricted token alone when its container failed; since decision 58 it is not started.

## 4. Measured

- **macOS**, 21 October 2026: the four helpers under their profiles; the browser fetches, decodes, draws and opens the sound device; idle costs unchanged.
- **Linux**, 22 October 2026, Ubuntu 26.04 on `foundintelquiet4060` (kernel 7.0, Landlock in the LSM list, unprivileged user namespaces allowed): the crate's probe tests pass (a decoder can open no file, start no program, connect nowhere and write nothing; a fetcher can start no program and write nothing, and has the network); the browser under Xvfb with the terminal channel reports the decoder "network namespace, Landlock, seccomp", the fetcher "Landlock, seccomp", the interface "network namespace, Landlock, seccomp", fetches and draws (47 frames in 20 s). *Since decision 58* a machine without Landlock 5 or user namespaces does not run the browser (section 8.2).
- **Windows**, 22 October 2026, Windows 11 in the virtual machine on `foundintelquiet4060`: the workspace builds and every test passes there; in the logged-in desktop session the browser runs with every layer on: the interface at low integrity in its job with the mitigation policies drew the channel's pages and opened the sound device, the decoder ran in a less privileged AppContainer. Started from an `ssh` session instead (session 0, no window station), the interface fails at `CoInitialize` with "access denied" at low integrity and the sound device is refused at medium: a test of the Windows sandbox must run in the desktop session (a one-off interactive scheduled task does it; `DEVELOPMENT.md` section 12.1). The network role reaches its relay from inside the sandbox: `the_keyboard_reaches_its_relay_confined` (`core/tests/confined_keyboard.rs`, needs the internet) passed in the virtual machine in both sessions, in the AppContainer and, with `NEOTELETEXT_CONFINE_NO_CONTAINER` forcing the fallback, under the restricted token at low integrity (the fallback and the variable were removed by decision 58); and on macOS under the profile.

## 5. Decision 52 (for the owner to confirm)

Decided on 22 October 2026, on the owner's instruction, after the options of 21 October 2026.
1. Every helper is confined on every platform by that platform's own means, from "deny everything", with the layered, degrade-and-report design the two browsers converged on: the one layer every machine has is required (seccomp on Linux; the token and job on Windows; the `sandbox-exec` profile on macOS), the stronger optional layers are applied when the machine allows and reported.
2. On Linux the crates `landlock` and `seccompiler`, with `libc`, as the engine's own `neottx` uses (its decision E.33); the deny-list rather than an allow-list of system calls, so no C library or kernel release can stop a helper from running.
3. On Windows the Win32 calls in `windows-sys`, already a dependency; `confine` is the third crate permitted unsafe code (requirement 10.12 amended), on its Linux and Windows modules only, each block with its reason.
4. A helper confines itself first thing, before any thread, where the platform makes a process confine itself (Linux); the trusted core confines it at its start where the parent does (macOS, Windows).
5. The trusted core on macOS stays unconfined until the App Sandbox at packaging (requirement 3.8); macOS refuses a nested `sandbox-exec`. *Amended by decision 56:* the trusted core is confined on every platform now, behind a broker (section 7).

*Item 1's "the stronger optional layers are applied when the machine allows and reported" rescinded by decision 58* (section 8): every layer is required.

*Reason:* security first (decision 4); a sandbox that depends on user namespaces fails on the machines users have, as Chromium's and Electron's did on Ubuntu 24.04; the crates keep the BPF program, where a mistake hides, with the people who run it in production.

- **Linux, sound and Wayland**, 22 October 2026, on the same host: with the sound server reachable (`XDG_RUNTIME_DIR` set), the confined interface opened the device through PipeWire; under a headless Wayland compositor (`sway`) the browser ran with every helper confined, 47 frames in 18 s, fetch and push connected. The crate's tests there pass (6, with the environment and probe tests).

## 6. What building changed

1. **The user namespace maps the user to themselves.** Unmapped, a process in a new user namespace is "nobody" to every other process, and PipeWire, which checks the peer's credentials, dropped the interface's connection; Wayland and X11 did not care. After `unshare`, the helper writes `deny` to `setgroups` and its own uid and gid to `uid_map` and `gid_map`, as `user_namespaces(7)` allows an unprivileged process to do.
2. **An AppContainer needs the user's profile variables** (`USERPROFILE`, `LOCALAPPDATA`, `APPDATA` and their kin) in the child's environment, or `CreateProcess` fails with "environment option not found"; the core passes them to every Windows helper.
3. **A Windows helper with a window must be started in the desktop session.** From an `ssh` session (session 0, no window station) the interface dies at `CoInitialize` at low integrity; the test runs through an interactive scheduled task.
4. **The interface needs the window manager's service on macOS.** On macOS 15.6 a window made without `com.apple.windowmanager.server` is made and drawn but never put on the screen: the browser showed its icon in the Dock and no window. Found on 23 October 2026 by allowing every service lookup (the window appeared) and reading the sandbox's denials; the interface's profile now allows it, and `profiles_start_from_nothing_and_give_the_network_only_to_fetchers` fails without it. Whether the window is on the screen is not tested automatically: it needs a desktop session.
5. Found on the way: `rsync` does not see a re-vendored crate, because `cargo vendor` stamps every file with the same date, so the Windows export is sent with `--checksum` and the virtual machine's vendor folder removed before its copy (`DEVELOPMENT.md` section 12.1).

## 7. The broker: the trusted core confined (decision 56)

### 7.1 Why a broker

The trusted core parses what the decoders and the interface send, holds the settings and decides every link, so a flaw in it is the browser's worst case; decision 52 left it unconfined because it starts the helpers, and on macOS a sandboxed process cannot start another under a different profile. The App Sandbox with XPC services was weighed and refused: XPC starts one instance of a service per application, not one decoder per channel, without private API, and a helper the core started itself would inherit the core's sandbox, wider than the decoder's. A broker keeps one process per channel and each helper's own sandbox, on every platform alike.

### 7.2 The shape

`neoteletext`, the program the user starts, is the broker, and the one process not confined. It does nothing but:
1. Start `neoteletext --core` confined, with the user's arguments, an environment of only what the core needs (`confine::environment(Role::Core)`), and one channel to it.
2. Serve the core's requests until the core ends, then end every helper still alive and end as the core did.

The core's requests:

| Request | What the broker does | Bound or rule |
|---|---|---|
| Spawn a program | Starts one of the browser's own helpers (`Program`: decoder, fetcher, interface, phone keyboard, sound helper, updater), beside itself, confined by the role that program has; passes the core its two pipes | The core names a program, never a path or a role; at most `MAX_HELPERS` (512) alive; at most `MAX_ARGUMENTS` (64) arguments of at most `MAX_TEXT` (4,096) bytes, which each helper checks as before |
| Kill, wait | Ends or waits for a helper the core started, by the number the broker gave it | Only the broker's own numbers |
| Read | Reads a file channel's bytes by the browser's channel rules (`source.rs`) and returns them | Refused under the user's secret folders (`.ssh`, `.gnupg`, `.aws`, the keychains and the like), however the path is named; at most 64 MB and 4,096 files (`MAX_REPLY`) |
| Open the web | Hands a checked address to the system's browser through `web-opener` | `web-opener`'s checks, as before (requirement 17.9) |
| Local time | Answers the time and the local zone's offset, read by the system's own rules (`history-and-bookmarks.md` section 14 item 1) | No payload |
| Cookie key | Reads the cookie file's key from the platform's secret store (`cookies.md` section 10) | No payload |
| Healthy | Tells the launcher the version is healthy (`installation-and-updates.md` section 3.2) | No payload |
| Update state | Returns the launcher's state file, or refuses when no launcher started the broker | At most 4 KiB, read strictly |

The channel is a local-domain socket pair on macOS and Linux, the core's standard input, which carries each helper's pipes as descriptors (`SCM_RIGHTS`, `confine/src/unix.rs`). On Windows it is two pipes, and the broker duplicates the helper's pipe handles into the core's process with `DuplicateHandle` and sends their values. Every message is a kind byte, a length and a payload, read within its bound; a request the broker cannot parse ends the core. The broker's decoders are fuzzed (`broker_requests`, `broker_replies`).

A compromised core gains nothing it did not have: it can start only the browser's helpers, each confined as always; read only what a file channel could already name, minus the secret folders; and open only what a link could already open.

### 7.3 The trusted core's confinement

| Platform | How | What the core keeps |
|---|---|---|
| macOS | The broker starts it under a `sandbox-exec` profile from "deny everything": `(deny network*)`, no services looked up, the system reads every process needs | Read and write in its settings folder |
| Linux | It confines itself first thing, before any thread, as the helpers do: no-new-privileges, an empty network namespace, Landlock, the seccomp deny-list with no socket family | Read and write in its settings folder; its inherited socket to the broker |
| Windows | The broker starts it in a less privileged AppContainer (`NeoTeletext.Core`) with no capabilities, in a job with every mitigation policy, no child processes and the Win32k filter; the settings folder is granted to the container's identity before the start. A container that cannot start means no core and no browser (decision 58) | Read and write in its settings folder |

The sound helper (decision 55) runs as its own role: on macOS a profile with the audio services and nothing else; on Linux `AF_UNIX` for the sound server and Landlock's read of the sound devices; on Windows an AppContainer with no capabilities.

### 7.4 What building changed

1. **A process denied Win32k cannot load `shell32.dll`.** On Windows `neoteletext.exe` is both the broker and the core, and the broker opens web pages through `ShellExecuteW`. Imported directly, `shell32.dll` loads `user32.dll` and `gdi32.dll`, whose start fails under the Win32k filter: the core ended before its first line with `STATUS_DLL_INIT_FAILED` (0xC0000142). `core/build.rs` delay-loads `shell32.dll` in that one program, so it loads at the broker's first hand-off and never in the core. `the_trusted_core_starts_confined_and_reaches_its_own_code` (`core/tests/confined_core.rs`) failed with exactly that exit before and passes after, in the virtual machine.
2. **The program folder needs no grant beyond `ALL APPLICATION PACKAGES`.** A less privileged AppContainer does not match that group, but the core and decoder still started and ran with only it granted (measured in the virtual machine, `ALL RESTRICTED APPLICATION PACKAGES` removed): the program's image is opened by the parent that creates the process, and the system's libraries grant both groups. The virtual machine's build script grants `ALL APPLICATION PACKAGES` read on its target folder, so the tests can start confined processes from it.
3. **The interface no longer needs ALSA's library or fontconfig's.** With sound in its own helper and fontconfig loaded at run time (`fontique`'s `fontconfig-dlopen`; the interface draws with its own fonts), `readelf -d` on Linux shows the interface needing only the C library, `libm` and `libgcc_s`; only `neoteletext-sound` needs `libasound.so.2`.

### 7.5 Measured, 22 October 2026

- **macOS:** the release browser with the reference channel: the broker started the core under `sandbox-exec`; every helper is the broker's child, under its own profile; typed 700 and Space, the podcast's stream reached the sound helper, which played it (12 peaks in 6 s) and said so, and the tab showed its speaker. A file channel read through the broker drew its pages. `a_tone_plays_through_the_confined_sound_helper` (`sound/tests/confined.rs`, needs an output device) passes.
- **Linux**, `foundintelquiet4060`: under X11 with PipeWire and under a headless `sway`, the core reports "network namespace, Landlock, seccomp" and every helper its own layers, and the pages draw (43 and 46 frames). With `libasound.so.2` and `libfontconfig.so.1` hidden by bind mounts in a private mount namespace, the browser starts and draws (42 frames); the sound helper fails to load and the core says "no sound" and carries on (requirement 16.6). The confined tone test passes through PipeWire (its sink there is the null sink: the host has no sound card).
- **Windows**, the virtual machine: the workspace builds and every test passes; with the program folder granting `ALL APPLICATION PACKAGES` alone, in the desktop session the core runs in its less privileged AppContainer, the decoder in its own, the sound helper in an AppContainer, the interface at low integrity, and the file channel read through the broker draws (17 frames in 15 s). The confined tone test passes in the desktop session.

## 8. Decision 58: every layer required

The owner decided on 22 October 2026: **security is paramount; there can be no weakening or downgrading of security.** It rescinds decision 52's "degrade and report".

### 8.1 What changed

| Platform | Before (decision 52) | Now (decision 58) |
|---|---|---|
| Linux | seccomp required; the network namespace and Landlock applied when the machine allowed, reported when not; Landlock "the best the kernel offers", reported "in part" when the kernel enforced less; an architecture without a filter reported | The network namespace (for every role but the network's), Landlock at version 5 enforced whole, and seccomp each required; the first that cannot be applied ends the process with its reason |
| Windows | When a container could not start, the helper (and from decision 56 the trusted core) started with the restricted token alone; `NEOTELETEXT_CONFINE_NO_CONTAINER` forced that for testing | Every process but the interface runs in its AppContainer or not at all; the variable is gone |
| macOS | The profile required | Unchanged |
| Elsewhere | "Not confined", and run | Refused: no sandbox, no browser |

The report says only what is in force; there is nothing "without" any more.

Building it found one downgrade that had been reported as normal: under X11 the interface's Landlock was enforced only "in part" on a kernel that supports all of it, because the display's authority file, a file, was given a directory's read rights. A file is now given a file's rights only.

### 8.2 What a machine needs now

- **Linux:** kernel 6.10 or later with Landlock enabled (Landlock 5); unprivileged user namespaces allowed for the browser; x86-64, AArch64 or RISC-V 64 (the architectures `seccompiler` writes filters for). Ubuntu from 24.04 restricts unprivileged user namespaces by default (`kernel.apparmor_restrict_unprivileged_userns = 1`) unless an AppArmor profile allows them for the program, as Ubuntu's own `apparmor` package carries for Chrome and Firefox; the test host `foundintelquiet4060` runs with the restriction off (measured: `0`), which is why every run there had its namespaces. On a default Ubuntu 24.04 or later, the browser will not start until such a profile is installed, which needs root (section 8.3). Kernels before 6.10 (Ubuntu 24.04's first kernel is 6.8; Debian 12's 6.1; RHEL 9's 5.14) do not run it.
- **Windows:** the user's profile folders in each process's environment, which the broker passes (`confine::environment`). A program folder that grants containers nothing does not stop a start: measured on 22 October 2026, a decoder started in its less privileged container from the user's temporary folder. Whether every helper then works from such a folder was not run; Program Files grants `ALL APPLICATION PACKAGES` read in any case.
- **macOS:** unchanged.

### 8.3 For the owner

Decision 54 installs per user, with no administrator, on every desktop. On Linux that cannot install the AppArmor profile a default Ubuntu now needs, nor raise an old kernel. The choices are the owner's: a system-wide package on such distributions that installs the profile, or a documented requirement that the user or their administrator allow it. Neither weakens the sandbox. *Decided by decision 77:* a stated requirement, not a system-wide package; every Linux version carries `neoteletext.apparmor`, and `neoteletext --install` prints the two administrator commands that install it.

### 8.4 Tests

- `a_helper_without_its_namespace_does_not_run` (`confine/tests/probe.rs`): a decoder whose `unshare` is refused, as on a distribution that forbids user namespaces, must refuse to run. On the code before, it ran "without a network namespace".
- `a_file_to_read_keeps_landlock_whole`: the interface with a file to read gets every Landlock right enforced. On the code before, "Landlock (in part)".
- `a_helper_whose_container_cannot_be_made_is_not_started` (`confine/src/lib.rs`, Windows): a decoder whose container cannot be made, without the user's profile folders in its environment, must not start. On the code before, it started with the restricted token alone; and `a_helper_gets_only_the_environment_it_is_given` had been passing that way, without its container, unseen. It is now given a decoder's real environment.
- The probes now run as a program of their own, `confine-probe`, with one thread, as a helper is when it confines itself: a test harness's threads make the kernel refuse a new user namespace, which the code before had silently reported as missing, so its own probe tests had run without the namespace.

## 9. The security review of 24 October 2026

The review of 24 October 2026 (`neoteletext-common/reviews/2026-10-24-security/FIX-PLAN.md`, work package WP-1) found these gaps in the sandboxes; decisions 86 to 100 record what was chosen, while the owner was away, for the owner to confirm. Each fix has a regression test that fails on the code before it.

### 9.1 Linux: no helper reaches the session's sockets (decision 86, finding F-01)

Landlock up to version 6 controls making a named socket, not connecting to one, and the network namespace does not touch path sockets. So every helper allowed `AF_UNIX`, the fetcher, the phone keyboard, the updater, the interface and the sound helper, could `connect` to the session bus at `/run/user/<uid>/bus`, where systemd's `StartTransientUnit` runs any program as the user, unconfined; and to the system bus, `ssh-agent`, `gpg-agent` or a container engine's socket. Measured on `foundintelquiet4060` before the fix: every one of those roles connected to the real session bus and the system bus.

- **The fetcher, the phone keyboard and the updater** lose `AF_UNIX`: their families are `AF_INET`, `AF_INET6` and `AF_NETLINK`, and seccomp refuses a netlink socket of any protocol but routing (`NETLINK_ROUTE`), which the resolver uses to ask the host's address families. glibc's name-service modules that use a local socket (nscd, `nss-resolve`, `mymachines`) fail with EPERM, which glibc takes as "unavailable", and names are resolved by its `dns` module over UDP (to `127.0.0.53` under systemd-resolved).
- **The interface and the sound helper** keep `AF_UNIX` for the display and the sound server, and get a mount namespace, made with the user namespace they already make. Before Landlock, `runtime::private_runtime` makes every mount private, keeps a detached copy (`open_tree`) of each socket the role needs, covers `/run`, `/tmp` and the runtime folder (`XDG_RUNTIME_DIR`, wherever it is) with empty private `tmpfs` folders (64 KiB, 64 entries, mode 0700, no devices, set-user-ID or programs), and mounts each kept socket back where it was (`move_mount`). The interface keeps its Wayland socket (`WAYLAND_DISPLAY`, by default `wayland-0`), its X server's (`/tmp/.X11-unix/X<n>` for `DISPLAY=:n`) and the display's authority file where it lies in a covered folder; the sound helper PipeWire's `pipewire-0` and PulseAudio's `pulse/native` (or `PULSE_SERVER`'s local socket). A path that does not exist is not needed; a mount that fails stops the helper (decision 58). The report gains the layer "private /run and /tmp". Abstract sockets were already out of reach in the empty network namespace.
- **`DBUS_SESSION_BUS_ADDRESS`** leaves the interface's environment: nothing in it uses D-Bus.
- **seccomp refuses `pidfd_getfd`** in every role, which copies another process's descriptor.

Not taken: a filtering D-Bus proxy in front of the interface, a dependency for a need that does not exist. Tests (`confine/tests/probe.rs`, Linux): `a_network_helper_reaches_no_local_socket` (the fetcher's and updater's connections to a session bus, a display and a sound server in a runtime folder of the test's own, to the real session bus and to the system bus are refused, an audit netlink socket is refused, a routing one opened, and `localhost` resolves), `the_interface_reaches_its_display_and_no_bus`, `the_sound_helper_reaches_its_server_and_no_bus` (each reaches its own socket in that runtime folder, and finds no bus there or in `/run`), and `no_helper_takes_another_process_descriptor`. All four failed on the code before.

