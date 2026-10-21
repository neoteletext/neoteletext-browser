//! A helper's self-confinement on Linux, in the order that lets each
//! layer be set up before the next forbids setting things up:
//!
//! 1. No new privileges, so no program it might start could gain any.
//! 2. For a helper with no network, an empty network namespace, where the
//!    kernel lets an unprivileged process make one (Debian before 11 and
//!    Ubuntu from 24.04 may not, nor a Flatpak or a container): nothing to
//!    connect to at all.
//! 3. Landlock, on kernels from 5.13: the directories the helper may read
//!    (none for a decoder) and, on kernels from 6.7, no TCP for one with
//!    no network. The best the running kernel offers, and reported.
//! 4. A seccomp system-call filter, on every kernel since 3.5: no starting
//!    programs, no tracing, no mounting, no kernel interfaces a helper has
//!    no use for, and sockets only of the families its role needs. A
//!    denylist rather than an allowlist, so a new C library or kernel
//!    release cannot stop a helper from running; the layer every machine
//!    has is the one a helper refuses to run without, except where no
//!    filter exists for the architecture, which is reported.
//!
//! What is written here as Chromium and Firefox arrived at it: the
//! namespace layer is best effort because it is the one that fails on
//! users' machines; the filter is mandatory because it never does.

use landlock::{ABI, Access, AccessFs, AccessNet, CompatLevel, Compatible, PathBeneath, PathFd, Ruleset, RulesetAttr, RulesetCreatedAttr, RulesetStatus};
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

/// The socket families a role may open.
fn families(role: Role) -> &'static [u64] {
    match role {
        Role::Decoder => &[],
        // The resolver uses netlink to ask which address families the
        // host has, and local-domain sockets for the name service cache;
        // neither reaches beyond the host.
        Role::Network => &[libc::AF_INET as u64, libc::AF_INET6 as u64, libc::AF_NETLINK as u64, libc::AF_UNIX as u64],
        // The display, the sound server and the input method are
        // local-domain sockets.
        Role::Interface => &[libc::AF_UNIX as u64],
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
        Role::Decoder => Vec::new(),
        Role::Network => RESOLVER_PATHS.iter().map(PathBuf::from).collect(),
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

/// The newest Landlock the crate knows; the ruleset is made for it and
/// falls back to what the running kernel has.
const LANDLOCK_ABI: ABI = ABI::V5;

/// Landlock: the readable paths, the sound devices for the interface, and
/// no TCP for a role without the network. Returns whether the kernel
/// enforced all of it or, being older, part.
fn landlock(role: Role) -> Result<RulesetStatus, String> {
    let abi = LANDLOCK_ABI;
    let mut ruleset =
        Ruleset::default().set_compatibility(CompatLevel::BestEffort).handle_access(AccessFs::from_all(abi)).map_err(|error| error.to_string())?;
    if role != Role::Network {
        ruleset = ruleset.handle_access(AccessNet::from_all(abi)).map_err(|error| error.to_string())?;
    }
    let mut created = ruleset.create().map_err(|error| error.to_string())?;
    for path in readable(role) {
        let Ok(fd) = PathFd::new(&path) else { continue };
        created = created.add_rule(PathBeneath::new(fd, AccessFs::from_read(abi))).map_err(|error| error.to_string())?;
    }
    if role == Role::Interface
        && let Ok(fd) = PathFd::new(Path::new(SOUND_DEVICES))
    {
        created = created.add_rule(PathBeneath::new(fd, AccessFs::from_all(abi))).map_err(|error| error.to_string())?;
    }
    let status = created.restrict_self().map_err(|error| error.to_string())?;
    match status.ruleset {
        RulesetStatus::FullyEnforced | RulesetStatus::PartiallyEnforced => Ok(status.ruleset),
        RulesetStatus::NotEnforced => Err(String::from("the kernel has no Landlock (before 5.13, or built without it)")),
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

/// Confines the calling process as `role` says.
pub fn apply(role: Role) -> Result<Report, String> {
    let mut report = Report::default();
    no_new_privileges().map_err(|error| format!("cannot refuse new privileges: {error}"))?;
    if role != Role::Network {
        match network_namespace() {
            Ok(()) => report.layers.push(String::from("network namespace")),
            Err(error) => report.missing.push(format!("a network namespace ({error})")),
        }
    }
    match landlock(role) {
        Ok(RulesetStatus::FullyEnforced) => report.layers.push(String::from("Landlock")),
        Ok(_) => report.layers.push(String::from("Landlock (in part)")),
        Err(error) => report.missing.push(format!("Landlock ({error})")),
    }
    match architecture() {
        Some(arch) => {
            seccomp(role, arch).map_err(|error| format!("cannot apply the system-call filter: {error}"))?;
            report.layers.push(String::from("seccomp"));
        }
        None => report.missing.push(String::from("a system-call filter (none exists for this architecture)")),
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::{Command, Stdio};

    /// What a probe tries once confined, as a child of the test.
    const PROBE: &str = "NEOTELETEXT_CONFINE_PROBE";

    /// Runs this test binary again as a probe for `role`, which confines
    /// itself and reports what it can still do.
    fn probe(role: &str) -> String {
        let program = std::env::current_exe().unwrap();
        let output = Command::new(program)
            .arg("--exact")
            .arg("linux::tests::probe_entry")
            .arg("--nocapture")
            .env(PROBE, role)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .unwrap();
        let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
        text.push_str(&String::from_utf8_lossy(&output.stderr));
        text
    }

    /// The probe: run by `probe`, never on its own. It confines itself as
    /// the role in the environment says, then tries what the role forbids.
    #[test]
    fn probe_entry() {
        let Ok(role) = std::env::var(PROBE) else { return };
        let role = match role.as_str() {
            "decoder" => Role::Decoder,
            "network" => Role::Network,
            _ => Role::Interface,
        };
        let report = apply(role).unwrap();
        let file = std::fs::File::open("/etc/hostname").map_or("refused", |_| "opened");
        let program = Command::new("/bin/true").status().map_or("refused", |_| "started");
        let tcp = std::net::TcpStream::connect_timeout(&"127.0.0.1:9".parse().unwrap(), std::time::Duration::from_millis(200))
            .map_or_else(|error| if error.kind() == std::io::ErrorKind::PermissionDenied { "refused" } else { "no listener" }, |_| "connected");
        let write = std::fs::write("/tmp/neoteletext-confine-probe", b"x").map_or("refused", |()| "wrote");
        println!("PROBE report={report}; file={file}; program={program}; tcp={tcp}; write={write}");
    }

    /// Requirement 3.4: a decoder can open no file, start no program and
    /// make no connection.
    #[test]
    fn a_decoder_can_do_nothing_but_compute() {
        let out = probe("decoder");
        let line = out.lines().find(|line| line.starts_with("PROBE")).unwrap_or_else(|| panic!("no probe line in {out}"));
        assert!(line.contains("file=refused"), "{line}");
        assert!(line.contains("program=refused"), "{line}");
        assert!(line.contains("tcp=refused"), "{line}");
        assert!(line.contains("write=refused"), "{line}");
        assert!(line.contains("seccomp"), "{line}");
    }

    /// Requirement 3.3: a fetcher may connect but not open files (beyond
    /// the resolver's) or start programs.
    #[test]
    fn a_fetcher_has_the_network_and_nothing_else() {
        let out = probe("network");
        let line = out.lines().find(|line| line.starts_with("PROBE")).unwrap_or_else(|| panic!("no probe line in {out}"));
        // /etc is readable for the resolver; a write anywhere is not.
        assert!(line.contains("program=refused"), "{line}");
        assert!(line.contains("write=refused"), "{line}");
        assert!(line.contains("tcp=no listener") || line.contains("tcp=connected"), "the network is open: {line}");
    }
}
