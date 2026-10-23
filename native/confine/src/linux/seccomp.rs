//! The seccomp system-call filter: a denylist, with sockets only of the
//! families a role needs, and on x86-64 nothing by the x32 ABI's numbers.

use seccompiler::{BpfProgram, SeccompAction, SeccompCmpArgLen, SeccompCmpOp, SeccompCondition, SeccompFilter, SeccompRule, TargetArch};
use std::collections::BTreeMap;

use super::paths::families;
use crate::Role;

/// The architectures a filter can be written for.
pub(super) fn architecture() -> Option<TargetArch> {
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
pub(super) fn seccomp(role: Role, arch: TargetArch) -> Result<(), String> {
    let mut rules: BTreeMap<i64, Vec<SeccompRule>> = BTreeMap::new();
    // Starting programs, tracing and reaching into other processes.
    let denied: &[libc::c_long] = &[
        libc::SYS_execve,
        libc::SYS_execveat,
        libc::SYS_ptrace,
        libc::SYS_process_vm_readv,
        libc::SYS_process_vm_writev,
        libc::SYS_kcmp,
        // A copy of another process's descriptor, which reaches whatever
        // that process holds open (decision 86).
        libc::SYS_pidfd_getfd,
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
    rules.insert(number(libc::SYS_ioctl), terminal()?);
    signals(&mut rules)?;
    // A new process, but not a new thread: clone without CLONE_THREAD.
    let thread_flag = u64::try_from(libc::CLONE_THREAD).map_err(|_| "CLONE_THREAD")?;
    let not_a_thread =
        SeccompRule::new(vec![SeccompCondition::new(0, SeccompCmpArgLen::Qword, SeccompCmpOp::MaskedEq(thread_flag), 0).map_err(|error| error.to_string())?])
            .map_err(|error| error.to_string())?;
    rules.insert(number(libc::SYS_clone), vec![not_a_thread]);
    sockets(role, &mut rules)?;
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
    seccompiler::apply_filter(&program).map_err(|error| error.to_string())?;
    #[cfg(target_arch = "x86_64")]
    seccompiler::apply_filter(&no_x32(refused)?).map_err(|error| error.to_string())?;
    Ok(())
}

/// Sockets: none for a decoder or the trusted core; for the others, a
/// socket of a family the role has no use for is refused.
fn sockets(role: Role, rules: &mut BTreeMap<i64, Vec<SeccompRule>>) -> Result<(), String> {
    let allowed = families(role);
    if allowed.is_empty() {
        for syscall in [libc::SYS_socket, libc::SYS_socketpair, libc::SYS_connect, libc::SYS_bind, libc::SYS_listen, libc::SYS_accept4] {
            rules.insert(number(syscall), always());
        }
        return Ok(());
    }
    // Every condition of one rule must hold, so one rule says "none of
    // these families".
    let other_family: Vec<(u8, SeccompCmpOp, u64)> = allowed.iter().map(|&family| (0, SeccompCmpOp::Ne, family)).collect();
    let mut socket = vec![rule(&other_family)?];
    // Of netlink, routing alone, which the resolver asks for the host's
    // address families (decision 86); the rules of one call are
    // alternatives, so this refuses every other netlink protocol.
    let netlink = libc::AF_NETLINK as u64;
    if allowed.contains(&netlink) {
        let route = u64::try_from(libc::NETLINK_ROUTE).map_err(|_| "NETLINK_ROUTE")?;
        socket.push(rule(&[(0, SeccompCmpOp::Eq, netlink), (2, SeccompCmpOp::Ne, route)])?);
    }
    rules.insert(number(libc::SYS_socket), socket);
    rules.insert(number(libc::SYS_listen), always());
    rules.insert(number(libc::SYS_accept4), always());
    Ok(())
}

/// Signals only to the helper itself (decision 94, finding F-60): the
/// user's other processes, the broker among them, run as the same user, so
/// without this a compromised helper could stop the broker or kill the
/// user's editor. The calls that name a process by its number may name
/// only this one, read now, before the filter, since no process can be
/// started after it; `tkill` names a thread of any process and the pidfd
/// calls a process by a descriptor, so they are refused whole. Landlock
/// 6's signal scoping would do it, but needs Linux 6.12 (decision 94).
fn signals(rules: &mut BTreeMap<i64, Vec<SeccompRule>>) -> Result<(), String> {
    let own = u64::from(std::process::id());
    for syscall in [libc::SYS_kill, libc::SYS_tgkill, libc::SYS_rt_sigqueueinfo, libc::SYS_rt_tgsigqueueinfo] {
        rules.insert(number(syscall), vec![rule(&[(0, SeccompCmpOp::Ne, own)])?]);
    }
    for syscall in [libc::SYS_tkill, libc::SYS_pidfd_open, libc::SYS_pidfd_send_signal] {
        rules.insert(number(syscall), always());
    }
    Ok(())
}

/// One rule: every condition, each on a 32-bit argument, must hold.
fn rule(conditions: &[(u8, SeccompCmpOp, u64)]) -> Result<SeccompRule, String> {
    let conditions = conditions
        .iter()
        .map(|(argument, operation, value)| SeccompCondition::new(*argument, SeccompCmpArgLen::Dword, operation.clone(), *value))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    SeccompRule::new(conditions).map_err(|error| error.to_string())
}

/// No typing into a terminal (finding F-22): a helper started from a
/// terminal shares it as standard error, and `TIOCSTI` queues input for
/// the user's shell, as `TIOCLINUX` does on a virtual console. Landlock's
/// device rule does not cover a descriptor opened before it.
fn terminal() -> Result<Vec<SeccompRule>, String> {
    [libc::TIOCSTI, libc::TIOCLINUX]
        .into_iter()
        .map(|request| {
            #[allow(clippy::useless_conversion, reason = "an ioctl request is a c_ulong with glibc and a c_int with musl")]
            let request = u64::try_from(request).map_err(|_| "a terminal request")?;
            rule(&[(1, SeccompCmpOp::Eq, request)])
        })
        .collect()
}

/// `AUDIT_ARCH_X86_64` (`include/uapi/linux/audit.h`: `EM_X86_64 |
/// __AUDIT_ARCH_64BIT | __AUDIT_ARCH_LE`), the architecture the x32 ABI
/// shares with x86-64.
#[cfg(target_arch = "x86_64")]
const AUDIT_ARCH_X86_64: u32 = libc::EM_X86_64 as u32 | AUDIT_ARCH_64BIT | AUDIT_ARCH_LE;
/// `__AUDIT_ARCH_64BIT` and `__AUDIT_ARCH_LE` (`include/uapi/linux/audit.h`).
#[cfg(target_arch = "x86_64")]
const AUDIT_ARCH_64BIT: u32 = 0x8000_0000;
#[cfg(target_arch = "x86_64")]
const AUDIT_ARCH_LE: u32 = 0x4000_0000;
/// `__X32_SYSCALL_BIT` (`arch/x86/include/uapi/asm/unistd.h`): a system
/// call number with it set is the x32 ABI's.
#[cfg(target_arch = "x86_64")]
pub(super) const X32_SYSCALL_BIT: u32 = 0x4000_0000;

/// One BPF instruction (`include/uapi/linux/filter.h`).
#[cfg(target_arch = "x86_64")]
fn instruction(code: u32, jump_true: u8, jump_false: u8, value: u32) -> Result<seccompiler::sock_filter, String> {
    let code = u16::try_from(code).map_err(|_| "a BPF code beyond 16 bits")?;
    Ok(seccompiler::sock_filter { code, jt: jump_true, jf: jump_false, k: value })
}

/// The offset of a field of `seccomp_data`, as a BPF load takes it.
#[cfg(target_arch = "x86_64")]
fn field(offset: usize) -> Result<u32, String> {
    u32::try_from(offset).map_err(|_| String::from("a seccomp_data offset beyond 32 bits"))
}

/// A filter refusing, with `refused`, every system call by an x32 number
/// (finding F-21): x32 shares x86-64's architecture value, which is all
/// seccompiler checks, so a call by its x32 number matches none of the
/// filter's rules, and with "allow" for what matches none, `execve`,
/// `bpf` or `io_uring_setup` would run where the kernel has x32. Written
/// by hand: seccompiler has no such check.
#[cfg(target_arch = "x86_64")]
fn no_x32(refused: u32) -> Result<BpfProgram, String> {
    use libc::{BPF_ABS, BPF_JEQ, BPF_JGE, BPF_JMP, BPF_K, BPF_LD, BPF_RET, BPF_W, SECCOMP_RET_ALLOW, SECCOMP_RET_DATA, SECCOMP_RET_ERRNO, seccomp_data};
    let arch = field(std::mem::offset_of!(seccomp_data, arch))?;
    let nr = field(std::mem::offset_of!(seccomp_data, nr))?;
    let errno = SECCOMP_RET_ERRNO | (refused & SECCOMP_RET_DATA);
    Ok(vec![
        // 0: another architecture is the first filter's to refuse; allow.
        instruction(BPF_LD | BPF_W | BPF_ABS, 0, 0, arch)?,
        instruction(BPF_JMP | BPF_JEQ | BPF_K, 0, 3, AUDIT_ARCH_X86_64)?,
        // 2: an x32 number is refused.
        instruction(BPF_LD | BPF_W | BPF_ABS, 0, 0, nr)?,
        instruction(BPF_JMP | BPF_JGE | BPF_K, 0, 1, X32_SYSCALL_BIT)?,
        instruction(BPF_RET | BPF_K, 0, 0, errno)?,
        // 5: anything else is the first filter's to decide.
        instruction(BPF_RET | BPF_K, 0, 0, SECCOMP_RET_ALLOW)?,
    ])
}
