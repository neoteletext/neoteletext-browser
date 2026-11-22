//! The script-host program as the broker starts it: confined as
//! `Role::Script`, on its pipes (script-api.md section 11.1; DEC-7266;
//! TODO-7292, and what TODO-7338 hands this program: the watchdog, a
//! last batch after a close, a fresh seed for each context).
//!
//! Nothing here asserts how fast this machine is. A script host's limits
//! are on the wall clock, and on a machine with no processor free a
//! script's start alone can pass its limit; a test whose script was
//! stopped for time before it ran is tried again, as the script
//! library's own tests are.

#[path = "../../script/tests/common/mod.rs"]
mod common;

use std::ffi::OsString;
use std::io::{BufReader, Read, Write};
use std::path::Path;
use std::time::{Duration, Instant};

use common::{claim, engine, key, lines, opening, tick, with_events};
use confine::{Input, Output, Process, Role, Spawn};
use neoteletext_script::watchdog::WATCHDOG_EXIT_STATUS;
use neoteletext_script::{API_VERSION, InProcess, LinkError, ProcessLink, ScriptLink, Settings, TimeLimits, Watchdog};
use protocol::script::relay::{HostOutput, STEP_DEADLINE, read_host_output};
use protocol::script::{ContextId, ContextOpen, EventKind, FailReason, FromScriptHost, LINK_VERSION, ToScriptHost, read_to_decoder, write_to_host};

/// How often a test whose script was stopped for time is tried again
/// when this machine had no processor for it.
const STARVED_TRIES: usize = 5;

/// The script host, started confined as the broker starts it.
fn start() -> Process {
    let program = Path::new(env!("CARGO_BIN_EXE_neoteletext-script-host"));
    let environment = confine::environment(Role::Script);
    let arguments: [OsString; 0] = [];
    confine::spawn(&Spawn {
        program,
        arguments: &arguments,
        role: Role::Script,
        environment: &environment,
        local_network: false,
        udp_port: None,
        writable: None,
    })
    .unwrap()
}

/// A script host and the link to it.
fn linked() -> (Process, ProcessLink<BufReader<Output>, Input>) {
    let mut process = start();
    let link = ProcessLink::new(BufReader::new(process.stdout.take().unwrap()), process.stdin.take().unwrap());
    (process, link)
}

/// An opening whose length is its script's.
fn opening_of(context: ContextId, source: &[u8]) -> ContextOpen {
    ContextOpen { source_bytes: u32::try_from(source.len()).unwrap(), ..opening(context) }
}

fn ended(mut process: Process) -> std::process::ExitStatus {
    process.wait().unwrap()
}

/// Whether a run was stopped for time, which on a machine with no
/// processor free says nothing of the script.
fn starved<T>(result: &Result<T, LinkError>) -> bool {
    matches!(result, Err(LinkError::Stopped(FailReason::Time, _)))
}

/// Section 11.1 and 11.2: a script host says READY, with this link's
/// version and the library's API version, before it reads anything, and
/// reports every layer of its confinement; told nothing more, it ends
/// successfully when its input ends.
#[test]
fn a_script_host_says_it_is_ready_before_it_reads_anything_and_ends_with_its_input() {
    let mut process = start();
    #[cfg(target_os = "linux")]
    assert_eq!(process.report.to_string(), "confines itself at its start");
    let mut output = BufReader::new(process.stdout.take().unwrap());
    let first = read_to_decoder(&mut output).unwrap();
    assert_eq!(first, Some(FromScriptHost::Ready { link_version: LINK_VERSION, api_max: API_VERSION }));
    drop(process.stdin.take());
    assert_eq!(read_to_decoder(&mut output).unwrap(), None, "it said something more");
    assert!(ended(process).success());
}

/// What `link` logs for one fixed run of two contexts: through the trait
/// alone, as a decoder or the runner drives it.
fn drive(link: &mut dyn ScriptLink) -> Result<Vec<String>, LinkError> {
    let mut log = Vec::new();
    for context in [ContextId::Channel, ContextId::Page] {
        let source = "const who = nt.level; nt.addEventListener('tick', (e) => { if (e.tick === 1) console.log(who + ' tick'); });
                      setTimeout(() => console.log(who + ' timer'), 0);
                      nt.addEventListener('keydown', (e) => console.log(who + ' key ' + e.key));";
        let mut open = opening_of(context, source.as_bytes());
        claim(&mut open, &["Enter"]);
        link.open(open, source.as_bytes())?;
    }
    log.extend(lines(&link.step(tick(1, 0))?));
    log.extend(lines(&link.step(with_events(2, 1, vec![key(EventKind::KeyDown, "Enter", 0)]))?));
    link.close(ContextId::Page);
    log.extend(lines(&link.step(tick(3, 2))?));
    link.close(ContextId::Channel);
    Ok(log)
}

/// Section 11.1 ("one interface") and TODO-7292 step 5: the same code
/// drives the scripts in a process of their own and in the caller's, and
/// they do the same.
#[test]
fn the_scripts_do_the_same_in_a_process_of_their_own_as_in_the_callers() {
    let mut in_process = InProcess::new(engine().clone(), Settings { time: TimeLimits::unhurried(), watchdog: None });
    let expected = drive(&mut in_process).unwrap();
    assert_eq!(expected, vec!["page key Enter", "channel key Enter", "page timer", "channel timer", "page tick", "channel tick"]);
    for _ in 0..STARVED_TRIES {
        let (process, mut link) = linked();
        let driven = drive(&mut link);
        if starved(&driven) {
            continue;
        }
        assert_eq!(driven.unwrap(), expected);
        assert_eq!(link.api_max(), Some(API_VERSION));
        assert!(matches!(link.step(tick(4, 3)), Err(LinkError::Context(_))), "a step after every context closed");
        link.shutdown();
        assert!(ended(process).success());
        return;
    }
    panic!("stopped for time in {STARVED_TRIES} tries: this machine has no processor free");
}

/// Section 3.1 and TODO-7338 item 3: what the one handling of `close`
/// staged leaves in a batch of its own, also when the last context goes
/// and no step follows; and the next page's context opens in the same
/// process, in a new container.
#[test]
fn what_a_close_handler_staged_leaves_in_a_last_batch() {
    for _ in 0..STARVED_TRIES {
        let (process, mut link) = linked();
        let source = b"nt.addEventListener('close', () => console.log('closing ' + nt.page.number));";
        let opened = link.open(opening_of(ContextId::Page, source), source).and_then(|()| link.step(tick(1, 0)));
        if starved(&opened) {
            continue;
        }
        opened.unwrap();
        link.close(ContextId::Page);
        let closed = link.take_closed();
        let closed: Vec<(ContextId, Vec<String>)> = closed.iter().map(|(context, batch)| (*context, lines(batch))).collect();
        assert_eq!(closed.len(), 1, "{closed:?}");
        assert_eq!(closed[0].0, ContextId::Page);
        assert!(closed[0].1.iter().any(|line| line.starts_with("closing ")), "{closed:?}");
        // The next page, in the same process.
        let next = b"console.log('next page');";
        let opened = link.open(opening_of(ContextId::Page, next), next).and_then(|()| link.step(tick(2, 0)));
        if starved(&opened) {
            continue;
        }
        assert_eq!(lines(&opened.unwrap()), vec!["next page"]);
        link.shutdown();
        assert!(ended(process).success());
        return;
    }
    panic!("stopped for time in {STARVED_TRIES} tries: this machine has no processor free");
}

/// Section 13 item 7, TODO-7279 item 3 and TODO-7338 item 4: a context's
/// random numbers start from a seed the script host made fresh from the
/// operating system, so two contexts given the same seed by their
/// decoders draw different numbers. (In the library alone the same seed
/// gives the same numbers, which its own tests hold.)
#[test]
fn a_context_given_the_same_seed_twice_draws_different_numbers() {
    let source = b"const bytes = new Uint8Array(16); crypto.getRandomValues(bytes); console.log(Array.from(bytes).join(',') + ' ' + Math.random());";
    let mut drawn = Vec::new();
    for _ in 0..STARVED_TRIES {
        let (process, mut link) = linked();
        let run = link.open(opening_of(ContextId::Page, source), source).and_then(|()| link.step(tick(1, 0)));
        link.shutdown();
        let _ = ended(process);
        if starved(&run) {
            continue;
        }
        drawn.push(lines(&run.unwrap()));
        if drawn.len() == 2 {
            break;
        }
    }
    assert_eq!(drawn.len(), 2, "this machine has no processor free");
    assert!(drawn[0].len() == 1 && drawn[1].len() == 1, "{drawn:?}");
    assert_ne!(drawn[0], drawn[1], "two contexts with the same seed drew the same numbers");
}

/// Writes `bytes` to a new script host and gives everything it answers
/// after READY, and how it ended.
fn answers_to(bytes: &[u8]) -> (Vec<FromScriptHost>, std::process::ExitStatus) {
    let mut process = start();
    let mut input = process.stdin.take().unwrap();
    // It may end before it has read everything: that is not this side's
    // failure.
    let _ = input.write_all(bytes);
    drop(input);
    let mut output = BufReader::new(process.stdout.take().unwrap());
    let mut said = Vec::new();
    while let Ok(Some(message)) = read_to_decoder(&mut output) {
        said.push(message);
    }
    (said, ended(process))
}

/// Section 11.2: a message that is malformed, of a kind a script host is
/// never given, or out of its order ends the script host with a FAILED
/// whose reason is the link, and it runs nothing.
#[test]
fn a_message_it_may_not_be_given_ends_it_with_a_failure_of_the_link() {
    let mut step_first = Vec::new();
    write_to_host(&mut step_first, &ToScriptHost::Step(Box::new(tick(1, 0)))).unwrap();
    let mut twice = Vec::new();
    write_to_host(&mut twice, &ToScriptHost::Hello { link_version: LINK_VERSION }).unwrap();
    write_to_host(&mut twice, &ToScriptHost::Hello { link_version: LINK_VERSION }).unwrap();
    let mut other_version = Vec::new();
    write_to_host(&mut other_version, &ToScriptHost::Hello { link_version: LINK_VERSION + 1 }).unwrap();
    let mut orphan = Vec::new();
    write_to_host(&mut orphan, &ToScriptHost::Hello { link_version: LINK_VERSION }).unwrap();
    write_to_host(&mut orphan, &ToScriptHost::Source { context: ContextId::Page, offset: 0, bytes: b"1".to_vec() }).unwrap();
    // A decoder's own message to the trusted core, and a READY, which
    // only a script host says.
    let cases: [(&str, &[u8]); 6] = [
        ("a step before the greeting", &step_first),
        ("two greetings", &twice),
        ("another link's version", &other_version),
        ("a source piece with no opening", &orphan),
        ("a kind that is no script host's", &[1, 0, 0, 0, 0]),
        ("a script host's own kind", &[226, 0, 0, 0, 4, 0, 1, 0, 1]),
    ];
    for (what, bytes) in cases {
        let (said, status) = answers_to(bytes);
        assert_eq!(said.first(), Some(&FromScriptHost::Ready { link_version: LINK_VERSION, api_max: API_VERSION }), "{what}");
        assert!(matches!(said.get(1), Some(FromScriptHost::Failed { reason: FailReason::Link, .. })), "{what}: {said:?}");
        assert_eq!(said.len(), 2, "{what}: {said:?}");
        assert!(!status.success(), "{what}");
    }
}

macro_rules! hostile {
    ($name:literal) => {
        ($name, include_str!(concat!("../../script/tests/hostile/", $name, ".js")))
    };
}

/// The security review's hostile scripts that attack what this program
/// adds, the process and its link (TODO-7336; the review's probe): the
/// top level that never returns, the callback that never returns, the
/// allocation without end, the recursion without end, the string that
/// doubles, and the copies and fills the spike's clock never saw. Run
/// in the confined process under the browser's limits, each is answered
/// or failed for every step, the process ends itself when its container
/// stops, and everything it writes is a message the trusted core relays
/// and the decoder's reader takes. The times are printed, not asserted:
/// the limits themselves are the script library's tests.
#[test]
fn the_reviews_hostile_scripts_are_each_answered_or_failed_and_the_process_ends() {
    const STEPS: u32 = 8;
    let scripts = [
        hostile!("toploop"),
        hostile!("loop"),
        hostile!("alloc"),
        hostile!("recurse"),
        hostile!("recurse2"),
        hostile!("bigstring"),
        hostile!("copy_top"),
        hostile!("copy_cb"),
        hostile!("fill_cb"),
        hostile!("timers"),
        hostile!("budget"),
        hostile!("promises"),
    ];
    for (name, source) in scripts {
        let mut process = start();
        let mut input = process.stdin.take().unwrap();
        let mut output = BufReader::new(process.stdout.take().unwrap());
        let mut asked = protocol::script::link::HostLink::new();
        let mut open = opening_of(ContextId::Page, source.as_bytes());
        open.tick_divisor = 1;
        let mut written = Vec::new();
        asked.open(&mut written, &open, source.as_bytes()).unwrap();
        input.write_all(&written).unwrap();
        input.flush().unwrap();
        let began = Instant::now();
        let mut slowest = Duration::ZERO;
        let mut outcome = String::from("answered every step");
        'steps: for sequence in 1..=STEPS {
            let mut written = Vec::new();
            if asked.step(&mut written, &tick(sequence, u64::from(sequence))).is_err() {
                break;
            }
            // A script host that has ended has closed its pipe.
            if input.write_all(&written).and_then(|()| input.flush()).is_err() {
                break;
            }
            let sent = Instant::now();
            loop {
                // Read as the trusted core reads it first: by kind.
                let relayed = match read_host_output(&mut output) {
                    Ok(Some(HostOutput::ToDecoder(relayed))) => relayed,
                    Ok(Some(HostOutput::Request(_))) => panic!("{name}: a request with no network"),
                    Ok(None) => {
                        outcome = String::from("ended");
                        break 'steps;
                    }
                    Err(error) => panic!("{name}: the trusted core would not relay what it wrote: {error}"),
                };
                let message = read_to_decoder(&mut relayed.frame()).unwrap_or_else(|error| panic!("{name}: the decoder would refuse it: {error}")).unwrap();
                match asked.receive(message) {
                    protocol::script::link::Received::Ready { .. } => {}
                    protocol::script::link::Received::Batch { .. } => {
                        slowest = slowest.max(sent.elapsed());
                        break;
                    }
                    protocol::script::link::Received::Stopped(stopped) => {
                        slowest = slowest.max(sent.elapsed());
                        outcome = format!("failed ({:?}): {}", stopped.reason, stopped.text);
                        assert_ne!(stopped.reason, FailReason::Link, "{name}: {outcome}");
                        break 'steps;
                    }
                    protocol::script::link::Received::Dropped => break 'steps,
                }
            }
        }
        drop(input);
        // Whatever it still writes, it ends: by itself once its container
        // stopped, or with its input.
        let mut rest = Vec::new();
        let _ = output.read_to_end(&mut rest);
        let status = ended(process);
        println!("hostile {name}: {outcome}; slowest answer {slowest:?} (STEP_DEADLINE {STEP_DEADLINE:?}); whole run {:?}; exit {status}", began.elapsed());
        if outcome.starts_with("failed") {
            assert!(!status.success(), "{name}: a stopped container's process ended as if told to");
        }
    }
}

/// The variable that makes this test program the watchdog's subject.
const WATCHED: &str = "NEOTELETEXT_SCRIPT_HOST_TEST_WATCHED";

/// Section 3.2 rule 5(b) and TODO-7338 item 1: the watchdog the program
/// gives every container ends the process with status 70 when an entry
/// is still running `WATCHDOG_GRACE` past its deadline, whatever the
/// thread inside is doing. No script can hold the interpreter that long
/// since the container's own clock checks were fixed (TODO-7270), so
/// the subject is this test program run again, with an entry that never
/// returns.
#[test]
fn the_watchdog_ends_a_process_whose_entry_never_returns_with_status_70() {
    if std::env::var_os(WATCHED).is_some() {
        let watchdog = Watchdog::ending_the_process().unwrap();
        watchdog.arm(Instant::now() + Duration::from_millis(5));
        // The entry that never returns.
        loop {
            std::thread::park();
        }
    }
    let began = Instant::now();
    let status = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "the_watchdog_ends_a_process_whose_entry_never_returns_with_status_70", "--nocapture", "--test-threads", "1"])
        .env(WATCHED, "1")
        .status()
        .unwrap();
    assert_eq!(status.code(), Some(WATCHDOG_EXIT_STATUS), "{status}");
    println!("the watchdog ended its process {:?} after it started", began.elapsed());
}

/// A field of `/proc/<pid>/<file>` in kibibytes, by its name.
#[cfg(target_os = "linux")]
fn kibibytes(pid: u32, file: &str, field: &str) -> u64 {
    let text = std::fs::read_to_string(format!("/proc/{pid}/{file}")).unwrap();
    let line = text.lines().find(|line| line.starts_with(field)).unwrap_or_else(|| panic!("{file} has no {field}"));
    line.split_whitespace().nth(1).unwrap().parse().unwrap()
}

/// TODO-7292's measurement, and section 11.1's memory limit as the
/// kernel holds it: how long a script host takes from its start to
/// READY (its confinement and the engine's module compiled) and from an
/// opening to its first batch (a container and a context made), and what
/// the process then holds, printed beside DEC-7263's figures for the
/// spike's script process (about 4.2 MB private and 0.29 MB of the
/// kernel's for a tab). The program confines itself on Linux, so it is
/// started here as any program, for its process number.
#[cfg(target_os = "linux")]
#[test]
fn a_script_hosts_start_and_memory_are_measured_and_its_limit_is_the_kernels() {
    use std::process::{Command, Stdio};
    let began = Instant::now();
    let mut child = Command::new(env!("CARGO_BIN_EXE_neoteletext-script-host")).stdin(Stdio::piped()).stdout(Stdio::piped()).spawn().unwrap();
    let pid = child.id();
    let mut link = ProcessLink::new(BufReader::new(child.stdout.take().unwrap()), child.stdin.take().unwrap());
    let source = b"let n = 0; nt.addEventListener('tick', () => { n++; });";
    let mut open = opening_of(ContextId::Page, source);
    open.tick_divisor = 1;
    link.open(open, source).unwrap();
    let opened = Instant::now();
    let first = link.step(tick(1, 0));
    let answered = Instant::now();
    // The limit, as the kernel reports it for the running process.
    let limits = std::fs::read_to_string(format!("/proc/{pid}/limits")).unwrap();
    let data = limits.lines().find(|line| line.starts_with("Max data size")).unwrap();
    let held: Vec<&str> = data.split_whitespace().collect();
    assert_eq!(held.get(3..5), Some(&["134217728", "134217728"][..]), "{data}");
    if !starved(&first) {
        first.unwrap();
        for sequence in 2..62 {
            link.step(tick(sequence, u64::from(sequence))).unwrap();
        }
    }
    let private = kibibytes(pid, "smaps_rollup", "Private_Dirty:") + kibibytes(pid, "smaps_rollup", "Private_Clean:");
    println!(
        "script host ({}): start to first batch {:?}, of which the opening to its batch {:?}; private {} KiB (dirty {} KiB), resident {} KiB, \
         writable data counted against the 131072 KiB limit {} KiB, page tables {} KiB; DEC-7263's spike: about 4300 KiB private, 300 KiB kernel",
        if cfg!(debug_assertions) { "unoptimised" } else { "release" },
        answered.duration_since(began),
        answered.duration_since(opened),
        private,
        kibibytes(pid, "smaps_rollup", "Private_Dirty:"),
        kibibytes(pid, "status", "VmRSS:"),
        kibibytes(pid, "status", "VmData:"),
        kibibytes(pid, "status", "VmPTE:"),
    );
    link.shutdown();
    assert!(child.wait().unwrap().success());
}
