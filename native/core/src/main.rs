//! The trusted core (design section 5): it keeps the tabs, their history
//! and their channels, reads each channel's data and hands it to that
//! channel's decoder at a fixed pace, forwards the pages that arrive,
//! checks every frame, encodes page images, and decides what the trusted
//! strip shows. It never parses teletext data and never draws; the
//! interface never reaches a decoder.
//!
//! Usage: `t42-browser [--allow-local <ip>]... [address]...`, each address
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

mod frames;
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
use neoteletext_core::consts::{FLASH_CYCLE_US, FLASH_UNITS};
use protocol::decoder::{FromDecoder, MAX_CHUNK, ToDecoder, read_from_decoder, write_to_decoder};
use protocol::fetcher::{FromFetcher, MAX_NOTICE, read_from_fetcher};
use protocol::{ToCore, ToInterface, clean, read_from_interface, write_to_interface};

#[cfg(feature = "links")]
use protocol::links::{LinkKind, Message as LinkMessage, Pointing, Spot};
use t42_browser::hearing::{Hearing, Send};
#[cfg(feature = "links")]
use t42_browser::links::{Channel as LinkChannel, Follow, Links, Refusal};

use crate::session::{HOME, Session, Went};
use crate::source::Next;

const DECODER: &str = "t42-decoder";
const FETCHER: &str = "t42-fetcher";
const INTERFACE: &str = "t42-interface";

/// A channel's data is handed to its decoder in pieces of 64 packets of 42
/// bytes (2,688 bytes) every 0.2 seconds, the engine's
/// `DEFAULT_FETCH_INTERVAL_US` (`architecture.md` section 5.1): slow
/// enough to watch the pages of a recording arrive, about 13 kilobytes a
/// second.
const PACKET_BYTES: usize = 42;
const CHUNK_PACKETS: usize = 64;
const CHUNK_BYTES: usize = PACKET_BYTES * CHUNK_PACKETS;
const LOAD_INTERVAL: Duration = Duration::from_millis(200);
/// The shown pages are drawn again once per flash unit: the engine's flash
/// cycle of one second in twelve units (its decision E.48), so every flash
/// rate it renders is shown.
const FLASH_UNIT: Duration = Duration::from_micros(FLASH_CYCLE_US / FLASH_UNITS as u64);

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
    Decoded(u32, FromDecoder),
    Fetched(u32, FromFetcher),
    FetcherEnded(u32, Option<String>),
    DecoderEnded(u32, Option<String>),
    Intent(ToCore),
    InterfaceEnded(Option<String>),
    Flash,
    Load,
}

struct Decoder {
    child: Child,
    input: BufWriter<ChildStdin>,
}

struct Fetcher {
    child: Child,
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
fn start_decoder(path: &Path, channel: u32, events: Sender<Event>) -> Result<Decoder, String> {
    let mut child = spawn(&mut sandboxed(path), "decoder")?;
    let output = child.stdout.take().ok_or("the decoder has no output")?;
    let input = BufWriter::new(child.stdin.take().ok_or("the decoder has no input")?);
    thread::spawn(move || {
        let mut output = BufReader::new(output);
        let end = loop {
            match read_from_decoder(&mut output) {
                Ok(Some(message)) => {
                    if events.send(Event::Decoded(channel, message)).is_err() {
                        return;
                    }
                }
                Ok(None) => break None,
                Err(error) => break Some(error.to_string()),
            }
        };
        let _ = events.send(Event::DecoderEnded(channel, end));
    });
    Ok(Decoder { child, input })
}

/// Starts a fetcher for the web channel at `address`, whose messages
/// arrive as events.
fn start_fetcher(path: &Path, address: &str, allow_local: &[String], channel: u32, events: Sender<Event>) -> Result<Fetcher, String> {
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
                    if events.send(Event::Fetched(channel, message)).is_err() {
                        return;
                    }
                }
                Ok(None) => break None,
                Err(error) => break Some(error.to_string()),
            }
        };
        let _ = events.send(Event::FetcherEnded(channel, end));
    });
    Ok(Fetcher { child })
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
    /// The links of the pages on screen (`hyperlinks.md`).
    #[cfg(feature = "links")]
    links: Links,
    /// When the trusted core started: the links' clock.
    #[cfg(feature = "links")]
    started: std::time::Instant,
    to_interface: BufWriter<ChildStdin>,
}

impl Core {
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
        let Some(decoder) = self.decoders.get_mut(&channel) else { return };
        // A decoder that cannot be written to has stopped; its reader
        // reports that, and its pages stay as they were.
        let _ = write_to_decoder(&mut decoder.input, message);
    }

    /// Starts a decoder for every channel that has none, and closes the
    /// decoders of channels no tab shows any more.
    fn match_decoders(&mut self) -> Result<(), String> {
        let channels: Vec<(u32, Option<String>, Option<String>)> =
            self.session.tabs.iter().map(|tab| (tab.channel.id, tab.channel.problem.clone(), tab.channel.web.clone())).collect();
        for (id, problem, web) in &channels {
            if !self.decoders.contains_key(id) {
                if let Some(problem) = problem {
                    eprintln!("t42-browser: {problem}");
                }
                let decoder = start_decoder(&self.decoder_path, *id, self.events.clone())?;
                self.decoders.insert(*id, decoder);
                // A web channel's pages come from its own fetcher.
                if let Some(address) = web {
                    let fetcher = start_fetcher(&self.fetcher_path, address, &self.allow_local, *id, self.events.clone())?;
                    self.fetchers.insert(*id, fetcher);
                }
            }
        }
        let unused: Vec<u32> = self.decoders.keys().copied().filter(|id| !channels.iter().any(|(used, ..)| used == id)).collect();
        for id in unused {
            #[cfg(feature = "links")]
            self.links.channel_gone(id);
            let sends = self.hearing.channel_gone(id);
            self.deliver(sends)?;
            if let Some(mut decoder) = self.decoders.remove(&id) {
                let _ = write_to_decoder(&mut decoder.input, &ToDecoder::Close);
                drop(decoder.input);
                let _ = decoder.child.wait();
            }
            if let Some(mut fetcher) = self.fetchers.remove(&id) {
                // Closing its input ends a fetcher.
                drop(fetcher.child.stdin.take());
                let _ = fetcher.child.kill();
                let _ = fetcher.child.wait();
            }
        }
        Ok(())
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
            FromFetcher::Page { bytes, .. } => {
                // Untouched, a piece at a time; each page file ends its last
                // line, so the next one starts cleanly.
                for piece in bytes.chunks(MAX_CHUNK - 1) {
                    self.tell(channel, &ToDecoder::Data(piece.to_vec()));
                }
                self.tell(channel, &ToDecoder::Data(b"\n".to_vec()));
            }
            FromFetcher::Notice(text) => eprintln!("t42-browser: channel {channel}: {}", clean(&text, MAX_NOTICE)),
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
                eprintln!("t42-browser: cell {column},{row} of image {version} of page {:03X} on channel {}", key.page, key.channel);
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
            ToCore::Navigate(typed) => {
                if let Err(reason) = self.session.navigate(&typed) {
                    eprintln!("t42-browser: {reason}");
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
            ToCore::Point(_) | ToCore::Typed { .. } | ToCore::Focused(_) | ToCore::Confirm(_) | ToCore::Cancel(_) => self.link_intent(&intent)?,
        }
        Ok(true)
    }

    /// Hands every channel the next piece of its data, whether or not it
    /// is on screen (decision 22).
    fn load(&mut self) {
        let mut messages = Vec::new();
        for tab in &mut self.session.tabs {
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
    fn decoder_ended(&mut self, channel: u32, reason: Option<String>) -> Result<(), String> {
        if let Some(reason) = reason {
            eprintln!("t42-browser: the decoder for channel {channel} stopped: {reason}");
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
            FromDecoder::Audio(audio) => {
                if let Some(message) = self.hearing.decoded(channel, audio) {
                    self.send(&message)?;
                }
            }
        }
        Ok(())
    }
}

/// Hyperlinks (`hyperlinks.md`): the trusted core decides every link and
/// follows it by its own rules.
#[cfg(feature = "links")]
impl Core {
    fn now(&self) -> u64 {
        u64::try_from(self.started.elapsed().as_millis()).unwrap_or(u64::MAX)
    }

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
            ToCore::Focused(focused) => self.links.focused(*focused),
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
                eprintln!("t42-browser: {what} was not followed: {refusal:?}");
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
                eprintln!("t42-browser: handed {address} to the web browser, from {from}, at {at} seconds past 1970");
                Ok(())
            }
            Err(error) => {
                eprintln!("t42-browser: {address}: {error}");
                self.link(LinkMessage::Pointing(Some(Pointing { kind: LinkKind::Web, text: "Could not open your web browser".into() })))
            }
        }
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
    every(FLASH_UNIT, events.clone(), || Event::Flash);
    every(LOAD_INTERVAL, events.clone(), || Event::Load);

    let mut session = Session::new();
    for address in &addresses {
        if let Err(reason) = session.open(address) {
            eprintln!("t42-browser: {address}: {reason}");
        }
    }
    if session.tabs.is_empty() {
        session.open(HOME)?;
    }
    session.active = 0;
    let mut core = Core {
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
        #[cfg(feature = "links")]
        links: Links::default(),
        #[cfg(feature = "links")]
        started: std::time::Instant::now(),
        to_interface: BufWriter::new(interface.stdin.take().ok_or("the interface has no input")?),
    };
    core.send_all()?;

    let result = loop {
        let Ok(event) = incoming.recv() else { break Ok(()) };
        let step = match event {
            Event::Decoded(channel, message) => core.decoded(channel, message),
            Event::Fetched(channel, message) => core.fetched(channel, message),
            Event::FetcherEnded(channel, reason) => {
                eprintln!("t42-browser: the fetcher for channel {channel} stopped{}", reason.map(|reason| format!(": {reason}")).unwrap_or_default());
                if let Some(mut fetcher) = core.fetchers.remove(&channel) {
                    let _ = fetcher.child.wait();
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
            // Every decoder ticks: the shown one redraws for flash, and each
            // keeps its live sounds' clock (the engine's REQ-AUD-014). A
            // decoder showing nothing renders nothing.
            Event::Flash => {
                let channels: Vec<u32> = core.decoders.keys().copied().collect();
                for channel in channels {
                    core.tell(channel, &ToDecoder::Tick);
                }
                Ok(())
            }
            Event::DecoderEnded(channel, reason) => core.decoder_ended(channel, reason),
            Event::InterfaceEnded(None) => break Ok(()),
            Event::InterfaceEnded(Some(reason)) => break Err(format!("the interface broke the protocol and was closed: {reason}")),
        };
        if let Err(error) = step {
            break Err(error);
        }
    };

    for (_, mut fetcher) in std::mem::take(&mut core.fetchers) {
        drop(fetcher.child.stdin.take());
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
            eprintln!("t42-browser: {error}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod program_tests {
    use super::*;

    #[test]
    fn a_helper_program_has_this_platforms_file_name() {
        let expected = if cfg!(windows) { "t42-interface.exe" } else { "t42-interface" };
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
