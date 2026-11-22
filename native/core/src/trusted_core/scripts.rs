//! The script hosts and the relay (`neoteletext-common/docs/script-api.md`
//! section 11.1; DEC-7266, TODO-7292): a channel's scripts run in a
//! confined process of their own, which the broker starts for the trusted
//! core, one for each (tab, channel), which is each of the core's
//! channels. The core stands between that process, the channel's decoder
//! and the partition's web client, and relays their messages **by kind
//! and length alone**: it reads no payload of the script link's or the
//! web client's messages, as it reads no page's bytes
//! (`protocol::script::relay`).
//!
//! - **Started** when the channel's decoder writes the script link's
//!   HELLO, which is how it reports that a script is to run.
//! - **Ended** when the decoder says SHUTDOWN, when the decoder itself is
//!   ended or replaced (its channel sleeps, or leaves its tab), and when
//!   the browser ends.
//! - **Killed** when it writes a kind it may not, when what it asks of
//!   the network was not allowed it, when a step or a close is not
//!   answered by its deadline ([`STEP_DEADLINE`], or [`START_DEADLINE`]
//!   after an opening), when more than [`MAX_STEPS_UNANSWERED`] wait, when
//!   it stops reading and what waits for it passes the bound on a
//!   process's pipe (`neoteletext::outbox::MAX_QUEUED_BYTES`), and when
//!   it says FAILED, after which a script host has nothing more to do.
//! - **Not started again.** A channel whose script host was killed, ended
//!   by itself or said FAILED has its scripts stopped until the viewer
//!   reloads, which makes a new channel (section 3.1). Its decoder is
//!   told with a FAILED of the trusted core's own, the only message of
//!   the script link the core ever writes.
//!
//! The deadlines are counted by kind: every STEP and every
//! `CONTEXT_CLOSE` relayed to a script host is answered by exactly one
//! BATCH (`native/script-host/src/lib.rs`).

use std::collections::{BTreeMap, VecDeque};
use std::io::Write;
use std::sync::Arc;
use std::sync::mpsc::Sender;
use std::time::Instant;

use confine::broker::{Broker, Exit, Helper, Program};
use neoteletext::outbox::Outbox;
use protocol::console::Level;
use protocol::script::relay::{HostOutput, LinkKind, MAX_STEPS_UNANSWERED, Relayed, START_DEADLINE, STEP_DEADLINE, read_host_output};
use protocol::script::{ContextId, FailReason, FromScriptHost, MAX_LOG_CHARS, write_to_decoder};

use super::processes::{Decoder, end, end_aside, relay, spawn};
use super::{Core, Event};

mod reports;
mod web;

/// The most script hosts alive at once, browser-wide (the
/// specification's `MAX_SCRIPT_HOSTS`, section 10.6; DEC-7282 item 8):
/// one for each tab there may be.
pub(super) const MAX_SCRIPT_HOSTS: usize = protocol::MAX_TABS;
// Each is a helper of the broker's, with the decoders, the fetchers and
// the web clients beside it.
const _: () = assert!(MAX_SCRIPT_HOSTS + protocol::web::MAX_WEB_CLIENTS + 2 * protocol::MAX_TABS < confine::broker::MAX_HELPERS);

/// The exit status of a script host its own watchdog ended, because an
/// entry into the script engine was still running past its limit
/// (section 3.2 rule 5(b)): the script library's `WATCHDOG_EXIT_STATUS`,
/// which this program does not link (requirement 1.3); a test of the
/// script-host program holds the two equal.
pub(super) const WATCHDOG_EXIT_STATUS: i32 = 70;

/// How long a script host that was told to end has before it is killed:
/// it has only to read one message and leave.
const END_WAIT: std::time::Duration = std::time::Duration::from_millis(200);

/// One channel's script host.
struct ScriptHost {
    child: Helper,
    input: Outbox,
    /// Which of the channel's script hosts this is.
    generation: u64,
    /// The decoder it serves, by its generation: a decoder that replaces
    /// that one knows nothing of this process, which is ended with it.
    decoder: u64,
    /// It has said READY.
    ready: bool,
    /// When each step and close relayed to it and not yet answered was
    /// relayed, oldest first.
    asked: VecDeque<Instant>,
    /// When the last piece of an opening was relayed to it, while
    /// anything waits: a context's start may take [`START_DEADLINE`].
    opening: Option<Instant>,
    /// The decoder reported that a context of it has the network
    /// (section 11.1); without that none of its requests is relayed.
    network: bool,
}

impl ScriptHost {
    /// When it must have answered the oldest thing asked, if anything
    /// waits.
    fn due(&self) -> Option<Instant> {
        let asked = self.asked.front()?.checked_add(STEP_DEADLINE)?;
        let opening = self.opening.and_then(|at| at.checked_add(START_DEADLINE));
        Some(opening.map_or(asked, |opening| asked.max(opening)))
    }

    /// Ends it at once.
    fn kill(mut self) {
        drop(self.input);
        drop(self.child.stdin.take());
        end_aside(self.child);
    }
}

/// What the trusted strip may say of a channel's scripts (section 3.1;
/// the indication itself is work package F5's).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(not(test), expect(dead_code, reason = "read by the trusted strip's indication, which work package F5 builds (TODO-7294)"))]
pub(super) enum ScriptState {
    /// The channel has no script host.
    None,
    /// Its scripts run.
    Running,
    /// Its scripts were stopped, for this reason, until it is reloaded.
    Stopped(FailReason),
}

/// Every channel's script host, and the web clients their requests go to.
#[derive(Default)]
pub(super) struct Scripts {
    hosts: BTreeMap<u32, ScriptHost>,
    /// The channels whose scripts were stopped, and why: none is started
    /// for them again.
    stopped: BTreeMap<u32, FailReason>,
    /// What each channel's scripts claim, as its decoder last reported.
    claims: BTreeMap<u32, protocol::decoder::script::ScriptClaims>,
    /// The decoder of each channel that was told what its scripts'
    /// contexts are told, by its generation: a decoder that replaces it
    /// is told again before its first script.
    told: BTreeMap<u32, u64>,
    /// The scripts each channel's decoder was given, whole, for a decoder
    /// that replaces it: the channel's own and its newest pages'.
    kept: BTreeMap<u32, Vec<reports::Kept>>,
    web: web::WebClients,
}

/// `text` as a line of the developer log, which a FAILED carries.
fn log_line(text: &str) -> String {
    text.chars().filter(|character| !character.is_control()).take(MAX_LOG_CHARS).collect()
}

/// Whether `exit` is a script host's end by its own watchdog: the text
/// the system gives the status, which is all the broker passes on, is
/// compared with the text it gives that status, so nothing is parsed.
fn ended_by_its_watchdog(exit: &Exit) -> bool {
    #[cfg(unix)]
    let status = {
        use std::os::unix::process::ExitStatusExt as _;
        // wait(2): an exit status is the second byte of what `wait` gives.
        std::process::ExitStatus::from_raw(WATCHDOG_EXIT_STATUS << 8)
    };
    #[cfg(windows)]
    let status = {
        use std::os::windows::process::ExitStatusExt as _;
        std::process::ExitStatus::from_raw(WATCHDOG_EXIT_STATUS.unsigned_abs())
    };
    !exit.success && exit.text == Exit::from(status).text
}

/// Starts a script host for `channel`, whose messages arrive as events.
fn start_host(broker: &Arc<Broker>, channel: u32, generation: u64, decoder: u64, events: Sender<Event>) -> Result<ScriptHost, String> {
    let mut child = spawn(broker, Program::ScriptHost, &[], false)?;
    let output = child.stdout.take().ok_or("the script host has no output")?;
    let input = Outbox::new(child.stdin.take().ok_or("the script host has no input")?);
    relay(
        output,
        read_host_output,
        events,
        move |said| Event::ScriptSaid(channel, generation, said),
        move |end| Event::ScriptHostEnded(channel, generation, end),
    );
    Ok(ScriptHost { child, input, generation, decoder, ready: false, asked: VecDeque::new(), opening: None, network: false })
}

impl Core {
    /// What the strip may say of `channel`'s scripts.
    #[cfg_attr(not(test), expect(dead_code, reason = "read by the trusted strip's indication, which work package F5 builds (TODO-7294)"))]
    pub(super) fn script_state(&self, channel: u32) -> ScriptState {
        match (self.scripts.stopped.get(&channel), self.scripts.hosts.contains_key(&channel)) {
            (Some(reason), _) => ScriptState::Stopped(*reason),
            (None, true) => ScriptState::Running,
            (None, false) => ScriptState::None,
        }
    }

    /// The decoder of `channel` reported whether a context of its scripts
    /// has the network (section 11.1: "it relays the network kinds only
    /// for a context the decoder reported with `network: true` and only
    /// while `ChannelPolicy::network_allowed()` holds"). A web channel
    /// and a page opened from disk each have a partition to ask through
    /// (DEC-7315 version 2); a channel with none has no network whatever
    /// its decoder says.
    pub(super) fn script_network(&mut self, channel: u32, reported: bool) {
        let served = self.partition_of(channel).is_some();
        if let Some(host) = self.scripts.hosts.get_mut(&channel) {
            host.network = reported && served;
        }
    }

    /// A decoder wrote a message of the script link for its script host.
    pub(super) fn script_asked(&mut self, channel: u32, decoder: u64, asked: &Relayed) {
        if self.decoders.get(&channel).map(Decoder::generation) != Some(decoder) {
            return;
        }
        let kind = asked.link_kind();
        // A channel whose scripts were stopped is told so again when its
        // decoder, or a new decoder of it, greets; nothing is started.
        if self.scripts.stopped.contains_key(&channel) {
            if kind == Some(LinkKind::Hello) {
                self.tell_scripts_stopped(channel, "the scripts of this channel were stopped and stay stopped until it is reloaded");
            }
            return;
        }
        if kind == Some(LinkKind::Hello) && !self.scripts.hosts.contains_key(&channel) {
            if self.scripts.hosts.len() >= MAX_SCRIPT_HOSTS {
                return self.stop_scripts(channel, FailReason::Memory, "MAX_SCRIPT_HOSTS: as many script processes run as the browser allows");
            }
            self.generation = self.generation.wrapping_add(1);
            let started = self.broker().map(Arc::clone).and_then(|broker| start_host(&broker, channel, self.generation, decoder, self.events.clone()));
            match started {
                Ok(host) => {
                    self.note(Level::Info, Some(channel), &format!("script host: confined: {}", host.child.report));
                    self.scripts.hosts.insert(channel, host);
                }
                Err(error) => return self.stop_scripts(channel, FailReason::Link, &error),
            }
        }
        let Some(host) = self.scripts.hosts.get_mut(&channel) else { return };
        let now = Instant::now();
        match kind {
            Some(LinkKind::Step | LinkKind::ContextClose) => host.asked.push_back(now),
            Some(LinkKind::ContextOpen | LinkKind::Source) => host.opening = Some(now),
            _ => {}
        }
        let behind = host.asked.len() > MAX_STEPS_UNANSWERED;
        let passed = host.input.write_all(asked.frame()).and_then(|()| host.input.flush());
        if behind {
            return self.stop_scripts(channel, FailReason::Time, "the script host fell too many steps behind and was ended");
        }
        if passed.is_err() {
            return self.stop_scripts(channel, FailReason::Link, "the script host stopped reading and was ended");
        }
        if kind == Some(LinkKind::Shutdown) {
            self.end_script_host(channel);
        }
    }

    /// A script host wrote a message: for its decoder, or for its web
    /// client. Its payload is not read.
    pub(super) fn script_said(&mut self, channel: u32, generation: u64, said: HostOutput) {
        let Some(host) = self.scripts.hosts.get_mut(&channel).filter(|host| host.generation == generation) else { return };
        match said {
            HostOutput::ToDecoder(relayed) => {
                match relayed.link_kind() {
                    Some(LinkKind::Ready) => host.ready = true,
                    Some(LinkKind::Batch) => {
                        host.asked.pop_front();
                        if host.asked.is_empty() {
                            host.opening = None;
                        }
                    }
                    _ => {}
                }
                let decoder = host.decoder;
                if let Some(decoder) = self.decoders.get_mut(&channel).filter(|running| running.generation() == decoder) {
                    // A decoder that cannot be written to has stopped; its
                    // reader reports that.
                    let _ = decoder.pass(relayed.frame());
                }
                // A script host that said FAILED has stopped its container
                // for good and has nothing more to do (section 3.1).
                if relayed.link_kind() == Some(LinkKind::Failed) {
                    self.scripts_stopped(channel, FailReason::Trap, "the script host said its scripts stopped", false);
                }
            }
            HostOutput::Request(relayed) => {
                if !host.network {
                    return self.stop_scripts(channel, FailReason::Link, "the script host asked for the network, which its scripts do not have, and was ended");
                }
                if let Err(error) = self.relay_request(channel, relayed) {
                    self.stop_scripts(channel, FailReason::Link, &error);
                }
            }
        }
    }

    /// A script host's output ended, or broke the protocol.
    pub(super) fn script_host_ended(&mut self, channel: u32, generation: u64, reason: Option<String>) {
        if self.scripts.hosts.get(&channel).map(|host| host.generation) != Some(generation) {
            // One the trusted core ended itself has nothing more to say.
            return;
        }
        if let Some(reason) = reason {
            return self.stop_scripts(channel, FailReason::Link, &format!("the script host wrote what it may not and was ended: {reason}"));
        }
        // It ended untold. How says why: its watchdog's status is a time
        // limit passed. It is waited for aside, since the broker holds its
        // one channel through a wait.
        let Some(mut host) = self.scripts.hosts.remove(&channel) else { return };
        self.web_tab_closes(channel);
        drop(host.input);
        drop(host.child.stdin.take());
        let events = self.events.clone();
        let decoder = host.decoder;
        std::thread::spawn(move || {
            let _ = host.child.kill();
            let exit = host.child.wait().unwrap_or_else(|error| Exit { success: false, text: error.to_string() });
            let _ = events.send(Event::ScriptHostExited(channel, decoder, exit));
        });
    }

    /// A script host that ended untold has been waited for.
    pub(super) fn script_host_exited(&mut self, channel: u32, decoder: u64, exit: &Exit) {
        if self.scripts.stopped.contains_key(&channel) || self.decoders.get(&channel).map(Decoder::generation) != Some(decoder) {
            return;
        }
        if ended_by_its_watchdog(exit) {
            self.scripts_stopped(channel, FailReason::Time, "a script ran past its time limit and its process was ended by its watchdog", true);
        } else {
            self.scripts_stopped(channel, FailReason::Link, &format!("the script host ended ({exit})"), true);
        }
    }

    /// Kills `channel`'s script host and stops its scripts until the
    /// channel is reloaded.
    fn stop_scripts(&mut self, channel: u32, reason: FailReason, text: &str) {
        self.scripts_stopped(channel, reason, text, true);
    }

    /// Notes that `channel`'s scripts have stopped for good, ends its
    /// script host if it still runs, and, with `tell`, tells its decoder.
    fn scripts_stopped(&mut self, channel: u32, reason: FailReason, text: &str, tell: bool) {
        if let Some(host) = self.scripts.hosts.remove(&channel) {
            host.kill();
        }
        self.web_tab_closes(channel);
        if self.scripts.stopped.insert(channel, reason).is_some() {
            return;
        }
        self.log(Level::Error, Some(channel), &format!("the scripts of channel {channel} were stopped: {text}"));
        if tell {
            self.tell_scripts_stopped_for(channel, reason, text);
        }
    }

    /// Tells `channel`'s decoder that its scripts are stopped, for the
    /// reason kept.
    fn tell_scripts_stopped(&mut self, channel: u32, text: &str) {
        let reason = self.scripts.stopped.get(&channel).copied().unwrap_or(FailReason::Link);
        self.tell_scripts_stopped_for(channel, reason, text);
    }

    /// The one message of the script link the trusted core writes: a
    /// FAILED of its own, for the container as a whole, which the channel
    /// context stands for.
    fn tell_scripts_stopped_for(&mut self, channel: u32, reason: FailReason, text: &str) {
        let mut frame = Vec::new();
        if write_to_decoder(&mut frame, &FromScriptHost::Failed { context: ContextId::Channel, reason, text: log_line(text) }).is_ok()
            && let Some(decoder) = self.decoders.get_mut(&channel)
        {
            let _ = decoder.pass(&frame);
        }
    }

    /// Ends `channel`'s script host, told or about to be: its input is
    /// closed, and it is killed if it has not left by [`END_WAIT`]. The
    /// channel's scripts are not marked stopped: a decoder that starts
    /// again starts them again.
    fn end_script_host(&mut self, channel: u32) {
        let Some(mut host) = self.scripts.hosts.remove(&channel) else { return };
        self.web_tab_closes(channel);
        drop(host.input);
        drop(host.child.stdin.take());
        std::thread::spawn(move || {
            std::thread::sleep(END_WAIT);
            end(&mut host.child);
        });
    }

    /// When the first script host must have answered, if any is asked.
    pub(super) fn scripts_due_at(&self) -> Option<Instant> {
        self.scripts.hosts.values().filter_map(ScriptHost::due).min()
    }

    /// The scripts' timed work, and their keeping in step with the
    /// decoders: a script host past its deadline is killed (section
    /// 11.1), and one whose decoder was ended or replaced is ended with
    /// it.
    pub(super) fn scripts_tick(&mut self, now: Instant) {
        let late: Vec<u32> = self.scripts.hosts.iter().filter(|(_, host)| host.due().is_some_and(|due| now >= due)).map(|(&channel, _)| channel).collect();
        for channel in late {
            self.stop_scripts(channel, FailReason::Time, "STEP_DEADLINE: the script host did not answer in time and was ended");
        }
        let orphaned: Vec<u32> = self
            .scripts
            .hosts
            .iter()
            .filter(|(channel, host)| self.decoders.get(channel).map(Decoder::generation) != Some(host.decoder))
            .map(|(&channel, _)| channel)
            .collect();
        for channel in orphaned {
            self.end_script_host(channel);
        }
        // A channel no tab holds any more is forgotten: a reload made a
        // new one.
        if !self.scripts.stopped.is_empty() {
            let open: Vec<u32> = self.session.all_tabs().map(|tab| tab.channel.id).collect();
            self.scripts.stopped.retain(|channel, _| open.contains(channel));
        }
    }

    /// Ends every script host and web client: the browser is ending.
    pub(super) fn end_scripts(&mut self) {
        for (_, host) in std::mem::take(&mut self.scripts.hosts) {
            host.kill();
        }
        self.scripts.web.end_all();
    }
}

#[cfg(all(test, unix, feature = "interaction"))]
mod loading_tests;
#[cfg(all(test, unix))]
mod tests;
