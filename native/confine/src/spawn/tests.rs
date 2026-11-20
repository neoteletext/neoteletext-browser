//! The sandbox from the parent's side: what a helper is given, and that
//! it is not started when it cannot be confined (decision 58).

use super::*;
use crate::environment;
use std::io::{BufRead as _, BufReader};

/// What a spawned copy of this test binary does: prints its environment.
const PROBE: &str = "NEOTELETEXT_CONFINE_ENVIRONMENT_PROBE";

/// The probe: run by `a_helper_gets_only_the_environment_it_is_given`,
/// never on its own.
#[test]
fn environment_probe() {
    if std::env::var_os(PROBE).is_none() {
        return;
    }
    for (name, value) in std::env::vars_os() {
        println!("ENV {}={}", name.to_string_lossy(), value.to_string_lossy());
    }
}

/// Requirement 3.7: a helper's environment is exactly what the trusted
/// core passes; nothing of the core's own leaks. Before this, every
/// helper inherited the core's whole environment. It is given what a
/// decoder is given: on Windows a container cannot be made without the
/// user's profile folders, and without its container a decoder is not
/// started (decision 58).
#[test]
fn a_helper_gets_only_the_environment_it_is_given() {
    let program = std::env::current_exe().unwrap();
    let arguments = [OsString::from("--exact"), OsString::from("spawn::tests::environment_probe"), OsString::from("--nocapture")];
    let mut environment = environment(Role::Decoder);
    environment.extend([(OsString::from(PROBE), OsString::from("1")), (OsString::from("NEOTELETEXT_GIVEN"), OsString::from("yes"))]);
    let mut process = spawn(&Spawn {
        program: &program,
        arguments: &arguments,
        role: Role::Decoder,
        environment: &environment,
        local_network: false,
        udp_port: None,
        writable: None,
    })
    .unwrap();
    let output = process.stdout.take().unwrap();
    let names: Vec<String> = BufReader::new(output)
        .lines()
        .map_while(Result::ok)
        .filter_map(|line| line.strip_prefix("ENV ").map(|rest| rest.split('=').next().unwrap_or("").to_owned()))
        .collect();
    let _ = process.wait();
    assert!(names.contains(&String::from("NEOTELETEXT_GIVEN")), "{names:?}");
    // Nothing of the core's environment that was not passed, such as
    // its path or shell, is there.
    let passed: Vec<String> = environment.iter().map(|(name, _)| name.to_string_lossy().into_owned()).collect();
    for leaked in ["HOME", "PATH", "USER", "SHELL", "TMPDIR", "USERPROFILE", "APPDATA"].into_iter().filter(|name| !passed.iter().any(|given| given == name)) {
        assert!(!names.contains(&String::from(leaked)), "{leaked} leaked: {names:?}");
    }
    assert!(!process.report.layers.is_empty(), "{}", process.report);
}

/// Decision 58: a helper whose container cannot be made is not
/// started. Without the user's profile folders in its environment an
/// `AppContainer` cannot be created; the code before started the helper
/// with the restricted token alone instead, which is how this crate's
/// own environment test had run unseen without its container.
#[cfg(windows)]
#[test]
fn a_helper_whose_container_cannot_be_made_is_not_started() {
    let program = std::env::current_exe().unwrap();
    let arguments = [OsString::from("--exact"), OsString::from("spawn::tests::environment_probe")];
    let started =
        spawn(&Spawn { program: &program, arguments: &arguments, role: Role::Decoder, environment: &[], local_network: false, udp_port: None, writable: None });
    let refused = match started {
        Ok(mut process) => {
            let _ = process.kill();
            let _ = process.wait();
            format!("started: {}", process.report)
        }
        Err(error) => error.to_string(),
    };
    assert!(refused.starts_with("cannot start in its AppContainer"), "{refused}");
}

#[test]
fn a_report_reads_as_a_list() {
    let report = Report { layers: vec!["network namespace".into(), "Landlock".into(), "seccomp".into()] };
    assert_eq!(report.to_string(), "network namespace, Landlock, seccomp");
}

/// What a spawned copy of this test binary reaches as the interface: the
/// listener's port, and the file it must not read.
#[cfg(windows)]
const INTERFACE_PORT: &str = "NEOTELETEXT_CONFINE_INTERFACE_PORT";
#[cfg(windows)]
const INTERFACE_FILE: &str = "NEOTELETEXT_CONFINE_INTERFACE_FILE";
/// An address on the internet the interface must not reach (Cloudflare's
/// resolver, which answers TLS on 443).
#[cfg(windows)]
const INTERNET: &str = "1.1.1.1:443";
/// How long a connection may take to be refused or not answered.
#[cfg(windows)]
const CONNECT_WAIT: std::time::Duration = std::time::Duration::from_secs(3);

/// The interface probe: run by
/// `the_interface_reaches_no_network_and_no_user_file`, never on its own.
#[cfg(windows)]
#[test]
fn interface_probe() {
    let (Some(port), Some(file)) = (std::env::var_os(INTERFACE_PORT), std::env::var_os(INTERFACE_FILE)) else {
        return;
    };
    let reach = |address: String| {
        address.parse().ok().and_then(|address| std::net::TcpStream::connect_timeout(&address, CONNECT_WAIT).ok()).map_or("refused", |_| "connected")
    };
    let loopback = reach(format!("127.0.0.1:{}", port.to_string_lossy()));
    let internet = reach(String::from(INTERNET));
    let read = std::fs::read(file).map_or("refused", |_| "read");
    println!("REACH loopback={loopback}; internet={internet}; file={read}");
}

/// Decision 90 (finding F-04): the interface runs in an `AppContainer`
/// with no capabilities, so it reaches no network and reads none of the
/// user's files. Before, under its restricted token at low integrity, it
/// connected to this test's listener and read a file in the user's
/// profile folder.
#[cfg(windows)]
#[test]
fn the_interface_reaches_no_network_and_no_user_file() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let profile = std::path::PathBuf::from(std::env::var_os("USERPROFILE").unwrap());
    let file = profile.join("neoteletext-confine-probe.txt");
    std::fs::write(&file, b"the user's").unwrap();
    let program = std::env::current_exe().unwrap();
    let arguments = [OsString::from("--exact"), OsString::from("spawn::tests::interface_probe"), OsString::from("--nocapture")];
    let mut environment = environment(Role::Interface);
    environment.extend([(OsString::from(INTERFACE_PORT), OsString::from(port.to_string())), (OsString::from(INTERFACE_FILE), file.clone().into_os_string())]);
    let mut process = spawn(&Spawn {
        program: &program,
        arguments: &arguments,
        role: Role::Interface,
        environment: &environment,
        local_network: false,
        udp_port: None,
        writable: None,
    })
    .unwrap();
    let output = process.stdout.take().unwrap();
    let line = BufReader::new(output).lines().map_while(Result::ok).filter(|line| line.starts_with("REACH")).last().unwrap_or_default();
    let _ = process.wait();
    let _ = std::fs::remove_file(&file);
    drop(listener);
    for expected in ["loopback=refused", "internet=refused", "file=refused"] {
        assert!(line.contains(expected), "{expected}: {line} ({})", process.report);
    }
    assert!(process.report.layers.iter().any(|layer| layer == "AppContainer"), "{}", process.report);
}

/// What a spawned copy of this test binary connects to as a fetcher: a
/// listener's port on this computer.
#[cfg(target_os = "macos")]
const LOOPBACK_PORT: &str = "NEOTELETEXT_CONFINE_LOOPBACK_PORT";

/// The loopback probe: run by `a_fetcher_reaches_this_computer_only_when_allowed`,
/// never on its own.
#[cfg(target_os = "macos")]
#[test]
fn loopback_probe() {
    let Some(port) = std::env::var_os(LOOPBACK_PORT) else { return };
    let address = format!("127.0.0.1:{}", port.to_string_lossy());
    let reached = std::net::TcpStream::connect(address).map_or("refused", |_| "connected");
    println!("LOOPBACK {reached}");
}

/// Decision 89 (finding F-05): on macOS a fetcher's profile refuses this
/// computer's addresses unless the user allowed the local network
/// (requirement 5.10), so a compromised fetcher cannot reach the services
/// on loopback. Before, it connected either way.
#[cfg(target_os = "macos")]
#[test]
fn a_fetcher_reaches_this_computer_only_when_allowed() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port().to_string();
    let program = std::env::current_exe().unwrap();
    let arguments = [OsString::from("--exact"), OsString::from("spawn::tests::loopback_probe"), OsString::from("--nocapture")];
    let mut environment = environment(Role::Network);
    environment.push((OsString::from(LOOPBACK_PORT), OsString::from(&port)));
    let reached = |local_network: bool| {
        let mut process = spawn(&Spawn {
            program: &program,
            arguments: &arguments,
            role: Role::Network,
            environment: &environment,
            local_network,
            udp_port: None,
            writable: None,
        })
        .unwrap();
        // Read to the end, so the probe's last lines find a reader.
        let line =
            BufReader::new(process.stdout.take().unwrap()).lines().map_while(Result::ok).filter(|line| line.starts_with("LOOPBACK")).last().unwrap_or_default();
        let _ = process.wait();
        line
    };
    assert_eq!(reached(false), "LOOPBACK refused");
    assert_eq!(reached(true), "LOOPBACK connected");
}

/// What a copy of this test binary does, run as the interface: declares a
/// path on the channel it was started with.
const DECLARER_PROBE: &str = "NEOTELETEXT_CONFINE_DECLARER_PROBE";

/// The probe: run by `the_interface_alone_has_a_channel_to_the_broker`,
/// never on its own.
#[test]
fn declarer_probe() {
    if std::env::var_os(DECLARER_PROBE).is_none() {
        return;
    }
    let mut declarer = crate::broker::Declarer::from_parent().unwrap();
    declarer.declare(&crate::broker::Declaration { paths: vec![String::from("/declared.tti")], keep: true }).unwrap();
    println!("DECLARED");
}

/// DEC-1551 item 3: the interface reaches the broker on its own channel,
/// under its real sandbox, and is believed once the broker has acknowledged;
/// no other role is given one. Before this nothing but the core's pipes
/// reached the interface.
#[test]
fn the_interface_alone_has_a_channel_to_the_broker() {
    use std::sync::{Arc, Mutex};

    let program = std::env::current_exe().unwrap();
    let arguments = [OsString::from("--exact"), OsString::from("spawn::tests::declarer_probe"), OsString::from("--nocapture")];
    let mut environment = environment(Role::Interface);
    environment.push((OsString::from(DECLARER_PROBE), OsString::from("1")));
    let start = |role| {
        spawn(&Spawn { program: &program, arguments: &arguments, role, environment: &environment, local_network: false, udp_port: None, writable: None })
            .unwrap()
    };

    let mut decoder = start(Role::Decoder);
    assert!(decoder.broker_side.is_none(), "only the interface has one");
    let _ = decoder.kill();
    let _ = decoder.wait();

    let mut interface = start(Role::Interface);
    let heard = Arc::new(Mutex::new(Vec::new()));
    let recorded = Arc::clone(&heard);
    crate::broker::server::hear_declarations(interface.broker_side.take().unwrap(), Arc::new(move |declaration| recorded.lock().unwrap().push(declaration)));
    let output: Vec<String> = BufReader::new(interface.stdout.take().unwrap()).lines().map_while(Result::ok).collect();
    let _ = interface.wait();
    assert!(output.iter().any(|line| line.contains("DECLARED")), "{output:?}");
    assert_eq!(*heard.lock().unwrap(), [crate::broker::Declaration { paths: vec![String::from("/declared.tti")], keep: true }]);
}

/// What a spawned copy of this test binary connects to as a fetcher on
/// Windows: a listener's port on this computer.
#[cfg(windows)]
const WINDOWS_LOOPBACK_PORT: &str = "NEOTELETEXT_CONFINE_WINDOWS_LOOPBACK_PORT";

/// The Windows loopback probe: run by
/// `a_fetcher_reaches_this_computer_only_with_the_exemption`, never on its
/// own. It says whether it connected, how the connection failed and how
/// long that took, and the fetcher container's SID as it derives it from
/// inside the container.
#[cfg(windows)]
#[test]
fn windows_loopback_probe() {
    let Some(port) = std::env::var_os(WINDOWS_LOOPBACK_PORT) else { return };
    let address: std::net::SocketAddr = format!("127.0.0.1:{}", port.to_string_lossy()).parse().unwrap();
    let started = std::time::Instant::now();
    let reached = match std::net::TcpStream::connect_timeout(&address, CONNECT_WAIT) {
        Ok(_) => String::from("connected"),
        Err(error) => format!("refused ({:?}, os error {:?})", error.kind(), error.raw_os_error()),
    };
    println!("LOOPBACK {reached} after {} ms", started.elapsed().as_millis());
    println!("SID {}", crate::fetcher_container().unwrap_or_default());
}

/// Whether an administrator has exempted the fetcher's container from
/// Windows' loopback isolation: `CheckNetIsolation LoopbackExempt -s` lists
/// each exempted container by its SID.
#[cfg(windows)]
fn fetcher_is_exempt(sid: &str) -> bool {
    let listed = std::process::Command::new("CheckNetIsolation.exe").args(["LoopbackExempt", "-s"]).output().unwrap();
    String::from_utf8_lossy(&listed.stdout).to_ascii_lowercase().contains(&sid.to_ascii_lowercase())
}

/// REQ-2402, decision 78, TODO-1251: on Windows a fetcher, even one allowed the local
/// network, reaches this computer only once an administrator has exempted
/// its container, and the SID the browser gives for that exemption is the
/// one the container derives for itself, inside it. The probe prints how
/// the connection failed, which the trusted core's message rests on.
#[cfg(windows)]
#[test]
fn a_fetcher_reaches_this_computer_only_with_the_exemption() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port().to_string();
    let program = std::env::current_exe().unwrap();
    let arguments = [OsString::from("--exact"), OsString::from("spawn::tests::windows_loopback_probe"), OsString::from("--nocapture")];
    let mut environment = environment(Role::Network);
    environment.push((OsString::from(WINDOWS_LOOPBACK_PORT), OsString::from(&port)));
    let mut process = spawn(&Spawn {
        program: &program,
        arguments: &arguments,
        role: Role::Network,
        environment: &environment,
        local_network: true,
        udp_port: None,
        writable: None,
    })
    .unwrap();
    let lines: Vec<String> = BufReader::new(process.stdout.take().unwrap()).lines().map_while(Result::ok).collect();
    let _ = process.wait();
    drop(listener);
    println!("{}", lines.join("\n"));
    let reached = lines.iter().find(|line| line.starts_with("LOOPBACK")).cloned().unwrap_or_default();
    let sid = lines.iter().find_map(|line| line.strip_prefix("SID ")).unwrap_or_default().to_owned();
    assert_eq!(sid, crate::fetcher_container().unwrap(), "the container derives the SID the broker derives");
    assert_eq!(reached.starts_with("LOOPBACK connected"), fetcher_is_exempt(&sid), "{reached}");
}

/// What a copy of this test binary does, run as a decoder: lists the
/// descriptors it holds.
#[cfg(unix)]
const DESCRIPTORS_PROBE: &str = "NEOTELETEXT_CONFINE_DESCRIPTORS_PROBE";

/// The probe: run by `a_helper_holds_only_its_own_descriptors_while_others_are_made`
/// and `a_descriptor_left_inheritable_does_not_reach_a_helper`, never on
/// its own.
#[cfg(unix)]
#[test]
fn descriptors_probe() {
    if std::env::var_os(DESCRIPTORS_PROBE).is_none() {
        return;
    }
    println!("DESCRIPTORS {:?}", crate::unix::open_descriptors());
}

/// Starts this test binary as a decoder that lists the descriptors it holds,
/// and returns that list as it printed it.
#[cfg(unix)]
fn descriptors_of_a_helper() -> String {
    let program = std::env::current_exe().unwrap();
    let arguments = [OsString::from("--exact"), OsString::from("spawn::tests::descriptors_probe"), OsString::from("--nocapture")];
    let mut environment = environment(Role::Decoder);
    environment.push((OsString::from(DESCRIPTORS_PROBE), OsString::from("1")));
    let mut process = spawn(&Spawn {
        program: &program,
        arguments: &arguments,
        role: Role::Decoder,
        environment: &environment,
        local_network: false,
        udp_port: None,
        writable: None,
    })
    .unwrap();
    let line = BufReader::new(process.stdout.take().unwrap())
        .lines()
        .map_while(Result::ok)
        .find_map(|line| line.strip_prefix("DESCRIPTORS ").map(str::to_owned))
        .unwrap_or_default();
    let _ = process.wait();
    line
}

/// REQ-673 and REQ-674 (TODO-1994): a confined helper holds what it is
/// given and what it opens itself, and nothing else, even while another
/// thread of its parent makes descriptors as it starts. On macOS the
/// standard library makes a socket or a pipe and sets it close-on-exec in a
/// second call, so a helper started between the two held the descriptor for
/// its whole life; the broker's single-instance listener and file dialog
/// make such descriptors on threads of their own. Linux makes them
/// close-on-exec atomically. What a helper opens itself is measured first,
/// with nothing else being made: on macOS a decoder opens the system log's
/// socket.
#[cfg(unix)]
#[test]
fn a_helper_holds_only_its_own_descriptors_while_others_are_made() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    /// How many helpers are started while other threads make descriptors:
    /// on the Mac, on the code before, about one start in five held one
    /// (19 of 100, measured on 3 November 2026), so this many leave no
    /// doubt; they take about 20 milliseconds each there.
    const STARTS: usize = 100;
    /// Threads making descriptors at once.
    const MAKERS: usize = 2;

    let own = descriptors_of_a_helper();
    assert!(own.starts_with("[0, 1, 2"), "a helper holds its standard input, output and error: {own}");
    let stop = Arc::new(AtomicBool::new(false));
    let makers: Vec<_> = (0..MAKERS)
        .map(|_| {
            let making = Arc::clone(&stop);
            std::thread::spawn(move || {
                while !making.load(Ordering::Relaxed) {
                    let pair = std::os::unix::net::UnixStream::pair();
                    let pipe = io::pipe();
                    drop((pair, pipe));
                }
            })
        })
        .collect();
    let held: Vec<String> = (0..STARTS).map(|_| descriptors_of_a_helper()).collect();
    stop.store(true, Ordering::Relaxed);
    for maker in makers {
        maker.join().unwrap();
    }
    let wrong: Vec<&String> = held.iter().filter(|line| **line != own).collect();
    assert!(wrong.is_empty(), "{} of {STARTS} helpers held other descriptors than their own {own}: {wrong:?}", wrong.len());
}

/// REQ-673 and REQ-674 (TODO-1994): a descriptor its parent holds without
/// close-on-exec, as a system library may leave one, does not reach a
/// helper on macOS. Before, every helper was given it.
#[cfg(target_os = "macos")]
#[test]
fn a_descriptor_left_inheritable_does_not_reach_a_helper() {
    /// The number the parent's descriptor is given or exceeds: far above
    /// the few a new helper opens itself, and below the 256 descriptors
    /// macOS allows a process by default (`launchctl limit maxfiles`).
    const HIGH: i32 = 100;

    let (reader, _writer) = io::pipe().unwrap();
    let inheritable = crate::unix::inheritable_copy(&reader, HIGH).unwrap();
    let number = std::os::fd::AsRawFd::as_raw_fd(&inheritable);
    let held = descriptors_of_a_helper();
    assert!(!held.contains(&number.to_string()), "the helper was given descriptor {number}: {held}");
}
