//! A measurement, run by hand: where the time of a step goes between the
//! decoder asking it and the decoder taking its answer (script-api.md
//! section 3.2 rule 5(b) and section 11.1, `STEP_DEADLINE`; DEC-7357
//! item 3). Each tab is the real decoder's scripts and engine and a real
//! script host, with this test between them as the trusted core is, as in
//! `decoder.rs`; a script that writes a row at every tick is stepped at
//! the page's rate, and for each step the wall-clock time is divided
//! into:
//!
//! - **host waiting:** the script host's threads were ready to run and
//!   had no processor (the kernel's account, `/proc/<pid>/task/*/schedstat`,
//!   second figure);
//! - **host running:** the script host's own processor time (first figure);
//! - **decoder waiting** and **decoder running:** the same two for the
//!   thread that runs the decoder's loop here;
//! - **answer unread:** from the answer's arrival on the pipe to the
//!   decoder's loop taking it;
//! - the rest, which is the pipes and both sides asleep.
//!
//! ```text
//! MEASURE_TABS=8 MEASURE_SECONDS=10 cargo test -p neoteletext-script-host --test deadline -- --ignored --nocapture
//! ```
#![cfg(target_os = "linux")]

use std::io::{BufReader, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{Receiver, RecvTimeoutError, channel};
use std::time::{Duration, Instant};

use decoder::Engine;
use decoder::scripts::Scripts;
use protocol::ProtocolError;
use protocol::decoder::FromDecoder;
use protocol::decoder::script::{ChannelFacts, ScriptOrder, ScriptReport, SourcePiece};
use protocol::rules::PageSub;
use protocol::script::relay::{DecoderOutput, HostOutput, LinkKind, read_decoder_output, read_host_output};
use protocol::script::{ContextId, FromScriptHost, read_to_decoder};

/// How long the decoder's loop waits for the script host between two
/// wakes, as `decoder.rs` does: less than a tick.
const TURN: Duration = Duration::from_millis(5);
/// The steps of each tab printed whole, slowest first.
const SLOWEST_SHOWN: usize = 3;

/// The kernel's account of some threads: nanoseconds on a processor, and
/// nanoseconds ready to run without one.
#[derive(Clone, Copy, Default)]
struct Account {
    running: u64,
    waiting: u64,
}

impl Account {
    fn of(path: &str) -> Self {
        let text = std::fs::read_to_string(path).unwrap_or_default();
        let mut figures = text.split_whitespace().map(|figure| figure.parse::<u64>().unwrap_or(0));
        Self { running: figures.next().unwrap_or(0), waiting: figures.next().unwrap_or(0) }
    }

    /// Every thread of the process `pid`.
    fn of_process(pid: u32) -> Self {
        let mut sum = Self::default();
        if let Ok(tasks) = std::fs::read_dir(format!("/proc/{pid}/task")) {
            for task in tasks.flatten() {
                let one = Self::of(&format!("{}/schedstat", task.path().display()));
                sum.running += one.running;
                sum.waiting += one.waiting;
            }
        }
        sum
    }

    fn own_thread() -> Self {
        Self::of("/proc/thread-self/schedstat")
    }

    fn since(self, earlier: Self) -> (Duration, Duration) {
        (Duration::from_nanos(self.running.saturating_sub(earlier.running)), Duration::from_nanos(self.waiting.saturating_sub(earlier.waiting)))
    }
}

/// Something asked of the script host and not yet answered.
struct Asked {
    at: Instant,
    host: Account,
    decoder: Account,
    /// It is the first thing asked after an opening.
    start: bool,
}

/// One answered step, divided.
#[derive(Clone, Copy)]
struct Split {
    total: Duration,
    host_waiting: Duration,
    host_running: Duration,
    decoder_waiting: Duration,
    decoder_running: Duration,
    unread: Duration,
    start: bool,
    /// The decoder stopped the script at this step instead of taking an
    /// answer.
    stopped: bool,
}

impl std::fmt::Display for Split {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let ms = |time: Duration| time.as_secs_f64() * 1000.0;
        write!(
            f,
            "{:8.1} ms = host waiting {:7.1} + host running {:7.1} | decoder waiting {:7.1}, decoder running {:7.1}, answer unread {:7.1}{}{}",
            ms(self.total),
            ms(self.host_waiting),
            ms(self.host_running),
            ms(self.decoder_waiting),
            ms(self.decoder_running),
            ms(self.unread),
            if self.start { "  (a start)" } else { "" },
            if self.stopped { "  STOPPED by the decoder" } else { "" },
        )
    }
}

/// What one tab gave: its answers divided, and its READY.
type Ran = (Vec<Split>, Option<(Duration, Account)>);

/// A message of the script host, when it arrived and the host's account
/// then.
type Arrived = (Result<FromScriptHost, ProtocolError>, Instant, Account);

struct Tab {
    scripts: Scripts,
    engine: Engine,
    host: Child,
    to_host: ChildStdin,
    from_host: Receiver<Arrived>,
    asked: std::collections::VecDeque<Asked>,
    opening: bool,
    splits: Vec<Split>,
    /// When the script host was started.
    started: Instant,
    /// Its READY: how long after its start, and its account then, which
    /// is of the process's whole life. Before READY it confines itself
    /// and compiles the engine's module and the prelude's bytecode;
    /// after it come the container, the prelude's run and the script.
    ready: Option<(Duration, Account)>,
}

impl Tab {
    fn new() -> Self {
        // The program confines itself on Linux, so it is started as any
        // program, for its process number.
        let started = Instant::now();
        let mut host =
            Command::new(env!("CARGO_BIN_EXE_neoteletext-script-host")).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().unwrap();
        let pid = host.id();
        let mut output = BufReader::new(host.stdout.take().unwrap());
        let (said, from_host) = channel();
        std::thread::spawn(move || {
            while let Ok(Some(HostOutput::ToDecoder(relayed))) = read_host_output(&mut output) {
                let arrived = (Instant::now(), Account::of_process(pid));
                let read = read_to_decoder(&mut relayed.frame()).and_then(|message| message.ok_or(ProtocolError::BadValue("an empty frame")));
                if said.send((read, arrived.0, arrived.1)).is_err() {
                    return;
                }
            }
        });
        let mut engine = Engine::new();
        engine.feed(b"PN,10000\nSC,0000\nPS,8000\nOL,1,FILE ROW ONE\n");
        engine.finish();
        let to_host = host.stdin.take().unwrap();
        Self {
            scripts: Scripts::new(),
            engine,
            host,
            to_host,
            from_host,
            asked: std::collections::VecDeque::new(),
            opening: false,
            splits: Vec::new(),
            started,
            ready: None,
        }
    }

    fn pass(&mut self, out: &[u8]) -> bool {
        let mut stopped = false;
        let mut input = out;
        while let Some(message) = read_decoder_output(&mut input).unwrap() {
            match message {
                DecoderOutput::Core(FromDecoder::Script(ScriptReport::Stopped { text, .. })) => {
                    stopped = true;
                    eprintln!("    stopped: {text}");
                }
                DecoderOutput::Core(_) => {}
                DecoderOutput::ToHost(relayed) => {
                    match relayed.link_kind() {
                        Some(LinkKind::ContextOpen | LinkKind::Source) => self.opening = true,
                        Some(LinkKind::Step | LinkKind::ContextClose) => {
                            let start = std::mem::take(&mut self.opening);
                            self.asked.push_back(Asked {
                                at: Instant::now(),
                                host: Account::of_process(self.host.id()),
                                decoder: Account::own_thread(),
                                start,
                            });
                        }
                        _ => {}
                    }
                    let _ = self.to_host.write_all(relayed.frame()).and_then(|()| self.to_host.flush());
                }
            }
        }
        stopped
    }

    fn split(&self, asked: &Asked, arrived: Option<(Instant, Account)>, stopped: bool) -> Split {
        let now = Instant::now();
        let (host_then, at) = arrived.map_or_else(|| (Account::of_process(self.host.id()), now), |(at, host)| (host, at));
        let (host_running, host_waiting) = host_then.since(asked.host);
        let (decoder_running, decoder_waiting) = Account::own_thread().since(asked.decoder);
        Split {
            total: now.duration_since(asked.at),
            host_waiting,
            host_running,
            decoder_waiting,
            decoder_running,
            unread: now.duration_since(at),
            start: asked.start,
            stopped,
        }
    }

    /// Opens the page and runs the decoder's loop for `time`, or until
    /// the decoder stops the script.
    fn run(&mut self, time: Duration) {
        let page = PageSub::new(0x100, 0);
        let script = br#"/*! nt {"api":1,"tick":1} */
            let ticks = 0;
            nt.addEventListener("tick", () => { ticks += 1; nt.teletext.page.put(5 + ticks % 10, 0, "TICK " + ticks); });"#;
        let mut out = Vec::new();
        let facts = ChannelFacts { channel_url: "https://example.test/harbour/".into(), channel_title: "Harbour".into(), ..ChannelFacts::default() };
        self.scripts.order(ScriptOrder::Channel(facts), &mut self.engine, &mut out).unwrap();
        let piece =
            SourcePiece { level: ContextId::Page, page, name: "100.js".into(), total: u32::try_from(script.len()).unwrap(), offset: 0, bytes: script.to_vec() };
        self.scripts.order(ScriptOrder::Source(piece), &mut self.engine, &mut out).unwrap();
        self.scripts.page_opened(Some(page), &mut self.engine, &mut out).unwrap();
        self.pass(&out);
        let began = Instant::now();
        while began.elapsed() < time {
            let mut out = Vec::new();
            self.scripts.wake(&mut self.engine, &mut out).unwrap();
            let mut arrived = None;
            match self.from_host.recv_timeout(TURN) {
                Ok((message, at, host)) => {
                    if matches!(message, Ok(FromScriptHost::Batch(_))) {
                        arrived = Some((at, host));
                    }
                    if matches!(message, Ok(FromScriptHost::Ready { .. })) {
                        self.ready = Some((at.duration_since(self.started), host));
                    }
                    self.scripts.received(message, &mut self.engine, &mut out).unwrap();
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => break,
            }
            if let Some(arrived) = arrived
                && let Some(asked) = self.asked.pop_front()
            {
                let split = self.split(&asked, Some(arrived), false);
                self.splits.push(split);
            }
            if self.pass(&out) {
                // The answer the decoder did not wait for, if it has come.
                let arrived = self.from_host.try_recv().ok().map(|(_, at, host)| (at, host));
                if let Some(asked) = self.asked.pop_front() {
                    let mut split = self.split(&asked, arrived, true);
                    if arrived.is_none() {
                        split.unread = Duration::ZERO;
                    }
                    self.splits.push(split);
                }
                break;
            }
        }
        let _ = self.host.kill();
        let _ = self.host.wait();
    }
}

fn load() -> String {
    std::fs::read_to_string("/proc/loadavg").unwrap_or_default().split_whitespace().take(3).collect::<Vec<_>>().join(" ")
}

#[test]
#[ignore = "a measurement, run by hand: MEASURE_TABS=8 MEASURE_SECONDS=10 cargo test -p neoteletext-script-host --test deadline -- --ignored --nocapture"]
fn where_a_steps_time_goes() {
    let number = |name: &str, otherwise: u64| std::env::var(name).ok().and_then(|value| value.parse().ok()).unwrap_or(otherwise);
    let tabs = number("MEASURE_TABS", 1);
    let time = Duration::from_secs(number("MEASURE_SECONDS", 10));
    let before = load();
    let ran: Vec<Ran> = std::thread::scope(|scope| {
        let running: Vec<_> = (0..tabs)
            .map(|_| {
                scope.spawn(move || {
                    let mut tab = Tab::new();
                    tab.run(time);
                    (tab.splits, tab.ready)
                })
            })
            .collect();
        running.into_iter().map(|tab| tab.join().unwrap()).collect()
    });
    let build = if cfg!(debug_assertions) { "debug" } else { "release" };
    println!(
        "== {build}, {tabs} tab(s) for {time:?}; load before {before}, after {}; {} processors",
        load(),
        std::thread::available_parallelism().map_or(0, usize::from)
    );
    let readies: Vec<(Duration, Account)> = ran.iter().filter_map(|(_, ready)| *ready).collect();
    let ran: Vec<Vec<Split>> = ran.into_iter().map(|(splits, _)| splits).collect();
    let mut steps: Vec<Split> = ran.iter().flatten().copied().filter(|split| !split.start).collect();
    let starts: Vec<Split> = ran.iter().flatten().copied().filter(|split| split.start).collect();
    steps.sort_by_key(|split| split.total);
    let at = |share: usize| steps.get((steps.len() * share / 1000).min(steps.len().saturating_sub(1))).map_or(Duration::ZERO, |split| split.total);
    let over = |limit: u64| steps.iter().filter(|split| split.total >= Duration::from_millis(limit)).count();
    println!(
        "steps: {}; half within {:?}, 99 in 100 within {:?}, 999 in 1000 within {:?}, slowest {:?}; over 50 ms {}, over 100 ms {}, over 250 ms {}, over 1 s {}; stopped by the decoder {}",
        steps.len(),
        at(500),
        at(990),
        at(999),
        steps.last().map_or(Duration::ZERO, |split| split.total),
        over(50),
        over(100),
        over(250),
        over(1000),
        ran.iter().flatten().filter(|split| split.stopped).count()
    );
    let sum = |part: fn(&Split) -> Duration, of: &[Split]| of.iter().map(part).sum::<Duration>();
    let late: Vec<Split> = steps.iter().copied().filter(|split| split.total >= Duration::from_millis(100) || split.stopped).collect();
    for (name, of) in [("all steps", steps.as_slice()), ("steps of 100 ms and more, and the stopped", late.as_slice())] {
        let total = sum(|split| split.total, of).as_secs_f64().max(f64::MIN_POSITIVE);
        let share = |part: fn(&Split) -> Duration| 100.0 * sum(part, of).as_secs_f64() / total;
        println!(
            "{name} ({}): of their time, host waiting for a processor {:.0}%, host running {:.0}%; decoder's thread waiting {:.0}%, running {:.0}%; answer unread {:.0}%",
            of.len(),
            share(|split| split.host_waiting),
            share(|split| split.host_running),
            share(|split| split.decoder_waiting),
            share(|split| split.decoder_running),
            share(|split| split.unread),
        );
    }
    println!("from the process's start to its READY (confined, the engine's module and the prelude's bytecode compiled):");
    for (after, account) in &readies {
        let (running, waiting) = account.since(Account::default());
        println!(
            "  {:8.1} ms = host waiting {:7.1} + host running {:7.1}",
            after.as_secs_f64() * 1000.0,
            waiting.as_secs_f64() * 1000.0,
            running.as_secs_f64() * 1000.0
        );
    }
    println!(
        "starts (the opening, sent at the process's start, to its first answer: READY's time above, then the container, the prelude's run and the script):"
    );
    for split in &starts {
        println!("  {split}");
    }
    println!("slowest steps and every stop:");
    for split in steps.iter().rev().take(SLOWEST_SHOWN) {
        println!("  {split}");
    }
    for split in steps.iter().filter(|split| split.stopped) {
        println!("  {split}");
    }
}
