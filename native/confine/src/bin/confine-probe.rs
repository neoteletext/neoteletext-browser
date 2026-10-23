//! The sandbox tests' probe (`tests/probe.rs`), never shipped: a process of
//! one thread, as a helper is when it confines itself, which confines itself
//! as `NEOTELETEXT_CONFINE_PROBE` says and prints what it can still do. A
//! test harness cannot be its own probe: its threads make the kernel refuse
//! a new user namespace. Linux only; elsewhere the parent confines a process.

#[cfg(target_os = "linux")]
fn main() {
    linux::main();
}

#[cfg(not(target_os = "linux"))]
fn main() {}

#[cfg(target_os = "linux")]
mod linux {
    use std::collections::BTreeMap;
    use std::process::Command;
    use std::time::Duration;

    use confine::Role;
    use seccompiler::{BpfProgram, SeccompAction, SeccompFilter, TargetArch};

    /// Names the role, or "decoder-without-namespaces".
    const PROBE: &str = "NEOTELETEXT_CONFINE_PROBE";
    /// How long a connection may take to be refused or not answered.
    const CONNECT_WAIT: Duration = Duration::from_millis(200);
    /// The folder a role that writes one may write.
    const WRITABLE: &str = "NEOTELETEXT_CONFINE_PROBE_WRITABLE";
    /// The updater's one port in the probe: the port the probe's first
    /// connection tries (9, discard), so the second (10) is another.
    const UPDATER_PORT: u16 = 9;

    /// As a distribution that forbids unprivileged user namespaces does:
    /// `unshare` is refused.
    fn forbid_namespaces() {
        let mut rules = BTreeMap::new();
        rules.insert(libc::SYS_unshare, Vec::new());
        let refused = u32::try_from(libc::EPERM).unwrap();
        let arch = TargetArch::try_from(std::env::consts::ARCH).unwrap();
        let filter = SeccompFilter::new(rules, SeccompAction::Allow, SeccompAction::Errno(refused), arch).unwrap();
        let program: BpfProgram = filter.try_into().unwrap();
        seccompiler::apply_filter(&program).unwrap();
    }

    pub fn main() {
        let role = match std::env::var(PROBE).unwrap_or_default().as_str() {
            "decoder" => Role::Decoder,
            "decoder-without-namespaces" => {
                forbid_namespaces();
                Role::Decoder
            }
            "network" => Role::Network,
            "updater" => Role::Updater { port: UPDATER_PORT },
            _ => Role::Interface,
        };
        let writable = std::env::var_os(WRITABLE).map(std::path::PathBuf::from);
        let report = match confine::apply(role, writable.as_deref()) {
            Ok(report) => report,
            Err(error) => {
                println!("PROBE refused: {error}");
                return;
            }
        };
        let file = std::fs::File::open("/etc/hostname").map_or("refused", |_| "opened");
        let program = Command::new("/bin/true").status().map_or("refused", |_| "started");
        let tcp = std::net::TcpStream::connect_timeout(&"127.0.0.1:9".parse().unwrap(), CONNECT_WAIT)
            .map_or_else(|error| if error.kind() == std::io::ErrorKind::PermissionDenied { "refused" } else { "no listener" }, |_| "connected");
        let write = std::fs::write("/tmp/neoteletext-confine-probe", b"x").map_or("refused", |()| "wrote");
        let other_port = std::net::TcpStream::connect_timeout(&"127.0.0.1:10".parse().unwrap(), CONNECT_WAIT)
            .map_or_else(|error| if error.kind() == std::io::ErrorKind::PermissionDenied { "refused" } else { "no listener" }, |_| "connected");
        let own = writable.map_or("none", |folder| std::fs::write(folder.join("probe"), b"x").map_or("refused", |()| "wrote"));
        println!("PROBE report={report}; file={file}; program={program}; tcp={tcp}; write={write}; other-port={other_port}; own-folder={own}");
    }
}
