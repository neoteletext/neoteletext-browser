//! A helper's self-confinement on Linux, in the order that lets each
//! layer be set up before the next forbids setting things up:
//!
//! 1. No new privileges, so no program it might start could gain any.
//! 2. For a helper with no network, an empty network namespace: nothing to
//!    connect to at all.
//! 3. Landlock, all of what [`LANDLOCK_ABI`] can say: the directories the
//!    helper may read (none for a decoder) and no TCP for one with no
//!    network.
//! 4. A seccomp system-call filter: no starting programs, no tracing, no
//!    mounting, no kernel interfaces a helper has no use for, and sockets
//!    only of the families its role needs. A denylist rather than an
//!    allowlist, so a new C library or kernel release cannot stop a helper
//!    from running.
//!
//! Every layer is required (decision 58: security is never weakened or
//! downgraded): a machine that cannot give one, such as a kernel without
//! that Landlock, a distribution that forbids unprivileged user
//! namespaces, or an architecture with no filter, gets an error and no
//! process, never a process with less.

use landlock::{
    ABI, Access, AccessFs, AccessNet, CompatLevel, Compatible, NetPort, PathBeneath, PathFd, Ruleset, RulesetAttr, RulesetCreatedAttr, RulesetStatus,
};
use seccompiler::{BpfProgram, SeccompAction, SeccompCmpArgLen, SeccompCmpOp, SeccompCondition, SeccompFilter, SeccompRule, TargetArch};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::{Report, Role};

/// Where the resolver's files and the C library's name services live;
/// a fetcher may read them (as the engine's `neottx` lists them).
const RESOLVER_PATHS: &[&str] = &["/etc", "/usr/lib", "/lib", "/usr/lib64", "/lib64", "/run/systemd/resolve", "/run/resolvconf"];
/// What the window system, fonts, the input method and the sound server
/// keep that the interface reads.
const INTERFACE_PATHS: &[&str] = &["/usr", "/etc", "/lib", "/lib64", "/opt", "/var/lib", "/var/cache/fontconfig", "/run", "/tmp", "/dev", "/proc", "/sys"];
/// What the interface may read under the user's home: fonts and their
/// configuration, and the display's authority file.
const INTERFACE_HOME_PATHS: &[&str] = &[".fonts", ".local/share/fonts", ".config/fontconfig", ".cache/fontconfig", ".Xauthority", ".config/gtk-3.0", ".icons"];
/// The sound devices, for ALSA's direct output.
const SOUND_DEVICES: &str = "/dev/snd";
/// What the sound helper reads: the sound libraries' plug-ins and
/// configuration, and the sound server's sockets.
const SOUND_PATHS: &[&str] = &["/usr", "/etc", "/lib", "/lib64", "/opt", "/var/lib", "/run", "/proc", "/sys", "/dev"];
/// What the sound helper may read under the user's home: ALSA's and the
/// sound server's configuration.
const SOUND_HOME_PATHS: &[&str] = &[".asoundrc", ".config/pulse", ".config/pipewire"];

/// The socket families a role may open.
fn families(role: Role) -> &'static [u64] {
    match role {
        Role::Decoder | Role::Core => &[],
        // The resolver uses netlink to ask which address families the
        // host has, and local-domain sockets for the name service cache;
        // neither reaches beyond the host.
        Role::Network | Role::Updater { .. } => &[libc::AF_INET as u64, libc::AF_INET6 as u64, libc::AF_NETLINK as u64, libc::AF_UNIX as u64],
        // The display, the sound server and the input method are
        // local-domain sockets.
        Role::Interface | Role::Sound => &[libc::AF_UNIX as u64],
    }
}

/// No new privileges: nothing started from here on can gain any, and
/// Landlock and seccomp require it.
fn no_new_privileges() -> Result<(), String> {
    // SAFETY: prctl with PR_SET_NO_NEW_PRIVS takes integer arguments only
    // and touches no memory of ours.
    #[allow(unsafe_code, reason = "a C library call with integer arguments")]
    let result = unsafe { libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) };
    if result == 0 { Ok(()) } else { Err(std::io::Error::last_os_error().to_string()) }
}

/// An empty network namespace, if the kernel lets an unprivileged process
/// make one: the process is then alone on a network with nothing on it.
/// The user namespace that makes that possible maps the user to
/// themselves, so the sound server and the display still see who is
/// connecting; unmapped, the process would be "nobody" to them, and
/// `PipeWire` drops such a client.
fn network_namespace() -> Result<(), String> {
    // SAFETY: these take no pointers and read the process's own ids.
    #[allow(unsafe_code, reason = "C library calls with integer arguments and results")]
    let (uid, gid) = unsafe { (libc::getuid(), libc::getgid()) };
    // SAFETY: unshare takes flags only; the process has one thread here,
    // which CLONE_NEWUSER requires.
    #[allow(unsafe_code, reason = "a C library call with integer arguments")]
    let result = unsafe { libc::unshare(libc::CLONE_NEWUSER | libc::CLONE_NEWNET) };
    if result != 0 {
        return Err(std::io::Error::last_os_error().to_string());
    }
    // user_namespaces(7): setgroups must be denied before an unprivileged
    // process may write its own group map.
    std::fs::write("/proc/self/setgroups", "deny").map_err(|error| format!("setgroups: {error}"))?;
    std::fs::write("/proc/self/uid_map", format!("{uid} {uid} 1")).map_err(|error| format!("uid_map: {error}"))?;
    std::fs::write("/proc/self/gid_map", format!("{gid} {gid} 1")).map_err(|error| format!("gid_map: {error}"))?;
    Ok(())
}

/// The paths a role may read, of those that exist.
fn readable(role: Role) -> Vec<PathBuf> {
    let mut paths: Vec<PathBuf> = match role {
        Role::Decoder | Role::Core => Vec::new(),
        Role::Network | Role::Updater { .. } => RESOLVER_PATHS.iter().map(PathBuf::from).collect(),
        Role::Sound => {
            let mut paths: Vec<PathBuf> = SOUND_PATHS.iter().map(PathBuf::from).collect();
            if let Some(home) = std::env::var_os("HOME").map(PathBuf::from).filter(|home| home.is_absolute()) {
                paths.extend(SOUND_HOME_PATHS.iter().map(|path| home.join(path)));
            }
            if let Some(runtime) = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).filter(|path| path.is_absolute()) {
                paths.push(runtime);
            }
            paths
        }
        Role::Interface => {
            let mut paths: Vec<PathBuf> = INTERFACE_PATHS.iter().map(PathBuf::from).collect();
            if let Some(home) = std::env::var_os("HOME").map(PathBuf::from).filter(|home| home.is_absolute()) {
                paths.extend(INTERFACE_HOME_PATHS.iter().map(|path| home.join(path)));
            }
            if let Some(runtime) = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).filter(|path| path.is_absolute()) {
                paths.push(runtime);
            }
            if let Some(authority) = std::env::var_os("XAUTHORITY").map(PathBuf::from).filter(|path| path.is_absolute()) {
                paths.push(authority);
            }
            paths
        }
    };
    paths.retain(|path| path.exists());
    paths
}

/// The Landlock the rules are written for (kernel 6.10 and later); a
/// kernel that cannot enforce all of it is refused (decision 58).
const LANDLOCK_ABI: ABI = ABI::V5;

/// Read access to `path`: a file cannot be given a directory's rights, so
/// a file is given only the read rights a file has. Asked for on a file,
/// a directory's rights would leave the ruleset enforced only in part.
fn read_access(path: &Path, abi: ABI) -> landlock::BitFlags<AccessFs> {
    if path.is_dir() { AccessFs::from_read(abi) } else { AccessFs::from_read(abi) & AccessFs::from_file(abi) }
}

/// Landlock: the readable paths, the sound devices for the sound helper,
/// the core's settings folder or the updater's staging folder written, no
/// TCP for a role without the network and one port for the updater, every
/// one of them enforced or none (decision 58).
fn landlock(role: Role, writable: Option<&Path>) -> Result<(), String> {
    let abi = LANDLOCK_ABI;
    let mut ruleset =
        Ruleset::default().set_compatibility(CompatLevel::HardRequirement).handle_access(AccessFs::from_all(abi)).map_err(|error| error.to_string())?;
    if role != Role::Network {
        ruleset = ruleset.handle_access(AccessNet::from_all(abi)).map_err(|error| error.to_string())?;
    }
    let mut created = ruleset.create().map_err(|error| error.to_string())?;
    for path in readable(role) {
        let Ok(fd) = PathFd::new(&path) else { continue };
        created = created.add_rule(PathBeneath::new(fd, read_access(&path, abi))).map_err(|error| error.to_string())?;
    }
    let written = match role {
        Role::Sound => Some(Path::new(SOUND_DEVICES)),
        Role::Core | Role::Updater { .. } => writable,
        Role::Decoder | Role::Network | Role::Interface => None,
    };
    if let Some(folder) = written
        && let Ok(fd) = PathFd::new(folder)
    {
        created = created.add_rule(PathBeneath::new(fd, AccessFs::from_all(abi))).map_err(|error| error.to_string())?;
    }
    // The updater connects to its one port, and binds none (section 8.1
    // of the installation design).
    if let Role::Updater { port } = role {
        created = created.add_rule(NetPort::new(port, AccessNet::ConnectTcp)).map_err(|error| error.to_string())?;
    }
    let status = created.restrict_self().map_err(|error| error.to_string())?;
    enforced(&status.ruleset)
}

/// Only a ruleset the kernel enforces whole is a sandbox (decision 58).
fn enforced(status: &RulesetStatus) -> Result<(), String> {
    match status {
        RulesetStatus::FullyEnforced => Ok(()),
        RulesetStatus::PartiallyEnforced => Err(String::from("this kernel enforces only part of the rules (Landlock 5, Linux 6.10, is needed)")),
        RulesetStatus::NotEnforced => Err(String::from("this kernel has no Landlock (Landlock 5, Linux 6.10, is needed)")),
    }
}

/// The architectures a filter can be written for.
fn architecture() -> Option<TargetArch> {
    if cfg!(target_arch = "x86_64") {
        Some(TargetArch::x86_64)
    } else if cfg!(target_arch = "aarch64") {
        Some(TargetArch::aarch64)
    } else if cfg!(target_arch = "riscv64") {
        Some(TargetArch::riscv64)
    } else {
        None
    }
}

/// A system call number as a rule's key.
#[allow(clippy::useless_conversion, reason = "c_long is 64 bits here and 32 bits on 32-bit targets")]
fn number(syscall: libc::c_long) -> i64 {
    i64::from(syscall)
}

/// A rule that matches always.
fn always() -> Vec<SeccompRule> {
    Vec::new()
}

/// The seccomp filter for `role`: what is denied, with EPERM.
fn seccomp(role: Role, arch: TargetArch) -> Result<(), String> {
    let mut rules: BTreeMap<i64, Vec<SeccompRule>> = BTreeMap::new();
    // Starting programs, tracing and reaching into other processes.
    let denied: &[libc::c_long] = &[
        libc::SYS_execve,
        libc::SYS_execveat,
        libc::SYS_ptrace,
        libc::SYS_process_vm_readv,
        libc::SYS_process_vm_writev,
        libc::SYS_kcmp,
        // Mounting and namespaces, after our own.
        libc::SYS_mount,
        libc::SYS_umount2,
        libc::SYS_pivot_root,
        libc::SYS_chroot,
        libc::SYS_unshare,
        libc::SYS_setns,
        // Kernel interfaces a helper has no use for.
        libc::SYS_io_uring_setup,
        libc::SYS_io_uring_enter,
        libc::SYS_io_uring_register,
        libc::SYS_bpf,
        libc::SYS_keyctl,
        libc::SYS_add_key,
        libc::SYS_request_key,
        libc::SYS_userfaultfd,
        libc::SYS_perf_event_open,
        libc::SYS_kexec_load,
        libc::SYS_kexec_file_load,
        libc::SYS_reboot,
        libc::SYS_swapon,
        libc::SYS_swapoff,
        libc::SYS_init_module,
        libc::SYS_finit_module,
        libc::SYS_delete_module,
        libc::SYS_acct,
        libc::SYS_settimeofday,
        libc::SYS_clock_settime,
        libc::SYS_adjtimex,
        libc::SYS_sethostname,
        libc::SYS_setdomainname,
        libc::SYS_open_by_handle_at,
        libc::SYS_fanotify_init,
    ];
    for &syscall in denied {
        rules.insert(number(syscall), always());
    }
    #[cfg(target_arch = "x86_64")]
    for syscall in [libc::SYS_fork, libc::SYS_vfork, libc::SYS_iopl, libc::SYS_ioperm] {
        rules.insert(number(syscall), always());
    }
    // A new process, but not a new thread: clone without CLONE_THREAD.
    let thread_flag = u64::try_from(libc::CLONE_THREAD).map_err(|_| "CLONE_THREAD")?;
    let not_a_thread =
        SeccompRule::new(vec![SeccompCondition::new(0, SeccompCmpArgLen::Qword, SeccompCmpOp::MaskedEq(thread_flag), 0).map_err(|error| error.to_string())?])
            .map_err(|error| error.to_string())?;
    rules.insert(number(libc::SYS_clone), vec![not_a_thread]);
    // Sockets: none for a decoder; for the others, a socket of a family
    // the role has no use for is refused (every condition of one rule
    // must hold, so one rule says "none of these families").
    let allowed = families(role);
    if allowed.is_empty() {
        for syscall in [libc::SYS_socket, libc::SYS_socketpair, libc::SYS_connect, libc::SYS_bind, libc::SYS_listen, libc::SYS_accept4] {
            rules.insert(number(syscall), always());
        }
    } else {
        let conditions = allowed
            .iter()
            .map(|&family| SeccompCondition::new(0, SeccompCmpArgLen::Dword, SeccompCmpOp::Ne, family))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())?;
        let other_family = SeccompRule::new(conditions).map_err(|error| error.to_string())?;
        rules.insert(number(libc::SYS_socket), vec![other_family]);
        rules.insert(number(libc::SYS_listen), always());
        rules.insert(number(libc::SYS_accept4), always());
    }
    let refused = u32::try_from(libc::EPERM).map_err(|_| "EPERM")?;
    let filter = SeccompFilter::new(rules, SeccompAction::Allow, SeccompAction::Errno(refused), arch).map_err(|error| error.to_string())?;
    let program: BpfProgram = filter.try_into().map_err(|error: seccompiler::BackendError| error.to_string())?;
    seccompiler::apply_filter(&program).map_err(|error| error.to_string())?;
    // clone3's flags are in memory the filter cannot read; "not
    // implemented" makes the C library fall back to clone, which the rule
    // above inspects.
    let not_implemented = u32::try_from(libc::ENOSYS).map_err(|_| "ENOSYS")?;
    let mut clone3 = BTreeMap::new();
    clone3.insert(number(libc::SYS_clone3), always());
    let filter = SeccompFilter::new(clone3, SeccompAction::Allow, SeccompAction::Errno(not_implemented), arch).map_err(|error| error.to_string())?;
    let program: BpfProgram = filter.try_into().map_err(|error: seccompiler::BackendError| error.to_string())?;
    seccompiler::apply_filter(&program).map_err(|error| error.to_string())
}

/// Confines the calling process as `role` says, every layer or none: the
/// first that cannot be applied is the error, and the process must end
/// (decision 58).
pub fn apply(role: Role, writable: Option<&Path>) -> Result<Report, String> {
    let mut report = Report::default();
    no_new_privileges().map_err(|error| format!("cannot refuse new privileges: {error}"))?;
    if !matches!(role, Role::Network | Role::Updater { .. }) {
        network_namespace().map_err(|error| format!("cannot make an empty network namespace: {error}"))?;
        report.layers.push(String::from("network namespace"));
    }
    landlock(role, writable).map_err(|error| format!("cannot apply Landlock: {error}"))?;
    report.layers.push(String::from("Landlock"));
    let arch = architecture().ok_or("no system-call filter exists for this architecture")?;
    seccomp(role, arch).map_err(|error| format!("cannot apply the system-call filter: {error}"))?;
    report.layers.push(String::from("seccomp"));
    Ok(report)
}
