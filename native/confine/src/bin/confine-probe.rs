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
    use std::io::ErrorKind;
    use std::net::ToSocketAddrs as _;
    use std::os::unix::fs::MetadataExt as _;
    use std::os::unix::net::UnixStream;
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use std::time::Duration;

    use confine::{Role, probe};
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
    /// The system's D-Bus socket (the D-Bus specification's well-known
    /// system bus address).
    const SYSTEM_BUS: &str = "/run/dbus/system_bus_socket";
    /// Where systemd's logind keeps a user's runtime folder (`pam_systemd(8)`).
    const USER_RUNTIME: &str = "/run/user";
    /// The longest a probe that stands for a broker, or a helper of it,
    /// lives: far longer than the test that ends it takes.
    const LINGER: Duration = Duration::from_secs(30);
    /// Where POSIX shared memory is made (`shm_open(3)`).
    const SHARED_MEMORY: &str = "/dev/shm";
    /// The size the probe gives the shared memory it makes: a page.
    const SHARED_MEMORY_PROBE_BYTES: u64 = 4096;
    /// A descriptor every process has: its standard input.
    const STANDARD_INPUT: i32 = 0;

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

    /// What became of a connection to a local socket: the sandbox refused
    /// the socket, the path was not there, or it connected.
    fn local(path: &Path) -> String {
        match UnixStream::connect(path) {
            Ok(_) => String::from("connected"),
            Err(error) if error.kind() == ErrorKind::PermissionDenied => String::from("refused"),
            Err(error) if error.kind() == ErrorKind::NotFound => String::from("absent"),
            Err(error) => format!("failed ({error})"),
        }
    }

    /// Whether a socket of `domain`, `kind` and `protocol` could be had.
    fn socket(domain: i32, kind: i32, protocol: i32) -> String {
        match probe::socket(domain, kind, protocol) {
            Ok(()) => String::from("opened"),
            Err(error) if error.kind() == ErrorKind::PermissionDenied => String::from("refused"),
            Err(error) => format!("failed ({error})"),
        }
    }

    /// A broker: it starts a helper, and lives until it is ended.
    fn parent() {
        let mut helper = Command::new(std::env::current_exe().unwrap()).env(PROBE, "helper").spawn().unwrap();
        std::thread::sleep(LINGER);
        let _ = helper.kill();
        let _ = helper.wait();
    }

    /// Its helper, confined: says who it is, and lives on.
    fn helper() {
        if let Err(error) = confine::apply(Role::Decoder, None) {
            println!("PROBE refused: {error}");
            return;
        }
        println!("PROBE helper={}", std::process::id());
        std::thread::sleep(LINGER);
    }

    /// The role `probe` names, the interface's for any other.
    fn role(probe: &str) -> Role {
        match probe {
            "decoder" => Role::Decoder,
            "decoder-without-namespaces" => {
                forbid_namespaces();
                Role::Decoder
            }
            "script" => Role::Script,
            "network" => Role::Network,
            "updater" => Role::Updater { port: UPDATER_PORT },
            "sound" => Role::Sound,
            _ => Role::Interface,
        }
    }

    pub fn main() {
        let probe = std::env::var(PROBE).unwrap_or_default();
        match probe.as_str() {
            "parent" => return parent(),
            "helper" => return helper(),
            _ => {}
        }
        let role = role(&probe);
        let writable = std::env::var_os(WRITABLE).map(PathBuf::from);
        // What is read of this process before it is confined, as a helper
        // reading its own state would.
        let uid = std::fs::metadata("/proc/self").map(|own| own.uid()).unwrap();
        let runtime = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).unwrap_or_default();
        let own_process = probe::own_process().unwrap();
        let (_terminal_main, terminal) = probe::controlling_terminal().unwrap();
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
            .map_or_else(|error| if error.kind() == ErrorKind::PermissionDenied { "refused" } else { "no listener" }, |_| "connected");
        let write = std::fs::write("/tmp/neoteletext-confine-probe", b"x").map_or("refused", |()| "wrote");
        let other_port = std::net::TcpStream::connect_timeout(&"127.0.0.1:10".parse().unwrap(), CONNECT_WAIT)
            .map_or_else(|error| if error.kind() == ErrorKind::PermissionDenied { "refused" } else { "no listener" }, |_| "connected");
        let own = writable.map_or("none", |folder| std::fs::write(folder.join("probe"), b"x").map_or("refused", |()| "wrote"));
        // POSIX shared memory, as `shm_open(3)` makes it: the interface's
        // presenter shares its image with the X server so (DEC-6659 item 24).
        let shared = Path::new(SHARED_MEMORY).join(format!("neoteletext-confine-probe-{}", std::process::id()));
        let shared_memory = match std::fs::OpenOptions::new().read(true).write(true).create_new(true).open(&shared) {
            Ok(file) => {
                let sized = file.set_len(SHARED_MEMORY_PROBE_BYTES);
                let _ = std::fs::remove_file(&shared);
                sized.map_or_else(|error| format!("made, not sized ({error})"), |()| String::from("made"))
            }
            Err(error) if error.kind() == ErrorKind::PermissionDenied => String::from("refused"),
            Err(error) => format!("failed ({error})"),
        };
        let bus = local(&runtime.join("bus"));
        let display = local(&runtime.join("wayland-0"));
        let sound = local(&runtime.join("pipewire-0"));
        let user_bus = local(&Path::new(USER_RUNTIME).join(uid.to_string()).join("bus"));
        let system_bus = local(Path::new(SYSTEM_BUS));
        let netlink_route = socket(libc::AF_NETLINK, libc::SOCK_RAW, libc::NETLINK_ROUTE);
        let netlink_audit = socket(libc::AF_NETLINK, libc::SOCK_RAW, libc::NETLINK_AUDIT);
        let resolve = ("localhost", 0).to_socket_addrs().map_or_else(|error| format!("failed ({error})"), |_| String::from("resolved"));
        let steal = probe::take_descriptor(&own_process, STANDARD_INPUT).map_or("refused", |()| "took");
        #[cfg(target_arch = "x86_64")]
        let x32 = match probe::x32_getpid() {
            Ok(_) => String::from("ran"),
            Err(error) if error.kind() == ErrorKind::PermissionDenied => String::from("refused"),
            Err(error) => format!("failed ({error})"),
        };
        #[cfg(not(target_arch = "x86_64"))]
        let x32 = "none";
        let signal = match probe::signal_parent() {
            Ok(()) => String::from("sent"),
            Err(error) if error.kind() == ErrorKind::PermissionDenied => String::from("refused"),
            Err(error) => format!("failed ({error})"),
        };
        let own_signal = probe::raise_own().map_or_else(|error| format!("failed ({error})"), |handled| String::from(if handled { "handled" } else { "lost" }));
        let typed = match probe::type_into(&terminal) {
            Ok(()) => String::from("typed"),
            Err(error) if error.kind() == ErrorKind::PermissionDenied => String::from("refused"),
            Err(error) => format!("failed ({error})"),
        };
        // More than a script host may hold, asked for in one piece
        // (`script-api.md` section 10.6), and a thread, which its
        // watchdog is (section 3.2 rule 5(b)).
        let mut room: Vec<u8> = Vec::new();
        let memory = room.try_reserve_exact(confine::SCRIPT_MEMORY_BYTES).map_or("refused", |()| "reserved");
        drop(room);
        let thread = std::thread::Builder::new().spawn(|| ()).map_or("refused", |thread| if thread.join().is_ok() { "ran" } else { "failed" });
        println!(
            "PROBE report={report}; memory={memory}; thread={thread}; file={file}; program={program}; tcp={tcp}; write={write}; other-port={other_port}; own-folder={own}; \
             bus={bus}; display={display}; sound={sound}; user-bus={user_bus}; system-bus={system_bus}; netlink-route={netlink_route}; \
             netlink-audit={netlink_audit}; resolve={resolve}; steal={steal}; shared-memory={shared_memory}; x32={x32}; terminal={typed}; signal-parent={signal}; own-signal={own_signal}"
        );
    }
}
