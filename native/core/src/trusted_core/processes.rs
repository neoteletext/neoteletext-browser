//! The helper processes the trusted core starts through the broker
//! (decision 56): a decoder, a fetcher, the bookmark reader, the
//! interface's pipe, and where a fetcher may connect.

use std::collections::BTreeMap;
use std::io::{BufReader, Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;

use confine::broker::{Broker, Helper, Program};
use interface_core::canvas::PictureLanguage;
use neoteletext::budget::Budget;
use neoteletext::outbox::Outbox;
use neoteletext::settings::{self as stored, Settings};
use protocol::decoder::{BookmarkEntry, FromDecoder, MAX_BOOKMARK_ENTRIES, MAX_CHUNK, ToDecoder, read_from_decoder, write_to_decoder};
use protocol::fetcher::{FETCHER_END_WAIT, FromFetcher, MAX_FETCHER_TABS, ToFetcher, read_from_fetcher_for, write_to_fetcher};
use protocol::{ProtocolError, read_from_interface};

use super::Event;

pub(super) struct Decoder {
    child: Helper,
    input: Outbox,
    /// Which of the channel's decoders this is: one put to sleep may still
    /// report after the next has started, and is then not listened to.
    generation: u64,
    /// It has said it is confined, its first message: an end before then
    /// is a failure to start, counted against the version (finding F-12).
    heard: bool,
}

impl Decoder {
    /// Its processor time since it started, from the operating system's
    /// account as the broker reads it (REQ-6626 item 1 a).
    pub(super) fn processor_time_us(&self) -> std::io::Result<u64> {
        self.child.processor_time_us()
    }
}

/// A tab's place in a fetcher process (REQ-6980): one process serves every
/// tab of one channel in one jar, and each tab has a place in it, with a
/// tab number that names its messages on the process's two pipes.
pub(super) struct Fetcher {
    process: Arc<Mutex<FetcherProcess>>,
    /// The number that names this tab's messages, never used twice in one
    /// process.
    tab: u32,
    /// As [`Decoder::generation`].
    generation: u64,
    /// What a fetcher is given for this tab: input messages (REQ-INT-133),
    /// the answers to its viewer state challenges (VS-119) and requests
    /// for files; none when the process has no input.
    input: Option<TabPipe>,
}

/// What decides which fetcher process serves a tab (REQ-6980): the jar,
/// and everything the process is started with, which is the channel's
/// address, the local addresses it may reach, the proxy, whether its tabs
/// may share a connection (REQ-6982), and the UDP port its sandbox allows
/// (REQ-6981). Tabs that differ in any are served by different processes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct FetcherKey {
    /// The tab is private: the private jar's (REQ-881).
    private: bool,
    arguments: Vec<String>,
    udp_port: Option<u16>,
}

impl FetcherKey {
    /// The key of a tab of the channel at `address`, private or not,
    /// whose fetcher may connect where `reach` says.
    pub(super) fn of(address: &str, private: bool, reach: &Reach) -> Self {
        let mut arguments = vec![address.to_owned()];
        for ip in &reach.allow_local {
            arguments.push(String::from("--allow-local"));
            arguments.push(ip.clone());
        }
        if let Some(proxy) = &reach.proxy {
            arguments.push(String::from("--proxy"));
            arguments.push(proxy.clone());
        }
        // With sharing off, or a proxy, through which QUIC cannot go
        // (requirement 19.10), the fetcher is told to keep every tab on
        // HTTP/1.1, and its sandbox is given no UDP (REQ-6982, REQ-6981).
        if !reach.shares() {
            arguments.push(String::from("--no-http3"));
        }
        Self { private, arguments, udp_port: reach.udp_port(address) }
    }
}

/// The tabs a fetcher process serves, by their numbers: each one's channel
/// and the generation of its place, which is how its messages become the
/// trusted core's events. The process's reader thread reads it; the
/// trusted core adds and removes tabs.
type Routes = Arc<Mutex<BTreeMap<u32, (u32, u64)>>>;

/// One fetcher process and what its tabs share.
struct FetcherProcess {
    key: Option<FetcherKey>,
    /// None once it was ended.
    child: Option<Helper>,
    /// The one pipe to it, on which every tab's messages go, each after
    /// its tab's number; none when it has no input, or once it was ended.
    input: Option<Outbox>,
    routes: Routes,
    /// The last tab number given.
    last_tab: u32,
    /// As [`Decoder::heard`], said once by the process for all its tabs.
    heard: Arc<AtomicBool>,
    /// It ended unheard, and that has been counted against the version,
    /// once however many tabs it served (finding F-12).
    counted: bool,
    report: confine::Report,
}

fn locked<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// One tab's side of the pipe to its fetcher process: what is written is
/// whole messages without a tab number, and a flush passes each on after
/// the tab's number, so that every caller writes to a tab's fetcher as it
/// did when the tab had a process of its own (REQ-6980).
pub(super) struct TabPipe {
    tab: u32,
    /// Written since the last flush.
    pending: Vec<u8>,
    process: Arc<Mutex<FetcherProcess>>,
}

impl Write for TabPipe {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.pending.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        let mut named = Vec::new();
        protocol::fetcher::name_tab(self.tab, &mut self.pending, &mut named);
        let mut process = locked(&self.process);
        let input = process.input.as_mut().ok_or(std::io::ErrorKind::BrokenPipe)?;
        if !named.is_empty() {
            input.write_all(&named)?;
        }
        input.flush()
    }
}

impl Decoder {
    /// The decoder `child`, of `generation`, written to through `input`,
    /// which has said nothing yet.
    pub(super) fn new(child: Helper, input: Outbox, generation: u64) -> Self {
        Self { child, input, generation, heard: false }
    }

    /// Which of the channel's decoders this is.
    pub(super) fn generation(&self) -> u64 {
        self.generation
    }

    /// Whether it has said it is confined.
    pub(super) fn heard(&self) -> bool {
        self.heard
    }

    /// It said it is confined.
    pub(super) fn hear(&mut self) {
        self.heard = true;
    }

    /// How it was confined.
    pub(super) fn report(&self) -> &confine::Report {
        &self.child.report
    }

    /// Writes `message` to its input, to be sent at the next flush.
    pub(super) fn tell(&mut self, message: &ToDecoder) -> Result<(), ProtocolError> {
        write_to_decoder(&mut self.input, message)
    }

    /// Sends what it was told; an error once it has stopped reading.
    pub(super) fn flush(&mut self) -> std::io::Result<()> {
        self.input.flush()
    }

    /// The bytes told it that it has not read yet.
    pub(super) fn queued(&self) -> usize {
        self.input.queued()
    }

    /// Ends it at once, told nothing: it broke its protocol or ended.
    pub(super) fn end(mut self) {
        end(&mut self.child);
    }
}

impl FetcherProcess {
    /// Gives a tab of `channel` a place of `generation`: its number, by
    /// which the reader thread knows its messages from now on.
    fn add(&mut self, channel: u32, generation: u64) -> u32 {
        // Never the process's own number, and never one used before: a
        // process is given far fewer than 2^32 tabs in its life.
        self.last_tab = self.last_tab.wrapping_add(1).max(protocol::fetcher::PROCESS_TAB.saturating_add(1));
        locked(&self.routes).insert(self.last_tab, (channel, generation));
        self.last_tab
    }

    /// Ends the pipe to it, and takes the process to be ended.
    fn close(&mut self) -> Option<Helper> {
        drop(self.input.take());
        let mut child = self.child.take()?;
        drop(child.stdin.take());
        Some(child)
    }
}

impl Fetcher {
    /// A fetcher process `child` of its own for one tab, whose place is of
    /// `generation`, given `input` if it has one, which has said nothing
    /// yet: a process no other tab joins, as a test makes one.
    #[cfg(test)]
    pub(super) fn new(child: Helper, generation: u64, input: Option<Outbox>) -> Self {
        Self::first(child, None, 0, generation, input, Arc::default())
    }

    /// The first place in the process `child`, for a tab of `channel`.
    fn first(child: Helper, key: Option<FetcherKey>, channel: u32, generation: u64, input: Option<Outbox>, routes: Routes) -> Self {
        let report = child.report.clone();
        let has_input = input.is_some();
        let mut process = FetcherProcess { key, child: Some(child), input, routes, last_tab: 0, heard: Arc::default(), counted: false, report };
        let tab = process.add(channel, generation);
        let process = Arc::new(Mutex::new(process));
        let mut fetcher = Self { input: has_input.then(|| TabPipe { tab, pending: Vec::new(), process: Arc::clone(&process) }), process, tab, generation };
        fetcher.tell(&ToFetcher::TabOpens);
        fetcher
    }

    /// Whether this tab's process serves the tabs of `key` and can take
    /// another: it has not been ended, and holds fewer than
    /// [`MAX_FETCHER_TABS`].
    pub(super) fn serves(&self, key: &FetcherKey) -> bool {
        let process = locked(&self.process);
        process.key.as_ref() == Some(key) && process.input.is_some() && locked(&process.routes).len() < MAX_FETCHER_TABS
    }

    /// A place of `generation` for another tab, of `channel`, in this
    /// tab's process (REQ-6980).
    pub(super) fn join(&self, channel: u32, generation: u64) -> Self {
        let tab = locked(&self.process).add(channel, generation);
        let process = Arc::clone(&self.process);
        let mut fetcher = Self { input: Some(TabPipe { tab, pending: Vec::new(), process: Arc::clone(&process) }), process, tab, generation };
        fetcher.tell(&ToFetcher::TabOpens);
        fetcher
    }

    /// Writes `message` for this tab and passes it on at once; a process
    /// that does not take it is its own to notice.
    fn tell(&mut self, message: &ToFetcher) {
        if let Some(input) = self.input.as_mut() {
            let _ = write_to_fetcher(input, message);
            let _ = input.flush();
        }
    }

    /// As [`Decoder::generation`].
    pub(super) fn generation(&self) -> u64 {
        self.generation
    }

    /// As [`Decoder::heard`]: its process said it is confined.
    #[cfg(test)]
    pub(super) fn heard(&self) -> bool {
        locked(&self.process).heard.load(Ordering::Acquire)
    }

    /// Its process said it is confined.
    pub(super) fn hear(&mut self) {
        locked(&self.process).heard.store(true, Ordering::Release);
    }

    /// Whether its process ended before it said it was confined and that
    /// has not yet been counted: true once for a process, however many
    /// tabs it served (finding F-12).
    pub(super) fn ended_unheard(&self) -> bool {
        let mut process = locked(&self.process);
        !process.heard.load(Ordering::Acquire) && !std::mem::replace(&mut process.counted, true)
    }

    /// How its process was confined.
    pub(super) fn report(&self) -> confine::Report {
        locked(&self.process).report.clone()
    }

    /// Its input: input messages, the answers to its viewer state
    /// challenges and requests for files, each named with this tab as it
    /// is passed on.
    pub(super) fn input(&mut self) -> Option<&mut TabPipe> {
        self.input.as_mut()
    }

    /// The tab leaves its process: the fetcher is told, and sends what
    /// the tab was last told before it closes the tab's connections, the
    /// channel told the tab went (neoteletext-common's DEC-1538 item 3).
    /// The process's last tab ends the process: closing its input ends a
    /// fetcher once every tab's last writes have left, and it is killed
    /// and waited for after that, on a thread of its own, so nothing waits
    /// for its network.
    pub(super) fn stop(mut self) {
        self.tell(&ToFetcher::TabLeaves);
        drop(self.input.take());
        let mut process = locked(&self.process);
        let last = {
            let mut routes = locked(&process.routes);
            routes.remove(&self.tab);
            routes.is_empty()
        };
        if last && let Some(mut child) = process.close() {
            thread::spawn(move || {
                thread::sleep(FETCHER_END_WAIT);
                end(&mut child);
            });
        }
    }

    /// Ends its process at once, told nothing: its output stopped, and
    /// that says nothing of whether it ended. A helper reads a hostile
    /// server's data, so one that broke its protocol or closed its output
    /// may sleep for ever (REQ-683). Every tab of the process hears that
    /// it ended, by the same event.
    pub(super) fn end(mut self) {
        drop(self.input.take());
        let mut process = locked(&self.process);
        locked(&process.routes).remove(&self.tab);
        if let Some(child) = process.close() {
            end_aside(child);
        }
    }
}

/// Where a new fetcher may connect: the local addresses allowed, from the
/// command line and the settings, and the proxy of the settings.
pub(super) struct Reach {
    allow_local: Vec<String>,
    proxy: Option<String>,
    /// A channel's tabs may share one connection (REQ-6982).
    share_connections: bool,
}

impl Reach {
    pub(super) fn of(command_line: &[String], settings: &Settings) -> Self {
        let mut allow_local = command_line.to_vec();
        allow_local.extend(settings.local.iter().map(ToString::to_string));
        Self { allow_local, proxy: settings.proxy.as_ref().map(stored::Proxy::url), share_connections: settings.share_connections }
    }

    /// Whether a fetcher started with this reach may share one HTTP/3
    /// connection among its tabs: the setting is on (REQ-6982) and no proxy
    /// is set, since QUIC would go direct (requirement 19.10).
    fn shares(&self) -> bool {
        self.share_connections && self.proxy.is_none()
    }

    /// The UDP port the fetcher of the channel at `address` may reach, for
    /// its tabs' shared HTTP/3 connection (REQ-6981): the channel's own
    /// port, 443 when its address names none, and none at all when sharing
    /// is off (REQ-6982), when a proxy is set, since nothing goes direct
    /// (requirement 19.10), or when the channel is not served over TLS,
    /// which QUIC always is.
    fn udp_port(&self, address: &str) -> Option<u16> {
        if !self.shares() {
            return None;
        }
        neoteletext_web::url::Url::parse(address).ok().filter(neoteletext_web::url::Url::is_secure).map(|url| url.port())
    }
}

/// Asks the broker for a helper (decision 56): it starts it confined as
/// its program's role says (decision 52), with an environment of only
/// what it needs, and passes the trusted core its pipes.
pub(super) fn spawn(broker: &Arc<Broker>, program: Program, arguments: &[String], local_network: bool) -> Result<Helper, String> {
    let helper = broker.spawn(program, arguments, local_network).map_err(|error| format!("cannot start the {}: {error}", program.name()))?;
    eprintln!("neoteletext: the {} started confined: {}", program.name(), helper.report);
    Ok(helper)
}

/// Ends a decoder: told to close, and killed rather than waited for, so
/// one that has stopped reading holds nothing up. It has nothing the
/// trusted core needs: what it was given is kept (decision 47).
pub(super) fn stop_decoder(mut decoder: Decoder) {
    let _ = write_to_decoder(&mut decoder.input, &ToDecoder::Close);
    drop(decoder.input);
    end(&mut decoder.child);
}

/// Ends a helper: killed, then waited for, so that none is left running or
/// unreaped; what either says is of no use to a helper being given up.
pub(super) fn end(helper: &mut Helper) {
    let _ = helper.kill();
    let _ = helper.wait();
}

/// Ends `helper` on a thread of its own. The broker holds its one channel
/// through a wait, so the event loop never waits for a helper it did not
/// choose to end, whatever that helper does.
pub(super) fn end_aside(mut helper: Helper) {
    thread::spawn(move || end(&mut helper));
}

/// Starts a decoder for `channel`, whose messages arrive as events.
pub(super) fn start_decoder(broker: &Arc<Broker>, channel: u32, generation: u64, store_pages: usize, events: Sender<Event>) -> Result<Decoder, String> {
    let mut child = spawn(broker, Program::Decoder, &[String::from("--store-pages"), store_pages.to_string()], false)?;
    let output = child.stdout.take().ok_or("the decoder has no output")?;
    let input = Outbox::new(child.stdin.take().ok_or("the decoder has no input")?);
    #[cfg(feature = "timed-test")]
    let read_from_decoder = timed_read_from_decoder;
    relay(
        output,
        read_from_decoder,
        events,
        move |message| Event::Decoded(channel, generation, message),
        move |end| Event::DecoderEnded(channel, generation, end),
    );
    Ok(Decoder::new(child, input, generation))
}

/// Starts a decoder to read another browser's bookmark file (decision
/// 71): it is given the file and reports what it holds, which arrives as
/// one event, and is ended.
pub(super) fn start_bookmark_reader(broker: &Arc<Broker>, file: Vec<u8>, events: Sender<Event>) -> Result<(), String> {
    let child = spawn(broker, Program::Decoder, &[String::from("--bookmarks")], false)?;
    read_bookmarks_with(child, file, events, BOOKMARK_READ_DEADLINE)
}

/// How long the bookmark reader has to read a file and say what it holds
/// before it is killed (finding F-85). Its largest file is
/// `MAX_BOOKMARK_FILE`; its reading time is not measured, and ten
/// seconds is chosen as far above what a bounded, single-pass reader of
/// that much should need.
const BOOKMARK_READ_DEADLINE: std::time::Duration = std::time::Duration::from_secs(10);

/// Gives the bookmark reader `child` the file, and sends what it reports,
/// or, once `deadline` has passed with no answer, kills it and says so.
fn read_bookmarks_with(mut child: Helper, file: Vec<u8>, events: Sender<Event>, deadline: std::time::Duration) -> Result<(), String> {
    let output = child.stdout.take().ok_or("the bookmark reader has no output")?;
    let mut input = child.stdin.take().ok_or("the bookmark reader has no input")?;
    let (answer, answered) = std::sync::mpsc::channel();
    // The exchange on a thread of its own, which killing the reader ends.
    thread::spawn(move || {
        let exchange = move || -> Result<(Vec<BookmarkEntry>, u32), String> {
            // The reader takes the whole file before it says anything.
            for chunk in file.chunks(MAX_CHUNK) {
                write_to_decoder(&mut input, &ToDecoder::Data(chunk.to_vec())).map_err(|error| error.to_string())?;
            }
            write_to_decoder(&mut input, &ToDecoder::End).map_err(|error| error.to_string())?;
            let mut output = BufReader::new(output);
            let mut entries = Vec::new();
            loop {
                match read_from_decoder(&mut output).map_err(|error| error.to_string())? {
                    // Each folder's end follows its folder: twice the bound
                    // holds every honest file.
                    Some(FromDecoder::Bookmark(entry)) if entries.len() < 2 * MAX_BOOKMARK_ENTRIES => entries.push(entry),
                    Some(FromDecoder::BookmarksRead { skipped }) => return Ok((entries, skipped)),
                    Some(_) => return Err(String::from("the bookmark reader said what it may not")),
                    None => return Err(String::from("it is not a bookmark file the reader can read")),
                }
            }
        };
        let _ = answer.send(exchange());
    });
    thread::spawn(move || {
        let read = answered.recv_timeout(deadline).unwrap_or_else(|_| Err(String::from("the bookmark reader took too long")));
        end(&mut child);
        let _ = events.send(Event::BookmarksRead(read));
    });
    Ok(())
}

/// Starts a decoder to convert one picture (`docs/picture-tools.md`): it is
/// given the choices and the picture, and its page, or why it made none,
/// arrives as one event of `generation`; then it is ended.
pub(super) fn start_picture_converter(
    broker: &Arc<Broker>,
    picture: Arc<Vec<u8>>,
    choice: interface_core::picture::PictureChoice,
    generation: u64,
    events: Sender<Event>,
) -> Result<(), String> {
    let child = spawn(broker, Program::Decoder, &[String::from("--picture")], false)?;
    convert_picture_with(child, picture, choice, generation, events, PICTURE_DEADLINE)
}

/// Starts a decoder to check one font for pictures the viewer chose
/// (renderer parity PAR-35), gives it the font in parts, and sends what it
/// answers as an event; once [`FONT_DEADLINE`] has passed, kills it and
/// says so.
pub(super) fn start_font_checker(
    broker: &Arc<Broker>,
    language: PictureLanguage,
    font: Arc<Vec<u8>>,
    generation: u64,
    events: Sender<Event>,
) -> Result<(), String> {
    let mut child = spawn(broker, Program::Decoder, &[String::from("--font")], false)?;
    let output = child.stdout.take().ok_or("the font checker has no output")?;
    let mut input = child.stdin.take().ok_or("the font checker has no input")?;
    let (answer, answered) = std::sync::mpsc::channel();
    thread::spawn(move || {
        let exchange = move || -> Result<(), String> {
            for part in protocol::regis::font_parts(language, &font) {
                write_to_decoder(&mut input, &ToDecoder::PictureFont(part)).map_err(|error| error.to_string())?;
            }
            match read_from_decoder(&mut BufReader::new(output)).map_err(|error| error.to_string())? {
                Some(FromDecoder::FontChecked { language: said, refusal }) if said == language => refusal.map_or(Ok(()), Err),
                Some(_) => Err(String::from("the font checker said what it may not")),
                None => Err(String::from("the font checker ended without an answer")),
            }
        };
        let _ = answer.send(exchange());
    });
    thread::spawn(move || {
        let checked = answered.recv_timeout(FONT_DEADLINE).unwrap_or_else(|_| Err(String::from("the font checker took too long")));
        end(&mut child);
        let _ = events.send(Event::FontChecked(generation, checked));
    });
    Ok(())
}

/// How long a font checker has before it is killed: parsing a font at the
/// largest bound, 1 MiB of text, takes milliseconds; ten seconds is far
/// above that and above a confined decoder's start.
const FONT_DEADLINE: std::time::Duration = std::time::Duration::from_secs(10);

/// How long a picture decoder has to convert before it is killed: a
/// conversion took at most 1.5 seconds at Level 3.5 in a release build on 3
/// November 2026 (the engine's `docs/spec/design/pictures.md`), and a 64 MB
/// picture's decoding more; thirty seconds is far above both.
const PICTURE_DEADLINE: std::time::Duration = std::time::Duration::from_secs(30);

/// Gives the picture decoder `child` the choices and the picture, and sends
/// what it answers, or, once `deadline` has passed, kills it and says so.
fn convert_picture_with(
    mut child: Helper,
    picture: Arc<Vec<u8>>,
    choice: interface_core::picture::PictureChoice,
    generation: u64,
    events: Sender<Event>,
    deadline: std::time::Duration,
) -> Result<(), String> {
    let output = child.stdout.take().ok_or("the picture converter has no output")?;
    let mut input = child.stdin.take().ok_or("the picture converter has no input")?;
    let (answer, answered) = std::sync::mpsc::channel();
    thread::spawn(move || {
        let exchange = move || -> Result<super::picture::Converted, String> {
            write_to_decoder(&mut input, &ToDecoder::Picture(choice)).map_err(|error| error.to_string())?;
            for chunk in picture.chunks(MAX_CHUNK) {
                write_to_decoder(&mut input, &ToDecoder::Data(chunk.to_vec())).map_err(|error| error.to_string())?;
            }
            write_to_decoder(&mut input, &ToDecoder::End).map_err(|error| error.to_string())?;
            let mut output = BufReader::new(output);
            let (tti, t42, summary) = match read_from_decoder(&mut output).map_err(|error| error.to_string())? {
                Some(FromDecoder::PictureConverted { tti, t42, summary }) => (tti, t42, summary),
                Some(FromDecoder::PictureRefused(reason)) => return Err(reason),
                Some(_) => return Err(String::from("the picture converter said what it may not")),
                None => return Err(String::from("the picture converter ended without a page")),
            };
            // The picture in ReGIS, then in NAPLPS (renderer parity PAR-36
            // item 2), each in the language it was asked in or none.
            let [regis, naplps] = [PictureLanguage::Regis, PictureLanguage::Naplps].map(|language| match read_from_decoder(&mut output) {
                Ok(Some(FromDecoder::PictureOutput { language: said, output })) if said == language => output,
                Ok(_) => Err(String::from("the picture converter said what it may not")),
                Err(error) => Err(error.to_string()),
            });
            Ok(super::picture::Converted { tti, t42, summary, regis, naplps })
        };
        let _ = answer.send(exchange());
    });
    thread::spawn(move || {
        let converted = answered.recv_timeout(deadline).unwrap_or_else(|_| Err(String::from("the picture converter took too long")));
        end(&mut child);
        let _ = events.send(Event::PictureConverted(generation, converted));
    });
    Ok(())
}

/// Starts a fetcher process for the tabs of `key`, with a first place of
/// `generation` for a tab of `channel`; its tabs' messages arrive as
/// events, each as the event of the tab it names (REQ-6980).
/// A fetcher is given its proxy only here, when it starts: a change of the
/// proxy reaches fetchers started after it (requirement 19.12), and so
/// does a change of whether a channel's tabs share a connection
/// (REQ-6982).
pub(super) fn start_fetcher(broker: &Arc<Broker>, key: FetcherKey, channel: u32, generation: u64, events: Sender<Event>) -> Result<Fetcher, String> {
    let local_network = key.arguments.iter().any(|argument| argument == "--allow-local");
    // Its sandbox allows UDP to its channel's port alone, and only when
    // its tabs may share a connection (REQ-6981).
    let mut child = broker.spawn_fetcher(&key.arguments, local_network, key.udp_port).map_err(|error| format!("cannot start the fetcher: {error}"))?;
    eprintln!("neoteletext: the fetcher started confined: {}", child.report);
    let output = child.stdout.take().ok_or("the fetcher has no output")?;
    let input = child.stdin.take().map(Outbox::new);
    let routes = Routes::default();
    let fetcher = Fetcher::first(child, Some(key), channel, generation, input, Arc::clone(&routes));
    let heard = Arc::clone(&locked(&fetcher.process).heard);
    let listened = Arc::clone(&routes);
    relay(
        output,
        read_from_fetcher_for,
        events,
        move |(tab, message)| fetched_event(&listened, &heard, tab, message),
        move |end| Event::FetcherProcessEnded(routes, end),
    );
    Ok(fetcher)
}

/// A channel and a generation no place has, for a message that is not to
/// be listened to: channels are numbered from 1.
const NO_PLACE: (u32, u64) = (0, 0);

/// The event of a fetcher process's `message` naming `tab`: the event of
/// the tab's place when the trusted core added that tab and has not
/// removed it. The process's own word that it is confined is noted here,
/// for all its tabs. Anything else, a message of a tab that left or one
/// never added, becomes an event of no place, which nothing listens to
/// (REQ-6980).
fn fetched_event(routes: &Routes, heard: &AtomicBool, tab: u32, message: FromFetcher) -> Event {
    if tab == protocol::fetcher::PROCESS_TAB && message == FromFetcher::Confined {
        heard.store(true, Ordering::Release);
    }
    let (channel, generation) = locked(routes).get(&tab).copied().unwrap_or(NO_PLACE);
    Event::Fetched(channel, generation, message)
}

/// The places of the tabs a fetcher process that ended was serving: each
/// tab's channel and generation.
pub(super) fn places(routes: &Routes) -> Vec<(u32, u64)> {
    locked(routes).values().copied().collect()
}

/// The routes of a fetcher process that serves tabs at `places`, each a
/// channel and a generation, as a test says one ended.
#[cfg(test)]
pub(super) fn places_of(places: &[(u32, u64)]) -> FetcherRoutes {
    Arc::new(Mutex::new((1..).zip(places.iter().copied()).collect()))
}

/// As [`Routes`], for the event that says a fetcher process ended.
pub(super) type FetcherRoutes = Routes;

/// A snapshot drawn by a snapshot decoder (REQ-6885): the sub-code drawn,
/// its image, and its source with concealed text blanked, untrusted until
/// the trusted core checks the image as it checks a page's frame.
pub(super) struct SnapshotDrawn {
    pub(super) page: u16,
    pub(super) sub: u16,
    pub(super) image: protocol::decoder::SnapshotImage,
    pub(super) source: Vec<u8>,
}

/// Starts a fetcher in snapshot mode for `page`'s `sub` of the channel at
/// `address` (REQ-6884, `docs/tab-thumbnails.md` section 9): a fresh
/// process given the address, the place and a tab's transport rules, and
/// nothing of the viewer. Its answer's bytes, counted against
/// `MAX_SNAPSHOT_BYTES`, or why it has none, arrive as one event of `job`;
/// it is ended at once after, or at `SNAPSHOT_FETCH_LIMIT`.
pub(super) fn start_snapshot_fetch(
    broker: &Arc<Broker>,
    address: &str,
    page: u16,
    sub: u16,
    reach: &Reach,
    job: u64,
    events: Sender<Event>,
) -> Result<(), String> {
    let mut arguments = vec![address.to_owned(), String::from("--snapshot"), format!("{page:03X}"), format!("{sub:04X}")];
    for ip in &reach.allow_local {
        arguments.push(String::from("--allow-local"));
        arguments.push(ip.clone());
    }
    if let Some(proxy) = &reach.proxy {
        arguments.push(String::from("--proxy"));
        arguments.push(proxy.clone());
    }
    let child = spawn(broker, Program::Fetcher, &arguments, !reach.allow_local.is_empty())?;
    fetch_snapshot_with(child, job, events, neoteletext::snapshots::SNAPSHOT_FETCH_LIMIT);
    Ok(())
}

/// Reads a snapshot fetcher's answer: `Confined`, then pieces within the
/// bound, then `Answered`; anything else is a miss.
fn read_snapshot_answer(output: impl Read) -> Result<Vec<u8>, String> {
    use protocol::snapshot::{FromSnapshot, MAX_SNAPSHOT_BYTES, SnapshotEnd, read_from_fetcher};
    let mut output = BufReader::new(output);
    if read_from_fetcher(&mut output).map_err(|error| error.to_string())? != Some(FromSnapshot::Confined) {
        return Err(String::from("the snapshot fetcher did not say it was confined"));
    }
    let mut bytes = Vec::new();
    loop {
        match read_from_fetcher(&mut output).map_err(|error| error.to_string())? {
            Some(FromSnapshot::Piece(piece)) => {
                if bytes.len().saturating_add(piece.len()) > MAX_SNAPSHOT_BYTES {
                    return Err(String::from("the snapshot passed its bound"));
                }
                bytes.extend_from_slice(&piece);
            }
            Some(FromSnapshot::Ended(SnapshotEnd::Answered)) if !bytes.is_empty() => return Ok(bytes),
            Some(FromSnapshot::Ended(end)) => return Err(format!("no snapshot: {end:?}")),
            Some(FromSnapshot::Confined) | None => return Err(String::from("the snapshot fetcher ended without an answer")),
        }
    }
}

/// [`start_snapshot_fetch`]'s exchange with the fetcher `child`, ended at
/// `deadline`.
fn fetch_snapshot_with(mut child: Helper, job: u64, events: Sender<Event>, deadline: std::time::Duration) {
    let output = child.stdout.take();
    let (answer, answered) = std::sync::mpsc::channel();
    thread::spawn(move || {
        let read = output.map_or_else(|| Err(String::from("the snapshot fetcher has no output")), read_snapshot_answer);
        let _ = answer.send(read);
    });
    thread::spawn(move || {
        let read = answered.recv_timeout(deadline).unwrap_or_else(|_| Err(String::from("the snapshot fetcher took too long")));
        // Its input closed is Close; it is killed rather than waited for.
        drop(child.stdin.take());
        end(&mut child);
        let _ = events.send(Event::SnapshotFetched(job, read));
    });
}

/// Starts a decoder in snapshot mode to draw `page`'s `sub` from `bytes`
/// (REQ-6885): a fresh process given the bytes and the place, whose
/// answer, or why it drew nothing, arrives as one event of `job`; it is
/// ended after, or at `SNAPSHOT_DRAW_LIMIT`.
pub(super) fn start_snapshot_draw(broker: &Arc<Broker>, bytes: Arc<Vec<u8>>, page: u16, sub: u16, job: u64, events: Sender<Event>) -> Result<(), String> {
    let child = spawn(broker, Program::Decoder, &[String::from("--snapshot")], false)?;
    draw_snapshot_with(child, bytes, page, sub, job, events, neoteletext::snapshots::SNAPSHOT_DRAW_LIMIT);
    Ok(())
}

/// [`start_snapshot_draw`]'s exchange with the decoder `child`, ended at
/// `deadline`.
fn draw_snapshot_with(mut child: Helper, bytes: Arc<Vec<u8>>, page: u16, sub: u16, job: u64, events: Sender<Event>, deadline: std::time::Duration) {
    let output = child.stdout.take();
    let input = child.stdin.take();
    let (answer, answered) = std::sync::mpsc::channel();
    thread::spawn(move || {
        let exchange = move || -> Result<SnapshotDrawn, String> {
            let mut input = input.ok_or("the snapshot decoder has no input")?;
            let output = output.ok_or("the snapshot decoder has no output")?;
            for piece in bytes.chunks(protocol::decoder::MAX_WHOLE) {
                write_to_decoder(&mut input, &ToDecoder::Whole { packets: false, pushed: false, bytes: piece.to_vec() }).map_err(|error| error.to_string())?;
            }
            write_to_decoder(&mut input, &ToDecoder::Snapshot { page, sub }).map_err(|error| error.to_string())?;
            input.flush().map_err(|error| error.to_string())?;
            let mut output = BufReader::new(output);
            let mut source = Vec::new();
            loop {
                match read_from_decoder(&mut output).map_err(|error| error.to_string())? {
                    Some(FromDecoder::SnapshotSource(piece)) => {
                        if source.len().saturating_add(piece.len()) > protocol::snapshot::MAX_SNAPSHOT_BYTES {
                            return Err(String::from("the snapshot's source passed its bound"));
                        }
                        source.extend_from_slice(&piece);
                    }
                    Some(FromDecoder::SnapshotDrawn { page: drawn, sub, image }) if drawn == page && !source.is_empty() => {
                        return Ok(SnapshotDrawn { page, sub, image, source });
                    }
                    Some(FromDecoder::SnapshotRefused { reason, .. }) => return Err(format!("the snapshot was not drawn: {reason:?}")),
                    Some(_) => return Err(String::from("the snapshot decoder said what it may not")),
                    None => return Err(String::from("the snapshot decoder ended without an answer")),
                }
            }
        };
        let _ = answer.send(exchange());
    });
    thread::spawn(move || {
        let drawn = answered.recv_timeout(deadline).unwrap_or_else(|_| Err(String::from("the snapshot decoder took too long")));
        end(&mut child);
        let _ = events.send(Event::SnapshotDrawn(job, Box::new(drawn)));
    });
}

pub(super) fn read_interface(child: &mut Helper, events: Sender<Event>) -> Result<(), String> {
    let output = child.stdout.take().ok_or("the interface has no output")?;
    #[cfg(feature = "timed-test")]
    let read_from_interface = timed_read_from_interface;
    relay(output, read_from_interface, events, interface_said, Event::InterfaceEnded);
    Ok(())
}

/// What the interface said, as the core's thread is to take it: its
/// frames stamped with the moment they were read, so that the time the
/// event waits for the core's thread is not counted against the display's
/// phase (RULE-98 item 2; DEC-6753).
fn interface_said(message: protocol::ToCore) -> Event {
    match message {
        protocol::ToCore::FramePhase(phase) => Event::FramePhase(phase, std::time::Instant::now()),
        other => Event::Intent(other),
    }
}

/// The timed test's hops (page-rules.md RULE-66): when the reader of the
/// decoder's pipe read a repaint or a `LocalMidi`, before the core's
/// thread takes it.
#[cfg(feature = "timed-test")]
fn timed_read_from_decoder(input: &mut dyn Read) -> Result<Option<FromDecoder>, ProtocolError> {
    let message = read_from_decoder(input);
    match &message {
        Ok(Some(FromDecoder::Regis { .. })) => protocol::timed_test::mark("core", "regis-arrived", std::time::Instant::now(), format_args!("")),
        Ok(Some(FromDecoder::Audio(protocol::audio::Audio::LocalMidi { .. }))) => {
            protocol::timed_test::mark("core", "local-midi-arrived", std::time::Instant::now(), format_args!(""));
        }
        _ => {}
    }
    message
}

/// The timed test's hops: when the reader of the interface's pipe read an
/// input event.
#[cfg(feature = "timed-test")]
fn timed_read_from_interface(input: &mut dyn Read) -> Result<Option<protocol::ToCore>, ProtocolError> {
    let message = read_from_interface(input);
    #[cfg(feature = "interaction")]
    if let Ok(Some(protocol::ToCore::Input(_))) = &message {
        protocol::timed_test::mark("core", "input-arrived", std::time::Instant::now(), format_args!(""));
    }
    message
}

/// A reader that counts the bytes read through it: a message's length on
/// the pipe, what its budget is charged.
struct Counting<R> {
    inner: R,
    bytes: usize,
}

impl<R: Read> Read for Counting<R> {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        let read = self.inner.read(buffer)?;
        self.bytes = self.bytes.saturating_add(read);
        Ok(read)
    }
}

/// Reads a helper's messages on a thread of its own, each sent to the
/// trusted core's thread as the event `said` makes of it, until the pipe
/// ends or breaks the protocol, which is sent as the event `ended` makes
/// of why; it stops early once the trusted core has gone. Each message's
/// bytes are taken from the helper's own budget before it is sent, and
/// given back once the core has handled it, so a helper that writes
/// faster than the core handles stalls on its own pipe (finding F-10).
pub(super) fn relay<M: 'static, E: std::fmt::Display + 'static>(
    output: impl Read + Send + 'static,
    read: fn(&mut dyn Read) -> Result<Option<M>, E>,
    events: Sender<Event>,
    said: impl Fn(M) -> Event + Send + 'static,
    ended: impl FnOnce(Option<String>) -> Event + Send + 'static,
) {
    thread::spawn(move || {
        let budget = Budget::default();
        let mut output = Counting { inner: BufReader::new(output), bytes: 0 };
        let end = loop {
            let before = output.bytes;
            match read(&mut output) {
                Ok(Some(message)) => {
                    let ticket = budget.take(output.bytes.saturating_sub(before));
                    if events.send(Event::Budgeted(Box::new(said(message)), ticket)).is_err() {
                        return;
                    }
                }
                Ok(None) => break None,
                Err(error) => break Some(error.to_string()),
            }
        };
        let _ = events.send(ended(end));
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::mpsc;
    use std::time::Duration;

    /// The dots of a page image (`FromDecoder::Frame`).
    const FRAME_DOTS: usize = 480 * 500;
    /// How long the trusted core handles nothing in the flood test.
    const HOLD: Duration = Duration::from_secs(1);
    /// Where a flood is stopped when nothing holds it: four times the
    /// budget, far past what the bound lets through.
    const FLOOD_CAP: usize = 4 * neoteletext::outbox::MAX_QUEUED_BYTES;
    /// What a pipe and the reader's buffer hold beyond the budget: Linux's
    /// largest pipe without privileges (`/proc/sys/fs/pipe-max-size`, 1 MiB)
    /// is more than macOS's 64 KiB and the reader's 8 KiB together.
    const PIPE_AND_READER_BUFFERS: usize = 1024 * 1024;

    /// Finding F-85: a bookmark reader that never answers is ended at its
    /// deadline, and the import says it was not imported.
    #[cfg(unix)]
    #[test]
    fn a_bookmark_reader_that_never_answers_is_ended() {
        const DEADLINE: Duration = Duration::from_millis(200);
        let program = std::path::PathBuf::from("/bin/sleep");
        let arguments = [std::ffi::OsString::from("60")];
        let spawn = confine::Spawn {
            program: &program,
            arguments: &arguments,
            role: confine::Role::Decoder,
            environment: &[],
            local_network: false,
            udp_port: None,
            writable: None,
        };
        let child = Helper::local(confine::spawn(&spawn).unwrap());
        let (events, incoming) = mpsc::channel();
        read_bookmarks_with(child, b"<!DOCTYPE NETSCAPE-Bookmark-file-1>".to_vec(), events, DEADLINE).unwrap();
        let answer = incoming.recv_timeout(Duration::from_secs(5));
        assert!(matches!(&answer, Ok(Event::BookmarksRead(Err(reason))) if reason == "the bookmark reader took too long"), "no answer in five seconds");
    }

    /// Finding F-10: a helper that writes faster than the trusted core
    /// handles its messages is held at its budget, on its own pipe, so the
    /// core's queue of its messages does not grow without bound.
    #[test]
    fn a_helper_that_floods_the_core_is_held_at_its_budget() {
        let (reader, mut writer) = std::io::pipe().unwrap();
        let (events, incoming) = mpsc::channel();
        relay(reader, read_from_decoder, events, |message| Event::Decoded(1, 0, message), |end| Event::DecoderEnded(1, 0, end));
        let mut frame = Vec::new();
        protocol::decoder::write_frame_to_core(&mut frame, 0x100, &[[0, 0, 0]], 0, &vec![0; FRAME_DOTS]).unwrap();
        let frame_bytes = frame.len();
        let written = Arc::new(AtomicUsize::new(0));
        let theirs = Arc::clone(&written);
        let flood = thread::spawn(move || {
            while theirs.load(Ordering::Relaxed) < FLOOD_CAP && writer.write_all(&frame).is_ok() {
                theirs.fetch_add(frame.len(), Ordering::Relaxed);
            }
        });
        thread::sleep(HOLD);
        let held_at = written.load(Ordering::Relaxed);
        // The core takes its events, then goes: the helper's writes end.
        drop(incoming);
        flood.join().unwrap();
        let most = neoteletext::outbox::MAX_QUEUED_BYTES + PIPE_AND_READER_BUFFERS + frame_bytes;
        assert!(held_at <= most, "the helper wrote {held_at} bytes while the core handled nothing; at most {most}");
    }

    /// Requirement 19.12: a fetcher is started with the proxy and local
    /// addresses of the settings.
    #[test]
    fn a_fetcher_is_given_the_proxy_and_the_local_addresses() {
        let settings =
            Settings { proxy: stored::Proxy::from_parts("http", "p.example", 3128), local: vec!["10.0.0.2".parse().unwrap()], ..Settings::default() };
        let reach = Reach::of(&["192.168.0.9".to_owned()], &settings);
        assert_eq!(reach.allow_local, ["192.168.0.9", "10.0.0.2"]);
        assert_eq!(reach.proxy.as_deref(), Some("http://p.example:3128"));
        assert_eq!(Reach::of(&[], &Settings::default()).proxy, None);
    }

    /// What a test reads of what was written to a fetcher process's pipe.
    #[derive(Clone)]
    struct Kept(Arc<Mutex<Vec<u8>>>);

    impl Write for Kept {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            locked(&self.0).extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    /// Every message written to a fetcher process so far, with its tab.
    fn written(kept: &Kept) -> Vec<(u32, ToFetcher)> {
        // The outbox's own thread writes: what was flushed arrives soon.
        thread::sleep(Duration::from_millis(100));
        let bytes = locked(&kept.0).clone();
        let mut input = bytes.as_slice();
        let mut messages = Vec::new();
        while let Some(message) = protocol::fetcher::read_to_fetcher_for(&mut input).unwrap() {
            messages.push(message);
        }
        messages
    }

    /// REQ-6980: each tab's place in a fetcher process writes on the one
    /// pipe under its own tab number, which is never the process's own and
    /// never used twice; joining says "tab opens" and leaving "tab
    /// leaves"; and the process is ended with its last tab. Before, a
    /// fetcher's pipe carried one tab's messages, unnamed.
    #[cfg(unix)]
    #[test]
    fn each_place_in_a_fetcher_process_names_its_tab() {
        use super::super::test_support::{ended_soon, sleeping_helper};
        let (child, output) = sleeping_helper(confine::Role::Network);
        let kept = Kept(Arc::default());
        let reach = Reach { allow_local: Vec::new(), proxy: None, share_connections: true };
        let key = FetcherKey::of("https://a.invalid/teletext/", false, &reach);
        let mut first = Fetcher::first(child, Some(key.clone()), 5, 1, Some(Outbox::new(kept.clone())), Routes::default());
        assert!(first.serves(&key));
        assert!(!first.serves(&FetcherKey::of("https://a.invalid/teletext/", true, &reach)), "REQ-881: the private jar's tabs have a process of their own");
        let mut second = first.join(6, 2);
        assert_ne!(first.tab, second.tab);
        assert!(first.tab != protocol::fetcher::PROCESS_TAB && second.tab != protocol::fetcher::PROCESS_TAB);
        let (first_tab, second_tab) = (first.tab, second.tab);
        write_to_fetcher(second.input().unwrap(), &ToFetcher::FetchPage { page: 0x200 }).unwrap();
        write_to_fetcher(first.input().unwrap(), &ToFetcher::FetchContents).unwrap();
        second.input().unwrap().flush().unwrap();
        first.input().unwrap().flush().unwrap();
        assert_eq!(
            written(&kept),
            [
                (first_tab, ToFetcher::TabOpens),
                (second_tab, ToFetcher::TabOpens),
                (second_tab, ToFetcher::FetchPage { page: 0x200 }),
                (first_tab, ToFetcher::FetchContents)
            ]
        );
        // The routes are what the reader thread listens by.
        let routes = Arc::clone(&locked(&first.process).routes);
        assert_eq!(places(&routes), [(5, 1), (6, 2)]);

        // The first tab leaves, with a last message not yet flushed: it
        // goes before the leaving, and the process stays for the second.
        write_to_fetcher(first.input().unwrap(), &ToFetcher::FetchPage { page: 0x100 }).unwrap();
        first.stop();
        assert_eq!(written(&kept)[4..], [(first_tab, ToFetcher::FetchPage { page: 0x100 }), (first_tab, ToFetcher::TabLeaves)]);
        assert_eq!(places(&routes), [(6, 2)]);
        assert!(second.serves(&key), "the process still serves");
        let third = second.join(7, 3);
        assert!(third.tab != first_tab && third.tab != second_tab, "a number is never used twice");
        third.stop();
        second.stop();
        assert_eq!(places(&routes), []);
        assert!(ended_soon(output), "the last tab's leaving ends the process");
    }

    /// REQ-6980: a message from a fetcher process is listened to only for
    /// a tab the trusted core added and has not removed, as that tab's own
    /// place; a message of a tab that left, or of one never added, is of
    /// no place. The process's own "confined" is heard for all its tabs.
    #[test]
    fn a_message_is_listened_to_only_for_a_tab_held() {
        let routes = places_of(&[(5, 1), (6, 2)]);
        let heard = AtomicBool::new(false);
        let place = |tab: u32, message: FromFetcher| match fetched_event(&routes, &heard, tab, message) {
            Event::Fetched(channel, generation, _) => (channel, generation),
            _ => panic!("not a fetcher's message"),
        };
        assert_eq!(place(1, FromFetcher::Pushing(true)), (5, 1));
        assert_eq!(place(2, FromFetcher::Pushing(true)), (6, 2));
        assert_eq!(place(3, FromFetcher::Pushing(true)), NO_PLACE, "a tab never added");
        locked(&routes).remove(&1);
        assert_eq!(place(1, FromFetcher::Pushing(true)), NO_PLACE, "a tab that left");
        // No tab's own word makes the process heard: only the process's.
        assert_eq!(place(2, FromFetcher::Confined), (6, 2));
        assert!(!heard.load(Ordering::Acquire));
        assert_eq!(place(protocol::fetcher::PROCESS_TAB, FromFetcher::Confined), NO_PLACE);
        assert!(heard.load(Ordering::Acquire));
    }

    /// REQ-6982, REQ-6981, requirement 19.10: a fetcher is started with
    /// `--no-http3`, and its sandbox given no UDP port, exactly when
    /// sharing is off or a proxy is set; with sharing on, the port is the
    /// channel's own, 443 when its address names none, and only for a
    /// channel served over TLS.
    #[test]
    fn a_fetcher_may_share_exactly_when_the_setting_is_on_and_no_proxy_is_set() {
        let reach = |share_connections: bool, proxy: Option<&str>| Reach { allow_local: Vec::new(), proxy: proxy.map(str::to_owned), share_connections };
        let key = |address: &str, reach: &Reach| FetcherKey::of(address, false, reach);
        let shared = key("https://a.example/teletext/", &reach(true, None));
        assert_eq!((shared.arguments.as_slice(), shared.udp_port), (["https://a.example/teletext/".to_owned()].as_slice(), Some(443)));
        assert_eq!(key("https://a.example:8443/teletext/", &reach(true, None)).udp_port, Some(8443));
        let plain = key("http://a.example/teletext/", &reach(true, None));
        assert_eq!((plain.arguments.len(), plain.udp_port), (1, None), "QUIC is always TLS: a plain channel is given no UDP");
        let separate = key("https://a.example/teletext/", &reach(false, None));
        assert_eq!((separate.arguments.last().map(String::as_str), separate.udp_port), (Some("--no-http3"), None));
        let proxied = key("https://a.example/teletext/", &reach(true, Some("http://p.example:3128")));
        assert_eq!(proxied.arguments[1..], ["--proxy", "http://p.example:3128", "--no-http3"], "nothing goes direct");
        assert_eq!(proxied.udp_port, None);
        assert_ne!(shared, separate, "a tab opened after the setting changed joins no process started before it");
    }
}
