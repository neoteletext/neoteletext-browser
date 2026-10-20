//! The trusted core (design section 5): it keeps the tabs, their history
//! and their channels, reads each channel's data and hands it to that
//! channel's decoder at a fixed pace, forwards the pages that arrive,
//! checks every frame, encodes page images, and decides what the trusted
//! strip shows. It never parses teletext data and never draws; the
//! interface never reaches a decoder.
//!
//! Usage: `neoteletext [--allow-local <ip>]... [address]...`, each address
//! a web channel's folder (`http://host/folder/`), a `file://` address, or
//! an absolute path to a recording, a page file or a folder of page files.
//! A channel on this computer or the local network is fetched only from
//! the addresses given with `--allow-local` (the engine's REQ-SEC-006).
//! The decoder and the interface are found beside this program.

// On Windows, a desktop program: no console window opens with it. The
// processes talk through the pipes they are given, not a console.
#![cfg_attr(windows, windows_subsystem = "windows")]
// No panicking constructs: this crate reads input from outside the
// process. The same list is in each such crate; scripts/check.sh checks
// that they agree.
#![deny(
    clippy::indexing_slicing,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::arithmetic_side_effects,
    clippy::unreachable,
    clippy::todo,
    clippy::unimplemented
)]

mod session;
mod source;

use std::collections::BTreeMap;
use std::io::{BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, ExitCode, Stdio};
use std::sync::mpsc::{self, Sender};
use std::thread;
use std::time::Duration;

use interface_core::canvas::PageKey;
use protocol::decoder::{FIRST_STORE_PAGES, FromDecoder, MAX_CHUNK, MAX_STORE_PAGES, MAX_WHOLE, ToDecoder, read_from_decoder, write_to_decoder};
use protocol::fetcher::{FromFetcher, MAX_NOTICE, Origin, read_from_fetcher};
use protocol::{ToCore, ToInterface, clean, read_from_interface, write_to_interface};

use neoteletext::frames;
use neoteletext::hearing::{Hearing, Send};
#[cfg(feature = "links")]
use neoteletext::links::{Channel as LinkChannel, Follow, Links, Refusal};
use neoteletext::sleep::Sleep;
#[cfg(feature = "links")]
use protocol::links::{LinkKind, Message as LinkMessage, Pointing, Spot};

use crate::session::{HOME, Session, Went};
use crate::source::Next;

const DECODER: &str = "neoteletext-decoder";
const FETCHER: &str = "neoteletext-fetcher";
const INTERFACE: &str = "neoteletext-interface";
#[cfg(feature = "phone-keyboard")]
const KEYBOARD: &str = "neoteletext-keyboard";
/// A relay other than qrkeyboard's own, for testing against one run
/// locally.
#[cfg(feature = "phone-keyboard")]
const RELAY_VARIABLE: &str = "NEOTELETEXT_QRKEYBOARD_RELAY";

/// A channel's data is handed to its decoder in pieces of 64 packets of 42
/// bytes (2,688 bytes) every 0.2 seconds, the engine's
/// `DEFAULT_FETCH_INTERVAL_US` (`architecture.md` section 5.1): slow
/// enough to watch the pages of a recording arrive, about 13 kilobytes a
/// second.
const PACKET_BYTES: usize = 42;
const CHUNK_PACKETS: usize = 64;
const CHUNK_BYTES: usize = PACKET_BYTES * CHUNK_PACKETS;
const LOAD_INTERVAL: Duration = Duration::from_millis(200);
/// How often the trusted core looks for tabs that may sleep: a tab sleeps
/// at most this long after its time is up (decision 47).
const SLEEP_CHECK: Duration = Duration::from_secs(60);

/// The interface's and the decoders' sandbox on macOS: no network and no
/// writing to files. Proof A showed the system webview cannot run like
/// this; neither of these has a webview (design section 5.2).
#[cfg(target_os = "macos")]
const SANDBOX: &str = "(version 1)
(allow default)
(deny network*)
(deny file-write*)
(allow file-write-data (literal \"/dev/null\"))";

enum Event {
    Decoded(u32, u64, FromDecoder),
    Fetched(u32, u64, FromFetcher),
    FetcherEnded(u32, u64, Option<String>),
    DecoderEnded(u32, u64, Option<String>),
    Intent(ToCore),
    InterfaceEnded(Option<String>),
    Load,
    /// Time to see which tabs may sleep (decision 47).
    Sleep,
    /// The phone keyboard process said something, untrusted.
    #[cfg(feature = "phone-keyboard")]
    Phone(protocol::keyboard::FromKeyboard),
    /// The phone keyboard process ended.
    #[cfg(feature = "phone-keyboard")]
    PhoneEnded(Option<String>),
}

struct Decoder {
    child: Child,
    input: BufWriter<ChildStdin>,
    /// Which of the channel's decoders this is: one put to sleep may still
    /// report after the next has started, and is then not listened to.
    generation: u64,
}

struct Fetcher {
    child: Child,
    /// As [`Decoder::generation`].
    generation: u64,
    /// The only thing a fetcher is given: key records (requirement 18.4).
    #[cfg(feature = "interaction")]
    input: Option<BufWriter<ChildStdin>>,
}

fn sibling(name: &str) -> Result<PathBuf, String> {
    let program = std::env::current_exe().map_err(|error| format!("cannot find this program: {error}"))?;
    let path = program.parent().map(|directory| directory.join(program_file(name))).ok_or("this program has no directory")?;
    if path.is_file() { Ok(path) } else { Err(format!("{} is missing", path.display())) }
}

/// A program's file name on this platform: `.exe` on Windows.
fn program_file(name: &str) -> String {
    format!("{name}{}", std::env::consts::EXE_SUFFIX)
}

fn spawn(command: &mut Command, what: &str) -> Result<Child, String> {
    command.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::inherit()).spawn().map_err(|error| format!("cannot start the {what}: {error}"))
}

/// The fetcher's sandbox on macOS: the network, which is its job, and no
/// writing to files.
#[cfg(target_os = "macos")]
const FETCHER_SANDBOX: &str = "(version 1)
(allow default)
(deny file-write*)
(allow file-write-data (literal \"/dev/null\"))";

/// A command that runs `path` with the network but no file writes.
fn sandboxed_with_network(path: &Path) -> Command {
    #[cfg(target_os = "macos")]
    {
        let mut command = Command::new("/usr/bin/sandbox-exec");
        command.arg("-p").arg(FETCHER_SANDBOX).arg(path);
        command
    }
    #[cfg(not(target_os = "macos"))]
    Command::new(path)
}

/// A command that runs `path` in the sandbox, where there is one.
fn sandboxed(path: &Path) -> Command {
    #[cfg(target_os = "macos")]
    {
        let mut command = Command::new("/usr/bin/sandbox-exec");
        command.arg("-p").arg(SANDBOX).arg(path);
        command
    }
    #[cfg(not(target_os = "macos"))]
    Command::new(path)
}

/// Starts a decoder for `channel`, whose messages arrive as events.
fn start_decoder(path: &Path, channel: u32, generation: u64, store_pages: usize, events: Sender<Event>) -> Result<Decoder, String> {
    let mut command = sandboxed(path);
    command.arg("--store-pages").arg(store_pages.to_string());
    let mut child = spawn(&mut command, "decoder")?;
    let output = child.stdout.take().ok_or("the decoder has no output")?;
    let input = BufWriter::new(child.stdin.take().ok_or("the decoder has no input")?);
    thread::spawn(move || {
        let mut output = BufReader::new(output);
        let end = loop {
            match read_from_decoder(&mut output) {
                Ok(Some(message)) => {
                    if events.send(Event::Decoded(channel, generation, message)).is_err() {
                        return;
                    }
                }
                Ok(None) => break None,
                Err(error) => break Some(error.to_string()),
            }
        };
        let _ = events.send(Event::DecoderEnded(channel, generation, end));
    });
    Ok(Decoder { child, input, generation })
}

/// Starts a fetcher for the web channel at `address`, whose messages
/// arrive as events.
fn start_fetcher(path: &Path, address: &str, allow_local: &[String], channel: u32, generation: u64, events: Sender<Event>) -> Result<Fetcher, String> {
    let mut command = sandboxed_with_network(path);
    command.arg(address);
    for ip in allow_local {
        command.arg("--allow-local").arg(ip);
    }
    let mut child = spawn(&mut command, "fetcher")?;
    let output = child.stdout.take().ok_or("the fetcher has no output")?;
    thread::spawn(move || {
        let mut output = BufReader::new(output);
        let end = loop {
            match read_from_fetcher(&mut output) {
                Ok(Some(message)) => {
                    if events.send(Event::Fetched(channel, generation, message)).is_err() {
                        return;
                    }
                }
                Ok(None) => break None,
                Err(error) => break Some(error.to_string()),
            }
        };
        let _ = events.send(Event::FetcherEnded(channel, generation, end));
    });
    #[cfg(feature = "interaction")]
    let input = child.stdin.take().map(BufWriter::new);
    Ok(Fetcher {
        child,
        generation,
        #[cfg(feature = "interaction")]
        input,
    })
}

fn read_interface(child: &mut Child, events: Sender<Event>) -> Result<(), String> {
    let output = child.stdout.take().ok_or("the interface has no output")?;
    thread::spawn(move || {
        let mut input = BufReader::new(output);
        let end = loop {
            match read_from_interface(&mut input) {
                Ok(Some(intent)) => {
                    if events.send(Event::Intent(intent)).is_err() {
                        return;
                    }
                }
                Ok(None) => break None,
                Err(error) => break Some(error.to_string()),
            }
        };
        let _ = events.send(Event::InterfaceEnded(end));
    });
    Ok(())
}

fn every(period: Duration, events: Sender<Event>, event: fn() -> Event) {
    thread::spawn(move || {
        loop {
            thread::sleep(period);
            if events.send(event()).is_err() {
                return;
            }
        }
    });
}

struct Core {
    session: Session,
    decoders: BTreeMap<u32, Decoder>,
    decoder_path: PathBuf,
    fetchers: BTreeMap<u32, Fetcher>,
    fetcher_path: PathBuf,
    /// Local-network servers the user allowed (`--allow-local`).
    allow_local: Vec<String>,
    events: Sender<Event>,
    frames: frames::Frames,
    /// The channel whose decoder was last told which pages are shown.
    showing: Option<u32>,
    /// What of each channel's sound reached the interface.
    hearing: Hearing,
    /// Which tabs sleep, and what their decoders were given (decision 47).
    sleep: Sleep,
    /// The generation of the next decoder or fetcher started.
    generation: u64,
    /// The store size of each channel's decoder, when grown past the first.
    store_pages: BTreeMap<u32, usize>,
    /// The pages last told shown to the decoder of [`Core::showing`].
    shown_pages: Vec<u16>,
    /// The links of the pages on screen (`hyperlinks.md`).
    #[cfg(feature = "links")]
    links: Links,
    /// When the trusted core started: the clock of links and of typing.
    #[cfg(any(feature = "links", feature = "interaction"))]
    started: std::time::Instant,
    /// Typing into a page (`interaction.md`).
    #[cfg(feature = "interaction")]
    typing: neoteletext::typing::Typing,
    /// The page the viewer opened in the active tab.
    #[cfg(feature = "interaction")]
    opened: Option<PageKey>,
    /// The phone keyboard process, while one runs.
    #[cfg(feature = "phone-keyboard")]
    phone: Option<(Child, BufWriter<ChildStdin>)>,
    /// What the interface shows of it.
    #[cfg(feature = "phone-keyboard")]
    phone_view: protocol::keyboard::PhoneView,
    to_interface: BufWriter<ChildStdin>,
}

impl Core {
    /// Milliseconds since the trusted core started.
    #[cfg(any(feature = "links", feature = "interaction"))]
    fn now(&self) -> u64 {
        u64::try_from(self.started.elapsed().as_millis()).unwrap_or(u64::MAX)
    }

    /// The window gained or lost the keyboard focus: links hand off only
    /// with it, and the channel being typed into is told.
    #[cfg(any(feature = "links", feature = "interaction"))]
    #[cfg_attr(not(feature = "interaction"), allow(clippy::unnecessary_wraps, reason = "only typing can fail here"))]
    fn focused(&mut self, focused: bool) -> Result<(), String> {
        #[cfg(feature = "links")]
        self.links.focused(focused);
        #[cfg(feature = "interaction")]
        {
            let sends = self.typing.focused(focused, self.now());
            self.deliver_typing(sends)?;
        }
        Ok(())
    }

    /// The keyboard's intents: a key for the open page's channel, and the
    /// phone keyboard's panel.
    #[cfg(feature = "interaction")]
    fn input_intent(&mut self, intent: ToCore) -> Result<(), String> {
        match intent {
            ToCore::Key(intent) => {
                let sends = self.typing.key(&intent, self.now());
                self.deliver_typing(sends)?;
            }
            ToCore::Editing(editing) => {
                let sends = self.typing.editing(editing, self.now());
                self.deliver_typing(sends)?;
            }
            #[cfg(feature = "phone-keyboard")]
            ToCore::PhonePanel(open) => self.phone_panel(open)?,
            #[cfg(feature = "phone-keyboard")]
            ToCore::PhoneFinish => {
                if let Some((_, input)) = &mut self.phone {
                    let _ = protocol::keyboard::write_to_keyboard(input, &protocol::keyboard::ToKeyboard::Finish);
                }
            }
            _ => {}
        }
        Ok(())
    }

    /// Delivers what the typing rules decided: key records to a channel's
    /// fetcher, the typing state to the interface.
    #[cfg(feature = "interaction")]
    fn deliver_typing(&mut self, sends: Vec<neoteletext::typing::Send>) -> Result<(), String> {
        use protocol::fetcher::{ToFetcher, write_to_fetcher};
        for send in sends {
            match send {
                neoteletext::typing::Send::Fetcher(channel, record) => {
                    let Some(input) = self.fetchers.get_mut(&channel).and_then(|fetcher| fetcher.input.as_mut()) else { continue };
                    // A fetcher that cannot be written to has stopped; its
                    // reader reports that, and the socket's state follows.
                    let _ = write_to_fetcher(input, &ToFetcher::Send(record));
                }
                neoteletext::typing::Send::Interface(state) => self.send(&ToInterface::Typing(state))?,
            }
        }
        Ok(())
    }

    fn send(&mut self, message: &ToInterface) -> Result<(), String> {
        write_to_interface(&mut self.to_interface, message).map_err(|error| format!("cannot write to the interface: {error}"))
    }

    /// Delivers what the hearing rules decided.
    fn deliver(&mut self, sends: Vec<Send>) -> Result<(), String> {
        for send in sends {
            match send {
                Send::Decoder(channel, message) => self.tell(channel, &message),
                Send::Interface(message) => self.send(&message)?,
            }
        }
        Ok(())
    }

    fn tell(&mut self, channel: u32, message: &ToDecoder) {
        // Kept whether or not the channel is awake: a sleeping channel's
        // pushed pages are there when it wakes.
        self.sleep.keep(channel, message);
        let Some(decoder) = self.decoders.get_mut(&channel) else { return };
        // A decoder that cannot be written to has stopped; its reader
        // reports that, and its pages stay as they were.
        let _ = write_to_decoder(&mut decoder.input, message);
    }

    /// Starts a decoder for every awake channel that has none, giving it
    /// what the channel's last decoder was given; puts to sleep the
    /// decoders of channels that may sleep (decision 47); starts a fetcher
    /// for every open web channel; and closes the decoders and fetchers of
    /// channels no tab shows any more.
    fn match_decoders(&mut self) -> Result<(), String> {
        let now = std::time::Instant::now();
        let active = self.active_channel();
        if let Some(channel) = active {
            self.sleep.seen(channel, now);
        }
        let channels: Vec<(u32, Option<String>, Option<String>)> =
            self.session.tabs.iter().map(|tab| (tab.channel.id, tab.channel.problem.clone(), tab.channel.web.clone())).collect();
        for (id, problem, web) in &channels {
            let awake = self.sleep.awake(*id, active, now);
            if awake && !self.decoders.contains_key(id) {
                if let Some(problem) = problem {
                    eprintln!("neoteletext: {problem}");
                }
                self.generation = self.generation.wrapping_add(1);
                let pages = self.store_pages.get(id).copied().unwrap_or(FIRST_STORE_PAGES);
                let mut decoder = start_decoder(&self.decoder_path, *id, self.generation, pages, self.events.clone())?;
                for message in self.sleep.kept(*id) {
                    let _ = write_to_decoder(&mut decoder.input, message);
                }
                self.decoders.insert(*id, decoder);
            } else if !awake && self.decoders.contains_key(id) {
                self.put_to_sleep(*id)?;
            }
            // A web channel's pages come from its own fetcher, which runs
            // for as long as its tab is open, asleep or not: fetching never
            // depends on viewing (decision 22).
            if let Some(address) = web
                && !self.fetchers.contains_key(id)
            {
                self.generation = self.generation.wrapping_add(1);
                let fetcher = start_fetcher(&self.fetcher_path, address, &self.allow_local, *id, self.generation, self.events.clone())?;
                self.fetchers.insert(*id, fetcher);
            }
        }
        let unused: Vec<u32> = self.decoders.keys().chain(self.fetchers.keys()).copied().filter(|id| !channels.iter().any(|(used, ..)| used == id)).collect();
        for id in unused {
            self.put_to_sleep(id)?;
            self.stop_fetcher(id);
            self.sleep.forget(id);
        }
        Ok(())
    }

    /// Closes a channel's decoder, and forgets what it showed: its links,
    /// its typing and its sounds.
    fn put_to_sleep(&mut self, id: u32) -> Result<(), String> {
        #[cfg(feature = "links")]
        self.links.channel_gone(id);
        #[cfg(feature = "interaction")]
        self.typing.channel_gone(id);
        let sends = self.hearing.channel_gone(id);
        self.deliver(sends)?;
        if let Some(mut decoder) = self.decoders.remove(&id) {
            let _ = write_to_decoder(&mut decoder.input, &ToDecoder::Close);
            drop(decoder.input);
            let _ = decoder.child.wait();
        }
        Ok(())
    }

    /// A decoder's store is full: it is started again with twice the room,
    /// up to the most a store holds, and given what the channel's decoder
    /// was given and the pages shown (decision 47). A channel whose data
    /// was too much to keep keeps the decoder it has.
    fn grow_decoder(&mut self, id: u32) -> Result<(), String> {
        let pages = self.store_pages.get(&id).copied().unwrap_or(FIRST_STORE_PAGES);
        if pages >= MAX_STORE_PAGES || !self.sleep.has_everything(id) {
            return Ok(());
        }
        let pages = pages.saturating_mul(2).min(MAX_STORE_PAGES);
        self.store_pages.insert(id, pages);
        if let Some(mut decoder) = self.decoders.remove(&id) {
            let _ = write_to_decoder(&mut decoder.input, &ToDecoder::Close);
            drop(decoder.input);
            let _ = decoder.child.wait();
        }
        self.generation = self.generation.wrapping_add(1);
        let mut decoder = start_decoder(&self.decoder_path, id, self.generation, pages, self.events.clone())?;
        for message in self.sleep.kept(id) {
            let _ = write_to_decoder(&mut decoder.input, message);
        }
        if self.showing == Some(id) {
            let _ = write_to_decoder(&mut decoder.input, &ToDecoder::Showing(self.shown_pages.clone()));
        }
        self.decoders.insert(id, decoder);
        Ok(())
    }

    fn stop_fetcher(&mut self, id: u32) {
        if let Some(mut fetcher) = self.fetchers.remove(&id) {
            // Closing its input ends a fetcher.
            drop(fetcher.child.stdin.take());
            #[cfg(feature = "interaction")]
            drop(fetcher.input.take());
            let _ = fetcher.child.kill();
            let _ = fetcher.child.wait();
        }
    }

    fn fetched(&mut self, channel: u32, message: FromFetcher) -> Result<(), String> {
        match message {
            FromFetcher::Contents(entries) => {
                let Some(tab) = self.session.tabs.iter_mut().find(|tab| tab.channel.id == channel) else { return Ok(()) };
                tab.channel.set_contents(&entries);
                if self.active_channel() == Some(channel) {
                    let view = self.session.active_tab().map(|tab| tab.channel.view());
                    if let Some(view) = view {
                        self.send(&ToInterface::Channel(view))?;
                    }
                }
            }
            FromFetcher::Page { origin: Origin::Fetched, bytes } => {
                // Untouched, a piece at a time; each page file ends its last
                // line, so the next one starts cleanly.
                for piece in bytes.chunks(MAX_CHUNK - 1) {
                    self.tell(channel, &ToDecoder::Data(piece.to_vec()));
                }
                self.tell(channel, &ToDecoder::Data(b"\n".to_vec()));
            }
            // A file or a pushed message of a form the fetcher knows goes
            // whole and at once, so its pages show as it arrives
            // (`interaction.md` section 10).
            FromFetcher::Page { origin, bytes } => {
                if bytes.len() <= MAX_WHOLE {
                    let packets = matches!(origin, Origin::PushedPackets | Origin::FetchedPackets);
                    let pushed = matches!(origin, Origin::PushedPackets | Origin::PushedPage);
                    self.tell(channel, &ToDecoder::Whole { packets, pushed, bytes });
                }
            }
            FromFetcher::Notice(text) => eprintln!("neoteletext: channel {channel}: {}", clean(&text, MAX_NOTICE)),
            FromFetcher::Pushing(open) => {
                self.sleep.set_pushing(channel, open);
                // A live channel wakes at once, not at the next look.
                if open {
                    self.match_decoders()?;
                }
            }
            #[cfg(feature = "interaction")]
            FromFetcher::Input { open, secure } => {
                let sends = self.typing.input(channel, open, secure, self.now());
                self.deliver_typing(sends)?;
            }
            #[cfg(feature = "links")]
            FromFetcher::Links { aliases, services } => {
                self.links.descriptor(channel, &aliases, &services);
                if self.active_channel() == Some(channel) {
                    self.link(LinkMessage::Aliases { channel, numbers: self.links.aliases(channel) })?;
                }
                self.redecide_channel(channel)?;
            }
        }
        Ok(())
    }

    /// Sends everything the interface shows: tabs, strip and channel. The
    /// interface then reports what is visible afresh.
    fn send_all(&mut self) -> Result<(), String> {
        self.match_decoders()?;
        // Another tab or channel: the keys leave the page they were going
        // to; the interface reports what it opens next.
        #[cfg(feature = "interaction")]
        {
            self.opened = None;
            let sends = self.typing.opened(self.active_channel(), None, self.now());
            self.deliver_typing(sends)?;
        }
        self.frames = frames::Frames::default();
        if let Some(previous) = self.showing.take() {
            self.tell(previous, &ToDecoder::Showing(Vec::new()));
        }
        let tabs = self.session.tab_infos();
        let active = self.session.active;
        self.send(&ToInterface::Tabs { tabs, active })?;
        self.send(&ToInterface::Strip(self.session.strip()))?;
        if let Some(tab) = self.session.active_tab() {
            let channel = tab.channel.view();
            self.send(&ToInterface::Channel(channel))?;
        }
        #[cfg(feature = "links")]
        self.send_links_afresh()?;
        self.send_mutes()
    }

    /// Tells the interface each tab's channel's mute, where it changed: a
    /// muted tab's new channel starts muted.
    fn send_mutes(&mut self) -> Result<(), String> {
        let tabs: Vec<(u32, bool)> = self.session.tabs.iter().map(|tab| (tab.channel.id, tab.muted)).collect();
        let sends = tabs.into_iter().filter_map(|(channel, muted)| self.hearing.mute(channel, muted)).collect();
        self.deliver(sends)
    }

    /// The tabs again, after a change of mute, and the mutes themselves.
    fn send_tabs(&mut self) -> Result<(), String> {
        let tabs = self.session.tab_infos();
        let active = self.session.active;
        self.send(&ToInterface::Tabs { tabs, active })?;
        self.send_mutes()
    }

    fn active_channel(&self) -> Option<u32> {
        self.session.active_tab().map(|tab| tab.channel.id)
    }

    /// Returns false when the window should close.
    fn intent(&mut self, intent: ToCore) -> Result<bool, String> {
        match intent {
            ToCore::Visible(visible) => {
                let Some(channel) = self.active_channel() else { return Ok(true) };
                // Only the active channel's pages can be asked for.
                let visible: Vec<PageKey> = visible.into_iter().filter(|key| key.channel == channel).collect();
                let pages = visible.iter().map(|key| key.page).collect();
                #[cfg(feature = "links")]
                self.links.retain(&visible);
                self.frames.set_visible(visible);
                self.shown_pages.clone_from(&pages);
                self.tell(channel, &ToDecoder::Showing(pages));
                self.showing = Some(channel);
            }
            ToCore::Cell { key, version, column, row } => {
                // A click means only "this cell"; the trusted core decides
                // whether a link is there (requirement 12.3).
                #[cfg(feature = "links")]
                {
                    let (now, active) = (self.now(), self.active_channel());
                    let followed = self.links.click(Spot { key, version, column, row }, active, now);
                    self.follow(followed, "a click")?;
                }
                #[cfg(not(feature = "links"))]
                eprintln!("neoteletext: cell {column},{row} of image {version} of page {:03X} on channel {}", key.page, key.channel);
            }
            ToCore::SelectTab(index) => {
                if usize::from(index) < self.session.tabs.len() {
                    self.session.active = usize::from(index);
                    self.send_all()?;
                }
            }
            ToCore::CloseTab(index) => {
                self.session.close(usize::from(index));
                if self.session.tabs.is_empty() {
                    return Ok(false);
                }
                self.send_all()?;
            }
            ToCore::NewTab => {
                self.session.open(HOME)?;
                self.send_all()?;
            }
            ToCore::Opened(key) => {
                #[cfg(feature = "interaction")]
                {
                    self.opened = key;
                    let sends = self.typing.opened(self.active_channel(), key, self.now());
                    self.deliver_typing(sends)?;
                }
                #[cfg(feature = "links")]
                self.links.opened(key);
                let sends = self.hearing.open(self.active_channel(), key);
                self.deliver(sends)?;
            }
            ToCore::MuteTab(index) => {
                if self.session.toggle_mute(usize::from(index)) {
                    self.send_tabs()?;
                }
            }
            ToCore::MuteAll(muted) => {
                self.session.mute_all(muted);
                self.send_tabs()?;
            }
            ToCore::Sounding(channels) => self.sleep.set_sounding(&channels),
            ToCore::Navigate(typed) => {
                if let Err(reason) = self.session.navigate(&typed) {
                    eprintln!("neoteletext: {reason}");
                }
                self.send_all()?;
            }
            ToCore::Back | ToCore::Forward => match self.session.go(matches!(intent, ToCore::Forward)) {
                Went::Nowhere => {}
                Went::Channel => self.send_all()?,
                // Another page of the same channel: it stays open.
                #[cfg_attr(not(feature = "links"), allow(unused_variables, reason = "only a link opens a page"))]
                Went::Page(page) => {
                    self.send(&ToInterface::Strip(self.session.strip()))?;
                    #[cfg(feature = "links")]
                    if let (Some(page), Some(channel)) = (page, self.active_channel()) {
                        self.link(LinkMessage::Show(PageKey { channel, page }))?;
                    }
                }
            },
            ToCore::Bookmark => {
                self.session.toggle_bookmark();
                self.send(&ToInterface::Strip(self.session.strip()))?;
            }
            #[cfg(feature = "links")]
            ToCore::Point(_) | ToCore::Typed { .. } | ToCore::Confirm(_) | ToCore::Cancel(_) => self.link_intent(&intent)?,
            #[cfg(any(feature = "links", feature = "interaction"))]
            ToCore::Focused(focused) => self.focused(focused)?,
            #[cfg(feature = "interaction")]
            ToCore::Key(_) | ToCore::Editing(_) => self.input_intent(intent)?,
            #[cfg(feature = "phone-keyboard")]
            ToCore::PhonePanel(_) | ToCore::PhoneFinish => self.input_intent(intent)?,
        }
        Ok(true)
    }

    /// Hands every channel the next piece of its data, whether or not it
    /// is on screen (decision 22).
    fn load(&mut self) {
        let mut messages = Vec::new();
        for tab in &mut self.session.tabs {
            // A sleeping channel's file waits where it is: what was read is
            // kept, and the rest is read when it wakes.
            if !self.decoders.contains_key(&tab.channel.id) {
                continue;
            }
            let message = match tab.channel.source.next(CHUNK_BYTES) {
                Next::Data(bytes) => ToDecoder::Data(bytes.to_vec()),
                Next::End => ToDecoder::End,
                Next::Done => continue,
            };
            messages.push((tab.channel.id, message));
        }
        for (channel, message) in messages {
            self.tell(channel, &message);
        }
    }

    /// A decoder that broke the protocol is closed; its channel's pages stay
    /// as they were, and its sounds go.
    fn decoder_ended(&mut self, channel: u32, generation: u64, reason: Option<String>) -> Result<(), String> {
        // A decoder put to sleep, or one before it, has nothing more to say.
        if self.decoders.get(&channel).map(|decoder| decoder.generation) != Some(generation) {
            return Ok(());
        }
        if let Some(reason) = reason {
            eprintln!("neoteletext: the decoder for channel {channel} stopped: {reason}");
        }
        if let Some(mut decoder) = self.decoders.remove(&channel) {
            let _ = decoder.child.kill();
            let _ = decoder.child.wait();
        }
        let sends = self.hearing.channel_gone(channel);
        self.deliver(sends)
    }

    fn decoded(&mut self, channel: u32, message: FromDecoder) -> Result<(), String> {
        let active = self.active_channel();
        match message {
            FromDecoder::StoreFull => self.grow_decoder(channel)?,
            FromDecoder::Arrived { page, title } => {
                let Some(tab) = self.session.tabs.iter_mut().find(|tab| tab.channel.id == channel) else { return Ok(()) };
                if let Some(title) = tab.channel.arrived(page, &title)
                    && active == Some(channel)
                {
                    self.send(&ToInterface::Page { channel, page, title })?;
                    // A number on a page shown becomes a link once its page exists.
                    #[cfg(feature = "links")]
                    self.redecide_channel(channel)?;
                }
            }
            FromDecoder::Updated { page } => {
                if active == Some(channel) {
                    self.send(&ToInterface::Updated { channel, page })?;
                }
            }
            FromDecoder::Frame { page, palette, indices } => {
                let key = PageKey { channel, page };
                match self.frames.accept(key, &palette, &indices) {
                    Some((version, png)) => {
                        // The image's links first, so the interface has them
                        // when the image arrives (`hyperlinks.md` section 8.1).
                        #[cfg(feature = "links")]
                        self.links_of_image(key, version)?;
                        self.send(&ToInterface::Frame { key, version, png })?;
                    }
                    // The same image again: only its links may have changed.
                    #[cfg(feature = "links")]
                    None => self.redecide(key)?,
                    #[cfg(not(feature = "links"))]
                    None => {}
                }
            }
            #[cfg(feature = "links")]
            FromDecoder::Links { page, with_image, candidates, numbers } => {
                let key = PageKey { channel, page };
                self.links.found(key, candidates, numbers);
                if !with_image {
                    self.redecide(key)?;
                }
            }
            #[cfg(feature = "interaction")]
            FromDecoder::Cursor { body } => {
                let sends = self.typing.cursor(channel, body, self.now());
                self.deliver_typing(sends)?;
            }
            FromDecoder::Audio(audio) => {
                if let Some(message) = self.hearing.decoded(channel, audio) {
                    self.send(&message)?;
                }
            }
        }
        Ok(())
    }
}

/// The phone keyboard (`interaction.md` section 9, requirement 18.9): a
/// process of its own holds the pairing secret; the trusted core shows its
/// QR in the panel and gives the interface the phone's keys, which act as
/// the interface's own.
#[cfg(feature = "phone-keyboard")]
impl Core {
    fn send_phone(&mut self) -> Result<(), String> {
        let view = self.phone_view.clone();
        self.send(&ToInterface::Phone(view))
    }

    /// The panel opened or closed. Opening starts the keyboard process if
    /// none runs; closing it before a phone is paired ends it.
    fn phone_panel(&mut self, open: bool) -> Result<(), String> {
        use protocol::keyboard::PhoneState;
        if !open {
            if self.phone_view.state != PhoneState::Connected
                && let Some((mut child, input)) = self.phone.take()
            {
                // Its input closing ends it, and the relay hears it leave.
                drop(input);
                let _ = child.wait();
                self.phone_view = protocol::keyboard::PhoneView { state: PhoneState::Finished, qr: None, pin: None };
                self.send_phone()?;
            }
            return Ok(());
        }
        if self.phone.is_some() {
            return self.send_phone();
        }
        let mut command = sandboxed_with_network(&sibling(KEYBOARD)?);
        // Without one, the keyboard process uses qrkeyboard's own relay.
        if let Ok(relay) = std::env::var(RELAY_VARIABLE) {
            command.arg(relay);
        }
        let mut child = spawn(&mut command, "phone keyboard")?;
        let output = child.stdout.take().ok_or("the phone keyboard has no output")?;
        let input = BufWriter::new(child.stdin.take().ok_or("the phone keyboard has no input")?);
        let events = self.events.clone();
        thread::spawn(move || {
            let mut output = BufReader::new(output);
            let end = loop {
                match protocol::keyboard::read_from_keyboard(&mut output) {
                    Ok(Some(message)) => {
                        if events.send(Event::Phone(message)).is_err() {
                            return;
                        }
                    }
                    Ok(None) => break None,
                    Err(error) => break Some(error.to_string()),
                }
            };
            let _ = events.send(Event::PhoneEnded(end));
        });
        self.phone = Some((child, input));
        self.phone_view = protocol::keyboard::PhoneView { state: PhoneState::Preparing, qr: None, pin: None };
        self.send_phone()
    }

    /// What the keyboard process said: its QR and state for the panel, and
    /// the phone's keys, only while it is paired.
    fn phone_said(&mut self, message: protocol::keyboard::FromKeyboard) -> Result<(), String> {
        use protocol::keyboard::{FromKeyboard, PhoneKey, PhoneState};
        match message {
            FromKeyboard::Join { qr, pin } => {
                self.phone_view.qr = Some(qr);
                self.phone_view.pin = pin;
                self.send_phone()
            }
            FromKeyboard::State(state) => {
                self.phone_view.state = state;
                if matches!(state, PhoneState::Finished | PhoneState::Connected) {
                    // Once paired, or over, the QR is not shown again.
                    self.phone_view.qr = None;
                    self.phone_view.pin = None;
                }
                self.send_phone()
            }
            FromKeyboard::Key { down, name } if self.phone_view.state == PhoneState::Connected => {
                self.send(&ToInterface::PhoneKey(PhoneKey::Key { down, name }))
            }
            FromKeyboard::Text(text) if self.phone_view.state == PhoneState::Connected => {
                self.send(&ToInterface::PhoneKey(PhoneKey::Text(clean(&text, protocol::keyboard::MAX_PHONE_TEXT))))
            }
            FromKeyboard::Key { .. } | FromKeyboard::Text(_) => Ok(()),
        }
    }

    fn phone_ended(&mut self, reason: Option<String>) -> Result<(), String> {
        if let Some(reason) = reason {
            eprintln!("neoteletext: the phone keyboard stopped: {reason}");
        }
        if let Some((mut child, _)) = self.phone.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        self.phone_view = protocol::keyboard::PhoneView { state: protocol::keyboard::PhoneState::Finished, qr: None, pin: None };
        self.send_phone()
    }
}

/// Hyperlinks (`hyperlinks.md`): the trusted core decides every link and
/// follows it by its own rules.
#[cfg(feature = "links")]
impl Core {
    fn link(&mut self, message: LinkMessage) -> Result<(), String> {
        self.send(&ToInterface::Link(message))
    }

    /// Runs `decide` with the link table and channel `id` as links see it.
    fn with_channel<T>(&mut self, id: u32, decide: impl FnOnce(&mut Links, &LinkChannel<'_>) -> T) -> Option<T> {
        let Self { session, links, .. } = self;
        let tab = session.tabs.iter().find(|tab| tab.channel.id == id)?;
        let address = tab.address()?;
        let exists = |page: u16| tab.channel.has_page(page);
        Some(decide(links, &LinkChannel { id, address, exists: &exists }))
    }

    /// A new image of a page was sent: its links, for the interface.
    fn links_of_image(&mut self, key: PageKey, version: u32) -> Result<(), String> {
        let now = self.now();
        if let Some(links) = self.with_channel(key.channel, |table, channel| table.image(key, version, channel, now)) {
            self.link(LinkMessage::Page { key, links })?;
        }
        self.repoint()
    }

    /// Decides a page's links again, and sends them when they changed.
    fn redecide(&mut self, key: PageKey) -> Result<(), String> {
        let now = self.now();
        if let Some(Some(links)) = self.with_channel(key.channel, |table, channel| table.redecide(key, channel, now)) {
            self.link(LinkMessage::Page { key, links })?;
        }
        self.repoint()
    }

    /// Decides again the links of every page on screen of `channel`.
    fn redecide_channel(&mut self, channel: u32) -> Result<(), String> {
        for key in self.links.pages_of(channel) {
            self.redecide(key)?;
        }
        Ok(())
    }

    /// The strip's destination follows a change of links.
    fn repoint(&mut self) -> Result<(), String> {
        match self.links.repoint() {
            Some(pointing) => self.link(LinkMessage::Pointing(pointing)),
            None => Ok(()),
        }
    }

    /// After a change of tab or channel: no page's links, no question in
    /// the strip, the channel's aliases, and the page its address names.
    fn send_links_afresh(&mut self) -> Result<(), String> {
        self.links.clear_pages();
        self.links.forget_prompt();
        self.link(LinkMessage::Prompt(None))?;
        self.link(LinkMessage::Pointing(None))?;
        let Some(channel) = self.active_channel() else { return Ok(()) };
        self.link(LinkMessage::Aliases { channel, numbers: self.links.aliases(channel) })?;
        let page = self.session.active_tab().and_then(|tab| tab.address()).and_then(|address| address.page);
        match page {
            Some(page) => self.link(LinkMessage::Show(PageKey { channel, page })),
            None => Ok(()),
        }
    }

    fn link_intent(&mut self, intent: &ToCore) -> Result<(), String> {
        let now = self.now();
        match intent {
            ToCore::Point(spot) => {
                let pointing = self.links.point(*spot);
                self.link(LinkMessage::Pointing(pointing))?;
            }
            ToCore::Typed { number, opened } => {
                let followed = self.links.typed(*number, *opened, self.active_channel(), now);
                self.follow(followed, "a typed page number")?;
            }
            ToCore::Confirm(id) => {
                let followed = self.links.confirm(*id, now);
                self.follow(followed, "a confirmation")?;
            }
            // Declining has no effect but to close the question (requirement 8.6).
            ToCore::Cancel(id) if self.links.cancel(*id) => self.link(LinkMessage::Prompt(None))?,
            _ => {}
        }
        Ok(())
    }

    /// Carries out what a click, a typed number or a confirmation led to.
    fn follow(&mut self, followed: Result<Follow, Refusal>, what: &str) -> Result<(), String> {
        match followed {
            Err(Refusal::NotALink) => Ok(()),
            Err(refusal) => {
                eprintln!("neoteletext: {what} was not followed: {refusal:?}");
                Ok(())
            }
            Ok(Follow::Page(page)) => {
                let Some(channel) = self.active_channel() else { return Ok(()) };
                if self.session.show_page(page).is_some() {
                    self.send(&ToInterface::Strip(self.session.strip()))?;
                }
                self.link(LinkMessage::Show(PageKey { channel, page }))
            }
            Ok(Follow::Ask(prompt)) => self.link(LinkMessage::Prompt(Some(prompt))),
            Ok(Follow::Leave(address)) => {
                self.session.follow(address)?;
                self.send_all()
            }
            Ok(Follow::HandOff(address)) => {
                self.link(LinkMessage::Prompt(None))?;
                self.hand_off(&address)
            }
        }
    }

    /// Hands a web page to the system's web browser, logged with the page
    /// it came from and the time (requirement 17.9).
    fn hand_off(&mut self, address: &str) -> Result<(), String> {
        let from = self.links.opened_page().map(|key| format!("page {:03X} of channel {}", key.page, key.channel)).unwrap_or_default();
        let at = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |since| since.as_secs());
        match web_opener::open(address) {
            Ok(()) => {
                eprintln!("neoteletext: handed {address} to the web browser, from {from}, at {at} seconds past 1970");
                Ok(())
            }
            Err(error) => {
                eprintln!("neoteletext: {address}: {error}");
                self.link(LinkMessage::Pointing(Some(Pointing { kind: LinkKind::Web, text: "Could not open your web browser".into() })))
            }
        }
    }
}

impl Core {
    fn new(session: Session, allow_local: Vec<String>, events: Sender<Event>, to_interface: BufWriter<ChildStdin>) -> Result<Self, String> {
        Ok(Self {
            session,
            decoders: BTreeMap::new(),
            decoder_path: sibling(DECODER)?,
            fetchers: BTreeMap::new(),
            fetcher_path: sibling(FETCHER)?,
            allow_local,
            events,
            frames: frames::Frames::default(),
            showing: None,
            hearing: Hearing::default(),
            sleep: Sleep::default(),
            generation: 0,
            store_pages: BTreeMap::new(),
            shown_pages: Vec::new(),
            #[cfg(feature = "links")]
            links: Links::default(),
            #[cfg(any(feature = "links", feature = "interaction"))]
            started: std::time::Instant::now(),
            #[cfg(feature = "interaction")]
            typing: neoteletext::typing::Typing::default(),
            #[cfg(feature = "interaction")]
            opened: None,
            #[cfg(feature = "phone-keyboard")]
            phone: None,
            #[cfg(feature = "phone-keyboard")]
            phone_view: protocol::keyboard::PhoneView { state: protocol::keyboard::PhoneState::Finished, qr: None, pin: None },
            to_interface,
        })
    }
}

fn run() -> Result<(), String> {
    let mut addresses = Vec::new();
    let mut allow_local = Vec::new();
    let mut arguments = std::env::args().skip(1);
    while let Some(argument) = arguments.next() {
        if argument == "--allow-local" {
            allow_local.push(arguments.next().ok_or("--allow-local needs an address")?);
        } else {
            addresses.push(argument);
        }
    }
    let mut interface = spawn(&mut sandboxed(&sibling(INTERFACE)?), "interface")?;
    let (events, incoming) = mpsc::channel();
    read_interface(&mut interface, events.clone())?;
    every(LOAD_INTERVAL, events.clone(), || Event::Load);
    every(SLEEP_CHECK, events.clone(), || Event::Sleep);

    let mut session = Session::new();
    for address in &addresses {
        if let Err(reason) = session.open(address) {
            eprintln!("neoteletext: {address}: {reason}");
        }
    }
    if session.tabs.is_empty() {
        session.open(HOME)?;
    }
    session.active = 0;
    let to_interface = BufWriter::new(interface.stdin.take().ok_or("the interface has no input")?);
    let mut core = Core::new(session, allow_local, events, to_interface)?;
    core.send_all()?;

    let result = loop {
        let Ok(event) = incoming.recv() else { break Ok(()) };
        let step = match event {
            Event::Decoded(channel, generation, message) => {
                if core.decoders.get(&channel).map(|decoder| decoder.generation) == Some(generation) {
                    core.decoded(channel, message)
                } else {
                    Ok(())
                }
            }
            Event::Fetched(channel, generation, message) => {
                if core.fetchers.get(&channel).map(|fetcher| fetcher.generation) == Some(generation) {
                    core.fetched(channel, message)
                } else {
                    Ok(())
                }
            }
            Event::FetcherEnded(channel, generation, reason) => {
                if core.fetchers.get(&channel).map(|fetcher| fetcher.generation) == Some(generation) {
                    eprintln!("neoteletext: the fetcher for channel {channel} stopped{}", reason.map(|reason| format!(": {reason}")).unwrap_or_default());
                    if let Some(mut fetcher) = core.fetchers.remove(&channel) {
                        let _ = fetcher.child.wait();
                    }
                }
                Ok(())
            }
            Event::Intent(intent) => match core.intent(intent) {
                Ok(true) => Ok(()),
                Ok(false) => break Ok(()),
                Err(error) => Err(error),
            },
            Event::Load => {
                core.load();
                Ok(())
            }
            Event::DecoderEnded(channel, generation, reason) => core.decoder_ended(channel, generation, reason),
            Event::Sleep => core.match_decoders(),
            #[cfg(feature = "phone-keyboard")]
            Event::Phone(message) => core.phone_said(message),
            #[cfg(feature = "phone-keyboard")]
            Event::PhoneEnded(reason) => core.phone_ended(reason),
            Event::InterfaceEnded(None) => break Ok(()),
            Event::InterfaceEnded(Some(reason)) => break Err(format!("the interface broke the protocol and was closed: {reason}")),
        };
        if let Err(error) = step {
            break Err(error);
        }
    };

    for (_, mut fetcher) in std::mem::take(&mut core.fetchers) {
        drop(fetcher.child.stdin.take());
        #[cfg(feature = "interaction")]
        drop(fetcher.input.take());
        let _ = fetcher.child.kill();
        let _ = fetcher.child.wait();
    }
    for (_, mut decoder) in std::mem::take(&mut core.decoders) {
        let _ = write_to_decoder(&mut decoder.input, &ToDecoder::Close);
        let _ = decoder.input.flush();
        drop(decoder.input);
        let _ = decoder.child.wait();
    }
    let _ = write_to_interface(&mut core.to_interface, &ToInterface::Close);
    drop(core);
    if result.is_err() {
        let _ = interface.kill();
    }
    let status = interface.wait().map_err(|error| format!("cannot wait for the interface: {error}"))?;
    ended(result, status)
}

/// The browser's outcome once the interface has ended: an interface that
/// crashed or failed is a failure even when it closed its pipe cleanly, as
/// a crashing process does.
fn ended(result: Result<(), String>, interface: std::process::ExitStatus) -> Result<(), String> {
    result?;
    if interface.success() { Ok(()) } else { Err(format!("the interface ended abnormally ({interface})")) }
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("neoteletext: {error}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod program_tests {
    use super::*;

    #[test]
    fn a_helper_program_has_this_platforms_file_name() {
        let expected = if cfg!(windows) { "neoteletext-interface.exe" } else { "neoteletext-interface" };
        assert_eq!(program_file(INTERFACE), expected);
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::process::ExitStatusExt;

    /// A wait status as the kernel reports it: a signal number in the low
    /// bits, or an exit code shifted into the second byte.
    const SEGMENTATION_FAULT: i32 = 11;
    const EXIT_CODE_SHIFT: u32 = 8;

    #[test]
    fn a_crashed_or_failed_interface_fails_the_browser() {
        let clean = std::process::ExitStatus::from_raw(0);
        assert_eq!(ended(Ok(()), clean), Ok(()));
        let crashed = std::process::ExitStatus::from_raw(SEGMENTATION_FAULT);
        assert!(ended(Ok(()), crashed).is_err_and(|error| error.contains("abnormally")), "a crash is not a clean exit");
        let failed = std::process::ExitStatus::from_raw(1 << EXIT_CODE_SHIFT);
        assert!(ended(Ok(()), failed).is_err());
        assert_eq!(ended(Err("first".into()), crashed), Err("first".into()), "the first error is the one reported");
    }
}
