//! Each role confined on Linux, as a helper confines itself, and what it can
//! still do: run through the single-threaded probe program.

#![cfg(target_os = "linux")]

use std::process::Command;

/// The probe's one line, run as `role` with `environment` added.
fn probe(role: &str, environment: &[(&str, &str)]) -> String {
    let mut command = Command::new(env!("CARGO_BIN_EXE_confine-probe"));
    command.env("NEOTELETEXT_CONFINE_PROBE", role);
    for (name, value) in environment {
        command.env(name, value);
    }
    let output = command.output().unwrap();
    let text = String::from_utf8_lossy(&output.stdout).into_owned();
    text.lines().find(|line| line.starts_with("PROBE")).map_or_else(|| panic!("no probe line in {text}"), str::to_owned)
}

/// Requirement 3.4: a decoder has every layer, and can open no file, start
/// no program and make no connection.
#[test]
fn a_decoder_can_do_nothing_but_compute() {
    let line = probe("decoder", &[]);
    assert!(line.starts_with("PROBE report=network namespace, Landlock, seccomp;"), "{line}");
    for refused in ["file=refused", "program=refused", "tcp=refused", "write=refused"] {
        assert!(line.contains(refused), "{line}");
    }
}

/// Requirement 3.3: a fetcher may connect but not write or start programs.
#[test]
fn a_fetcher_has_the_network_and_nothing_else() {
    let line = probe("network", &[]);
    assert!(line.starts_with("PROBE report=Landlock, seccomp;"), "{line}");
    assert!(line.contains("program=refused") && line.contains("write=refused"), "{line}");
    assert!(line.contains("tcp=no listener") || line.contains("tcp=connected"), "the network is open: {line}");
}

/// `installation-and-updates.md` section 8.1: the updater reaches its one
/// port and no other, writes its staging folder and nothing else, and
/// starts nothing.
#[test]
fn the_updater_reaches_one_port_and_writes_only_its_staging_folder() {
    let staging = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("probe-staging");
    std::fs::create_dir_all(&staging).unwrap();
    let line = probe("updater", &[("NEOTELETEXT_CONFINE_PROBE_WRITABLE", staging.to_str().unwrap())]);
    assert!(line.starts_with("PROBE report=Landlock, seccomp;"), "{line}");
    assert!(line.contains("tcp=no listener") || line.contains("tcp=connected"), "its port is open: {line}");
    for expected in ["other-port=refused", "program=refused", "write=refused", "own-folder=wrote"] {
        assert!(line.contains(expected), "{expected}: {line}");
    }
}

/// Decision 58: a helper that cannot have its network namespace, as on a
/// distribution that forbids user namespaces, is not run without it.
/// Before, it ran and reported "without a network namespace".
#[test]
fn a_helper_without_its_namespace_does_not_run() {
    let line = probe("decoder-without-namespaces", &[]);
    assert!(line.starts_with("PROBE refused: cannot make an empty network namespace"), "{line}");
}

/// Decision 58: Landlock is enforced whole. A file to read (here the
/// display's authority file) was given a directory's rights, which left
/// the interface's ruleset enforced only in part.
#[test]
fn a_file_to_read_keeps_landlock_whole() {
    let line = probe("interface", &[("XAUTHORITY", "/etc/hostname")]);
    assert!(line.starts_with("PROBE report=network namespace, private /run and /tmp, Landlock, seccomp;"), "{line}");
}

/// A runtime folder of the test's own, as `XDG_RUNTIME_DIR` names one,
/// holding listening sockets named as the session bus, the Wayland display
/// and the sound server name theirs. The listeners live as long as the
/// value; a connection is queued without being accepted.
struct Runtime {
    folder: std::path::PathBuf,
    _listeners: Vec<std::os::unix::net::UnixListener>,
}

impl Runtime {
    fn new(name: &str) -> Self {
        let folder = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("probe-runtime-{name}"));
        let _ = std::fs::remove_dir_all(&folder);
        std::fs::create_dir_all(&folder).unwrap();
        let listeners = ["bus", "wayland-0", "pipewire-0"].iter().map(|socket| std::os::unix::net::UnixListener::bind(folder.join(socket)).unwrap()).collect();
        Self { folder, _listeners: listeners }
    }

    fn probe(&self, role: &str) -> String {
        probe(role, &[("XDG_RUNTIME_DIR", self.folder.to_str().unwrap())])
    }
}

/// Decision 86 (finding F-01): a fetcher, the phone keyboard and the
/// updater may open no local socket, so none can reach the session bus,
/// where systemd starts any program unconfined, the system bus, or an
/// agent's socket; of netlink they keep routing alone, which the resolver
/// asks; names still resolve. Before, `bus` connected, and so did an audit
/// netlink socket.
#[test]
fn a_network_helper_reaches_no_local_socket() {
    let runtime = Runtime::new("network");
    for role in ["network", "updater"] {
        let line = runtime.probe(role);
        for expected in
            ["bus=refused", "display=refused", "user-bus=refused", "system-bus=refused", "netlink-route=opened", "netlink-audit=refused", "resolve=resolved"]
        {
            assert!(line.contains(expected), "{role}: {expected}: {line}");
        }
    }
}

/// Decision 86: the interface sees no `/run` or `/tmp` but its own private
/// ones, and its runtime folder holds only its display's socket, so it
/// reaches neither the session nor the system bus. Before, `bus`
/// connected, and the real session bus was there to reach.
#[test]
fn the_interface_reaches_its_display_and_no_bus() {
    let runtime = Runtime::new("interface");
    let line = runtime.probe("interface");
    assert!(line.starts_with("PROBE report=network namespace, private /run and /tmp, Landlock, seccomp;"), "{line}");
    for expected in ["display=connected", "bus=absent", "sound=absent", "user-bus=absent", "system-bus=absent"] {
        assert!(line.contains(expected), "{expected}: {line}");
    }
}

/// Decision 86: the sound helper reaches its sound server and no bus.
#[test]
fn the_sound_helper_reaches_its_server_and_no_bus() {
    let runtime = Runtime::new("sound");
    let line = runtime.probe("sound");
    assert!(line.starts_with("PROBE report=network namespace, private /run and /tmp, Landlock, seccomp;"), "{line}");
    for expected in ["sound=connected", "bus=absent", "display=absent", "user-bus=absent", "system-bus=absent"] {
        assert!(line.contains(expected), "{expected}: {line}");
    }
}

/// Decision 86: no helper may take a copy of another process's
/// descriptors (`pidfd_getfd`). Before, a fetcher took one of its own.
#[test]
fn no_helper_takes_another_process_descriptor() {
    for role in ["decoder", "network", "interface", "sound", "updater"] {
        let line = probe(role, &[]);
        assert!(line.contains("steal=refused"), "{role}: {line}");
    }
}

/// Finding F-21: the filter's deny-list holds under the x32 ABI's numbers
/// too, which seccompiler's architecture check alone lets through, since
/// x32 shares x86-64's architecture value. Before, `getpid` by its x32
/// number ran where the kernel has x32 and failed with ENOSYS where it has
/// not, so `execve` by its x32 number would have run where it has.
#[cfg(target_arch = "x86_64")]
#[test]
fn no_role_calls_by_the_x32_numbers() {
    for role in ["decoder", "network", "interface", "sound", "updater"] {
        let line = probe(role, &[]);
        assert!(line.contains("x32=refused"), "{role}: {line}");
    }
}

/// Finding F-22: no role may type into the terminal it was started from
/// (`TIOCSTI`, or `TIOCLINUX` on a virtual console), which would queue a
/// command for the user's shell. Before, the probe typed into its
/// controlling terminal where the kernel allows it
/// (`dev.tty.legacy_tiocsti = 1`), and where it does not, the kernel's own
/// refusal (EIO) was all there was.
#[test]
fn no_role_types_into_its_terminal() {
    for role in ["decoder", "network", "interface", "sound", "updater"] {
        let line = probe(role, &[]);
        assert!(line.contains("terminal=refused"), "{role}: {line}");
    }
}

/// Finding F-34: a confined helper ends when its parent, the broker, does,
/// however the broker ended. Before, a helper whose broker was killed
/// lived on, reparented.
#[test]
fn a_helper_ends_with_its_broker() {
    use std::io::BufRead as _;
    /// How long the helper has to end once its broker is killed: the
    /// kernel sends the signal as the parent exits.
    const ENDS_WITHIN: std::time::Duration = std::time::Duration::from_secs(5);
    let mut broker =
        Command::new(env!("CARGO_BIN_EXE_confine-probe")).env("NEOTELETEXT_CONFINE_PROBE", "parent").stdout(std::process::Stdio::piped()).spawn().unwrap();
    let line = std::io::BufReader::new(broker.stdout.take().unwrap()).lines().map_while(Result::ok).find(|line| line.starts_with("PROBE")).unwrap_or_default();
    let helper: u32 = line.strip_prefix("PROBE helper=").and_then(|pid| pid.parse().ok()).unwrap_or_else(|| panic!("no helper: {line}"));
    broker.kill().unwrap();
    broker.wait().unwrap();
    let ended = confine::probe::ends_within(helper, ENDS_WITHIN);
    if !ended {
        let _ = Command::new("kill").arg("-KILL").arg(helper.to_string()).status();
    }
    assert!(ended, "the helper {helper} outlived its broker");
}

/// Decision 94 (finding F-60): a helper may signal only itself, not its
/// broker or any other of the user's programs; its own signals, which the
/// C library's `abort` sends, still arrive. Before, a decoder could stop
/// its broker.
#[test]
fn a_helper_signals_only_itself() {
    for role in ["decoder", "network", "interface", "sound", "updater"] {
        let line = probe(role, &[]);
        for expected in ["signal-parent=refused", "own-signal=handled"] {
            assert!(line.contains(expected), "{role}: {expected}: {line}");
        }
    }
}
