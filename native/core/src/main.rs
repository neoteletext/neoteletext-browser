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
//! the addresses given with `--allow-local` or allowed in the settings
//! (the engine's REQ-SEC-006). With no address, the tab opens the home
//! address of the settings, which the trusted core keeps between runs
//! (`settings-home-console-proxy.md`). The decoder and the interface are
//! found beside this program.

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

mod cookie_control;
mod library;
mod page_controls;
mod session;
mod source;

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::io::{BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;
use std::sync::mpsc::{self, Sender};
use std::thread;
use std::time::Duration;

use interface_core::canvas::PageKey;
use interface_core::library::WindowGeometry;
use protocol::console::ConsoleTab;
use protocol::decoder::{FIRST_STORE_PAGES, FromDecoder, MAX_CHUNK, MAX_STORE_PAGES, MAX_WHOLE, ToDecoder, read_from_decoder, write_to_decoder};
use protocol::fetcher::{FromFetcher, MAX_NOTICE, Origin, read_from_fetcher};
use protocol::settings::{ProxyView, SettingChange, SettingsView};
use protocol::sound::{FromSound, ToSound, read_from_sound, write_to_sound};
use protocol::{ToCore, ToInterface, clean, read_from_interface, write_to_interface};

use confine::Role;
use confine::broker::{Broker, Exit, Handlers, Helper, Program};
use neoteletext::bookmarks::Bookmarks;
use neoteletext::console::{Console, exchange_text};
use neoteletext::frames;
use neoteletext::hearing::{Hearing, Send};
use neoteletext::history::History;
#[cfg(feature = "links")]
use neoteletext::links::{Channel as LinkChannel, Follow, Links, Refusal};
use neoteletext::outbox::Outbox;
use neoteletext::restore::Saved;
use neoteletext::settings::{self as stored, Settings};
use neoteletext::sleep::Sleep;
#[cfg(feature = "links")]
use protocol::links::{LinkKind, Message as LinkMessage, Pointing, Spot};

use crate::session::{Session, Went};
use crate::source::Next;

#[cfg(feature = "phone-keyboard")]
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
/// How long the window is still before its place is kept: a drag is one
/// write (`history-and-bookmarks.md` section 15.2).
const WINDOW_STILL: Duration = Duration::from_secs(1);
/// How long the strip says a notice (`settings-home-console-proxy-security.md`
/// section 3).
const NOTICE_TIME: Duration = Duration::from_secs(10);

enum Event {
    Decoded(u32, u64, FromDecoder),
    Fetched(u32, u64, FromFetcher),
    FetcherEnded(u32, u64, Option<String>),
    DecoderEnded(u32, u64, Option<String>),
    Intent(ToCore),
    InterfaceEnded(Option<String>),
    /// The phone keyboard process said something, untrusted.
    #[cfg(feature = "phone-keyboard")]
    Phone(protocol::keyboard::FromKeyboard),
    /// The phone keyboard process ended.
    #[cfg(feature = "phone-keyboard")]
    PhoneEnded(Option<String>),
    /// The sound helper said something, untrusted (decision 55).
    Sound(FromSound),
    /// The sound helper ended: the browser runs silent.
    SoundEnded(Option<String>),
}

struct Decoder {
    child: Helper,
    input: Outbox,
    /// Which of the channel's decoders this is: one put to sleep may still
    /// report after the next has started, and is then not listened to.
    generation: u64,
}

struct Fetcher {
    child: Helper,
    /// As [`Decoder::generation`].
    generation: u64,
    /// What a fetcher is given: input messages (REQ-INT-133), and the
    /// answers to its cookie questions (decision 62).
    input: Option<Outbox>,
}

/// Asks the broker for a helper (decision 56): it starts it confined as
/// its program's role says (decision 52), with an environment of only
/// what it needs, and passes the trusted core its pipes.
fn spawn(broker: &Arc<Broker>, program: Program, arguments: &[String], local_network: bool) -> Result<Helper, String> {
    let helper = broker.spawn(program, arguments, local_network).map_err(|error| format!("cannot start the {}: {error}", program.name()))?;
    eprintln!("neoteletext: the {} started confined: {}", program.name(), helper.report);
    Ok(helper)
}

/// Ends a decoder: told to close, and killed rather than waited for, so
/// one that has stopped reading holds nothing up. It has nothing the
/// trusted core needs: what it was given is kept (decision 47).
fn stop_decoder(mut decoder: Decoder) {
    let _ = write_to_decoder(&mut decoder.input, &ToDecoder::Close);
    drop(decoder.input);
    let _ = decoder.child.kill();
    let _ = decoder.child.wait();
}

/// Starts a decoder for `channel`, whose messages arrive as events.
fn start_decoder(broker: &Arc<Broker>, channel: u32, generation: u64, store_pages: usize, events: Sender<Event>) -> Result<Decoder, String> {
    let mut child = spawn(broker, Program::Decoder, &[String::from("--store-pages"), store_pages.to_string()], false)?;
    let output = child.stdout.take().ok_or("the decoder has no output")?;
    let input = Outbox::new(child.stdin.take().ok_or("the decoder has no input")?);
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
/// A fetcher is given its proxy only here, when it starts: a change of the
/// proxy reaches fetchers started after it (requirement 19.12).
fn start_fetcher(broker: &Arc<Broker>, address: &str, reach: &Reach, channel: u32, generation: u64, events: Sender<Event>) -> Result<Fetcher, String> {
    let mut arguments = vec![address.to_owned()];
    for ip in &reach.allow_local {
        arguments.push(String::from("--allow-local"));
        arguments.push(ip.clone());
    }
    if let Some(proxy) = &reach.proxy {
        arguments.push(String::from("--proxy"));
        arguments.push(proxy.clone());
    }
    let mut child = spawn(broker, Program::Fetcher, &arguments, !reach.allow_local.is_empty())?;
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
    let input = child.stdin.take().map(Outbox::new);
    Ok(Fetcher { child, generation, input })
}

fn read_interface(child: &mut Helper, events: Sender<Event>) -> Result<(), String> {
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

/// Where a new fetcher may connect: the local addresses allowed, from the
/// command line and the settings, and the proxy of the settings.
struct Reach {
    allow_local: Vec<String>,
    proxy: Option<String>,
}

impl Reach {
    fn of(command_line: &[String], settings: &Settings) -> Self {
        let mut allow_local = command_line.to_vec();
        allow_local.extend(settings.local.iter().map(ToString::to_string));
        Self { allow_local, proxy: settings.proxy.as_ref().map(stored::Proxy::url) }
    }
}

struct Core {
    session: Session,
    decoders: BTreeMap<u32, Decoder>,
    /// Starts helpers, reads file channels and opens web addresses for the
    /// core (decision 56); none in a test.
    broker: Option<Arc<Broker>>,
    fetchers: BTreeMap<u32, Fetcher>,
    /// Local-network servers the user allowed with `--allow-local`.
    allow_local: Vec<String>,
    /// The settings and where they are kept (requirement 19.1).
    settings: Settings,
    settings_folder: Option<PathBuf>,
    /// Why the last change the panel asked for was refused.
    refusal: Option<String>,
    /// What the strip says, until when (requirements 19.3, 19.10).
    notice: Option<(String, std::time::Instant)>,
    /// The developer console's entries (requirement 19.9).
    console: Console,
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
    /// When the trusted core started: the clock of links, of typing and of
    /// the console.
    started: std::time::Instant,
    /// When the channels are next given data, and the other work of that
    /// pace is done, while any of it is wanted ([`Core::loading_wanted`]).
    next_load: std::time::Instant,
    /// When next to see which tabs may sleep (decision 47).
    next_sleep_check: std::time::Instant,
    /// Typing into a page (`interaction.md`).
    #[cfg(feature = "interaction")]
    typing: neoteletext::typing::Typing,
    /// The page the viewer opened in the active tab.
    #[cfg(feature = "interaction")]
    opened: Option<PageKey>,
    /// The phone keyboard process, while one runs.
    #[cfg(feature = "phone-keyboard")]
    phone: Option<(Helper, Outbox)>,
    /// What the interface shows of it.
    #[cfg(feature = "phone-keyboard")]
    phone_view: protocol::keyboard::PhoneView,
    to_interface: Outbox,
    /// The sound helper's pipe, while it runs (decision 55).
    sound: Option<(Helper, Outbox)>,
    /// History, and the panels of history and bookmarks
    /// (`history-and-bookmarks.md`).
    library: library::Library,
    /// The window's place and size kept with the session, and one the
    /// interface reported since, kept once the window is still: a drag is
    /// one write (`history-and-bookmarks.md` section 15.2).
    window: Option<WindowGeometry>,
    window_moved: Option<(WindowGeometry, std::time::Instant)>,
    /// Every cookie, and the cookies panel (`cookies.md`).
    cookies: cookie_control::Cookies,
    /// The open page's controls (`page-controls.md`, decision 67).
    controls: page_controls::State,
    /// The session's text last written, so an unchanged one is not.
    kept_session: String,
}

impl Core {
    /// Milliseconds since the trusted core started.
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

    /// The keyboard's intents: input for the open page's channel, and the
    /// phone keyboard's panel.
    #[cfg(feature = "interaction")]
    fn input_intent(&mut self, intent: ToCore) -> Result<(), String> {
        match intent {
            ToCore::Input(intent) => {
                let sends = self.typing.input_intent(&intent, self.now());
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

    /// Lets time pass for typing: a channel that has not answered HELLO
    /// in time takes no input (INPUT.md section 6.1).
    #[cfg_attr(not(feature = "interaction"), allow(clippy::unnecessary_wraps, clippy::unused_self, reason = "only typing has a time"))]
    fn typing_tick(&mut self) -> Result<(), String> {
        #[cfg(feature = "interaction")]
        {
            let sends = self.typing.tick(self.now());
            self.deliver_typing(sends)?;
        }
        Ok(())
    }

    /// Delivers what the typing rules decided: input messages to a
    /// channel's fetcher, the typing state to the interface.
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
    fn deliver(&mut self, sends: Vec<Send>) {
        for send in sends {
            match send {
                Send::Decoder(channel, message) => self.tell(channel, message),
                Send::Sound(message) => self.tell_sound(&message),
            }
        }
    }

    fn tell(&mut self, channel: u32, message: ToDecoder) {
        if let Some(decoder) = self.decoders.get_mut(&channel) {
            // A decoder that cannot be written to has stopped; its reader
            // reports that, and its pages stay as they were.
            let _ = write_to_decoder(&mut decoder.input, &message);
        }
        // Kept whether or not the channel is awake: a sleeping channel's
        // pushed pages are there when it wakes.
        self.sleep.keep(channel, message);
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
                let mut decoder = start_decoder(self.broker()?, *id, self.generation, pages, self.events.clone())?;
                for message in self.sleep.kept(*id) {
                    let _ = write_to_decoder(&mut decoder.input, message);
                }
                self.decoders.insert(*id, decoder);
            } else if !awake && self.decoders.contains_key(id) {
                self.put_to_sleep(*id);
            }
            // A web channel's pages come from its own fetcher, which runs
            // for as long as its tab is open, asleep or not: fetching never
            // depends on viewing (decision 22).
            if let Some(address) = web
                && !self.fetchers.contains_key(id)
            {
                self.generation = self.generation.wrapping_add(1);
                let reach = Reach::of(&self.allow_local, &self.settings);
                let fetcher = start_fetcher(self.broker()?, address, &reach, *id, self.generation, self.events.clone())?;
                self.fetchers.insert(*id, fetcher);
            }
        }
        let unused: Vec<u32> = self.decoders.keys().chain(self.fetchers.keys()).copied().filter(|id| !channels.iter().any(|(used, ..)| used == id)).collect();
        for id in unused {
            self.put_to_sleep(id);
            self.stop_fetcher(id);
            self.sleep.forget(id);
            self.console.forget(id);
        }
        Ok(())
    }

    /// Closes a channel's decoder, and forgets what it showed: its links,
    /// its typing and its sounds.
    fn put_to_sleep(&mut self, id: u32) {
        #[cfg(feature = "links")]
        self.links.channel_gone(id);
        #[cfg(feature = "interaction")]
        self.typing.channel_gone(id);
        let sends = self.hearing.channel_gone(id);
        self.deliver(sends);
        if let Some(decoder) = self.decoders.remove(&id) {
            stop_decoder(decoder);
        }
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
        if let Some(decoder) = self.decoders.remove(&id) {
            stop_decoder(decoder);
        }
        self.generation = self.generation.wrapping_add(1);
        let mut decoder = start_decoder(self.broker()?, id, self.generation, pages, self.events.clone())?;
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
            drop(fetcher.input.take());
            let _ = fetcher.child.kill();
            let _ = fetcher.child.wait();
        }
    }

    fn fetched(&mut self, channel: u32, message: FromFetcher) -> Result<(), String> {
        self.console_fetched(channel, &message);
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
                    self.tell(channel, ToDecoder::Data(piece.to_vec()));
                }
                self.tell(channel, ToDecoder::Data(b"\n".to_vec()));
            }
            // A file or a pushed message of a form the fetcher knows goes
            // whole and at once, so its pages show as it arrives
            // (`interaction.md` section 10).
            FromFetcher::Page { origin, bytes } => {
                if bytes.len() <= MAX_WHOLE {
                    let packets = matches!(origin, Origin::PushedPackets | Origin::FetchedPackets);
                    let pushed = matches!(origin, Origin::PushedPackets | Origin::PushedPage);
                    self.tell(channel, ToDecoder::Whole { packets, pushed, bytes });
                }
            }
            FromFetcher::Notice(text) => eprintln!("neoteletext: channel {channel}: {}", clean(&text, MAX_NOTICE)),
            // Said in the trusted core's own words, of the channel's host as
            // the strip shows it: a fetcher cannot write the strip.
            FromFetcher::ProxyFailed => {
                let host = self.session.tabs.iter().find(|tab| tab.channel.id == channel).and_then(|tab| tab.address()).map(|address| address.host.clone());
                if let Some(host) = host {
                    let text = format!("The proxy refused the connection to {host}");
                    self.console.add(channel, ConsoleTab::Notices, self.now(), &text);
                    if self.active_channel() == Some(channel) {
                        self.say(text)?;
                    } else {
                        eprintln!("neoteletext: {text}");
                    }
                }
            }
            FromFetcher::Exchange { .. } => {}
            message @ (FromFetcher::CookieQuestion { .. }
            | FromFetcher::ReachedQuestion { .. }
            | FromFetcher::CookieSet { .. }
            | FromFetcher::ClearSiteData(_)) => {
                self.cookie_message(channel, message)?;
            }
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
            self.tell(previous, ToDecoder::Showing(Vec::new()));
        }
        let tabs = self.session.tab_infos();
        let active = self.session.active;
        self.send(&ToInterface::Tabs { tabs, active })?;
        self.send(&ToInterface::Strip(self.strip()))?;
        if let Some(tab) = self.session.active_tab() {
            let channel = tab.channel.view();
            self.send(&ToInterface::Channel(channel))?;
        }
        self.send_console()?;
        #[cfg(feature = "links")]
        self.send_links_afresh()?;
        self.send_mutes();
        // An address that names a sub-page opens its page there (decision 67).
        if let Some(tab) = self.session.active_tab()
            && let Some(address) = tab.address()
            && let Some(page) = address.page
        {
            let key = PageKey { channel: tab.channel.id, page };
            self.open_at_subpage(key, address.subpage);
        }
        Ok(())
    }

    /// Tells the interface each tab's channel's mute, where it changed: a
    /// muted tab's new channel starts muted.
    fn send_mutes(&mut self) {
        let tabs: Vec<(u32, bool)> = self.session.tabs.iter().map(|tab| (tab.channel.id, tab.muted)).collect();
        let sends = tabs.into_iter().filter_map(|(channel, muted)| self.hearing.mute(channel, muted)).collect();
        self.deliver(sends);
    }

    /// The tabs again, after a change of mute, and the mutes themselves.
    fn send_tabs(&mut self) -> Result<(), String> {
        let tabs = self.session.tab_infos();
        let active = self.session.active;
        self.send(&ToInterface::Tabs { tabs, active })?;
        self.send_mutes();
        Ok(())
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
                self.keep_subpages(&visible);
                self.frames.set_visible(visible);
                self.shown_pages.clone_from(&pages);
                self.tell(channel, ToDecoder::Showing(pages));
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
                self.send_history()?;
            }
            ToCore::NewTab => {
                let home = self.settings.home.clone();
                open_home(&mut self.session, &home)?;
                self.send_all()?;
            }
            // The start page's bookmark pages open their bookmarks, as the
            // user's own navigation (requirement 19.7).
            ToCore::Opened(Some(key)) if self.session.start_page_bookmark(key.channel, key.page).is_some() => self.open_start_page_bookmark(key)?,
            ToCore::Opened(key) => self.opened(key)?,
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
                if let Err(reason) = self.session.navigate(&typed, true) {
                    eprintln!("neoteletext: {reason}");
                }
                self.send_all()?;
            }
            ToCore::Back | ToCore::Forward => {
                let went = self.session.go(matches!(intent, ToCore::Forward));
                self.went(went)?;
            }
            ToCore::Reload | ToCore::PageControl(_) => self.page_intent(&intent)?,
            ToCore::Bookmark => self.star()?,
            ToCore::Home | ToCore::ChangeSetting(_) | ToCore::Console(_) => self.settings_intent(intent)?,
            ToCore::History(_)
            | ToCore::Bookmarks(_)
            | ToCore::Star(_)
            | ToCore::Bar(_)
            | ToCore::Suggest(_)
            | ToCore::Suggestion { .. }
            | ToCore::TabList(_)
            | ToCore::GoTo { .. }
            | ToCore::NewPrivateTab => self.library_intent(intent)?,
            ToCore::Window(window) => self.window_moved = std::time::Instant::now().checked_add(WINDOW_STILL).map(|still| (window, still)),
            ToCore::Cookies(request) => self.cookies_request(request)?,
            #[cfg(feature = "links")]
            ToCore::Point(_) | ToCore::Typed { .. } | ToCore::Confirm(_) | ToCore::Cancel(_) => self.link_intent(&intent)?,
            #[cfg(any(feature = "links", feature = "interaction"))]
            ToCore::Focused(focused) => self.focused(focused)?,
            #[cfg(feature = "interaction")]
            ToCore::Input(_) | ToCore::Editing(_) => self.input_intent(intent)?,
            #[cfg(feature = "phone-keyboard")]
            ToCore::PhonePanel(_) | ToCore::PhoneFinish => self.input_intent(intent)?,
        }
        // Another page of a channel already drawing is a visit at once.
        self.record_drawn()?;
        self.private_cookies_closed();
        self.keep_session();
        Ok(true)
    }

    /// Keeps the window's place the interface last reported.
    fn keep_window(&mut self) {
        if let Some((window, _)) = self.window_moved.take() {
            self.window = Some(window);
            self.keep_session();
        }
    }

    /// Writes the session if it changed, so that it can always be restored
    /// (requirement 20.21); a failure is logged.
    fn keep_session(&mut self) {
        let Some(folder) = &self.settings_folder else { return };
        let text = self.session.saved(self.window).text();
        if text == self.kept_session {
            return;
        }
        match Saved::save(folder, &text) {
            Ok(()) => self.kept_session = text,
            Err(error) => eprintln!("neoteletext: the session could not be kept in {}: {error}", folder.display()),
        }
    }

    /// Reload, and the open page's controls (decision 67).
    fn page_intent(&mut self, intent: &ToCore) -> Result<(), String> {
        match intent {
            ToCore::Reload => self.reload(),
            ToCore::PageControl(control) => self.page_control(*control),
            _ => Ok(()),
        }
    }

    /// The page open in the active tab changed: its keys, its links, its
    /// sounds and its controls follow it.
    fn opened(&mut self, key: Option<PageKey>) -> Result<(), String> {
        #[cfg(feature = "interaction")]
        {
            self.opened = key;
            let sends = self.typing.opened(self.active_channel(), key, self.now());
            self.deliver_typing(sends)?;
        }
        #[cfg(feature = "links")]
        self.links.opened(key);
        let sends = self.hearing.open(self.active_channel(), key);
        self.deliver(sends);
        self.page_opened(key)
    }

    /// Reload: the active tab's channel opens afresh (requirement 11.2).
    fn reload(&mut self) -> Result<(), String> {
        if self.session.reload() { self.send_all() } else { Ok(()) }
    }

    /// After Back, Forward or a step from their lists: another channel is
    /// sent afresh; another page of the same channel stays open.
    fn went(&mut self, went: Went) -> Result<(), String> {
        match went {
            Went::Nowhere => Ok(()),
            Went::Channel => self.send_all(),
            #[cfg_attr(not(feature = "links"), allow(unused_variables, reason = "only a link opens a page"))]
            Went::Page(page) => {
                self.send(&ToInterface::Strip(self.strip()))?;
                #[cfg(feature = "links")]
                if let (Some(page), Some(channel)) = (page, self.active_channel()) {
                    self.link(LinkMessage::Show(PageKey { channel, page }))?;
                }
                Ok(())
            }
        }
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
            self.tell(channel, message);
        }
    }

    /// A decoder that broke the protocol is closed; its channel's pages stay
    /// as they were, and its sounds go.
    fn decoder_ended(&mut self, channel: u32, generation: u64, reason: Option<String>) {
        // A decoder put to sleep, or one before it, has nothing more to say.
        if self.decoders.get(&channel).map(|decoder| decoder.generation) != Some(generation) {
            return;
        }
        if let Some(reason) = reason {
            eprintln!("neoteletext: the decoder for channel {channel} stopped: {reason}");
            let at = self.now();
            self.console.add(channel, ConsoleTab::Notices, at, &format!("the decoder stopped: {reason}"));
        }
        if let Some(mut decoder) = self.decoders.remove(&channel) {
            let _ = decoder.child.kill();
            let _ = decoder.child.wait();
        }
        let sends = self.hearing.channel_gone(channel);
        self.deliver(sends);
    }

    fn decoded(&mut self, channel: u32, message: FromDecoder) -> Result<(), String> {
        let active = self.active_channel();
        match message {
            FromDecoder::StoreFull => self.grow_decoder(channel)?,
            FromDecoder::Arrived { page, title } => {
                let Some(tab) = self.session.tabs.iter_mut().find(|tab| tab.channel.id == channel) else { return Ok(()) };
                let title = tab.channel.arrived(page, &title);
                if let Some(title) = &title {
                    let at = self.now();
                    self.console.add(channel, ConsoleTab::Pages, at, &format!("page {page:03X} arrived: {title}"));
                }
                if let Some(title) = title
                    && active == Some(channel)
                {
                    self.send(&ToInterface::Page { channel, page, title })?;
                    // A number on a page shown becomes a link once its page exists.
                    #[cfg(feature = "links")]
                    self.redecide_channel(channel)?;
                }
            }
            FromDecoder::Updated { page } => {
                let at = self.now();
                self.console.add(channel, ConsoleTab::Pages, at, &format!("page {page:03X} changed"));
                if active == Some(channel) {
                    self.send(&ToInterface::Updated { channel, page })?;
                }
            }
            FromDecoder::Frame { page, palette, screen, indices } => {
                let key = PageKey { channel, page };
                match self.frames.accept(key, palette, screen, indices) {
                    Some((version, image)) => {
                        // The image's links first, so the interface has them
                        // when the image arrives (`hyperlinks.md` section 8.1).
                        #[cfg(feature = "links")]
                        self.links_of_image(key, version)?;
                        self.send(&ToInterface::Frame { key, version, image })?;
                        // The page drew: its tab's visit is recorded
                        // (requirement 20.1).
                        self.record_pending(channel)?;
                    }
                    // The same image again: only its links may have changed.
                    #[cfg(feature = "links")]
                    None => self.redecide(key)?,
                    #[cfg(not(feature = "links"))]
                    None => {}
                }
                // Its Fastext keys may lead elsewhere now.
                self.send_controls()?;
            }
            #[cfg(feature = "links")]
            FromDecoder::Links { page, with_image, candidates, numbers, fastext } => {
                let key = PageKey { channel, page };
                self.links.found(key, candidates, numbers, fastext);
                if !with_image {
                    self.redecide(key)?;
                    self.send_controls()?;
                }
            }
            FromDecoder::SubPages { page, subcodes, shown } => self.subpages(PageKey { channel, page }, subcodes, shown)?,
            #[cfg(feature = "interaction")]
            FromDecoder::Cursor { body } => {
                let sends = self.typing.cursor(channel, body, self.now());
                self.deliver_typing(sends)?;
            }
            #[cfg(feature = "interaction")]
            FromDecoder::InputControl { body } => {
                let sends = self.typing.control(channel, &body, self.now());
                self.deliver_typing(sends)?;
            }
            FromDecoder::Audio(audio) => {
                if let Some(message) = self.hearing.decoded(channel, audio) {
                    self.tell_sound(&message);
                }
            }
        }
        Ok(())
    }
}

/// Settings, the strip's notice and the developer console
/// (`settings-home-console-proxy.md`).
impl Core {
    /// The strip as the session has it, with the notice while it lasts.
    fn strip(&self) -> interface_core::browser::StripState {
        let mut strip = self.session.strip();
        if let Some((text, until)) = &self.notice
            && std::time::Instant::now() < *until
        {
            strip.notice = clean(text, protocol::MAX_STRIP_NOTICE);
        }
        strip
    }

    /// The strip's intents of the settings: Home, a change the panel asks
    /// for, and the console.
    fn settings_intent(&mut self, intent: ToCore) -> Result<(), String> {
        match intent {
            ToCore::Home => {
                let home = self.settings.home.clone();
                if let Err(reason) = self.session.navigate(&home, false) {
                    eprintln!("neoteletext: {home}: {reason}");
                }
                self.send_all()
            }
            ToCore::ChangeSetting(change) => self.change_setting(change),
            ToCore::Console(open) => {
                self.console.set_open(open);
                self.send_console()
            }
            _ => Ok(()),
        }
    }

    /// A bookmark page of the start page was opened: its bookmark opens.
    fn open_start_page_bookmark(&mut self, key: PageKey) -> Result<(), String> {
        if let Some(bookmark) = self.session.start_page_bookmark(key.channel, key.page)
            && let Err(reason) = self.session.open_from_start_page(&bookmark)
        {
            eprintln!("neoteletext: {reason}");
        }
        self.send_all()
    }

    /// Says `text` in the strip for a while, and in the log.
    fn say(&mut self, text: String) -> Result<(), String> {
        eprintln!("neoteletext: {text}");
        let now = std::time::Instant::now();
        self.notice = Some((text, now.checked_add(NOTICE_TIME).unwrap_or(now)));
        self.send(&ToInterface::Strip(self.strip()))
    }

    /// The notice's time is up: the strip shows the address again.
    fn notice_tick(&mut self) -> Result<(), String> {
        if self.notice.as_ref().is_some_and(|(_, until)| std::time::Instant::now() >= *until) {
            self.notice = None;
            self.send(&ToInterface::Strip(self.strip()))?;
        }
        Ok(())
    }

    /// What a fetcher said, as the console's entries: exchanges and push
    /// events, packets, notices (requirement 19.9).
    fn console_fetched(&mut self, channel: u32, message: &FromFetcher) {
        let at = self.now();
        match message {
            FromFetcher::Page { origin, bytes } => {
                let pushed = matches!(origin, Origin::PushedPackets | Origin::PushedPage);
                let packets = matches!(origin, Origin::PushedPackets | Origin::FetchedPackets);
                if packets {
                    self.console.packets(channel, at, pushed, bytes);
                }
                if pushed {
                    let form = if packets { "packets" } else { "a page" };
                    self.console.add(channel, ConsoleTab::Network, at, &format!("push: {form}, {} bytes", bytes.len()));
                }
            }
            FromFetcher::Pushing(open) => {
                self.console.add(channel, ConsoleTab::Network, at, if *open { "push: connected" } else { "push: lost" });
            }
            FromFetcher::Exchange { method, url, status, bytes, elapsed_us } => {
                self.console.add(channel, ConsoleTab::Network, at, &exchange_text(method, url, *status, *bytes, *elapsed_us));
            }
            FromFetcher::Notice(text) => self.console.add(channel, ConsoleTab::Notices, at, text),
            _ => {}
        }
    }

    /// Sends the open tab's new console entries, while the console is open.
    fn send_console(&mut self) -> Result<(), String> {
        for update in self.console.updates(self.active_channel()) {
            self.send(&ToInterface::Console(update))?;
        }
        Ok(())
    }

    /// Sends what the last event produced to each process, before waiting
    /// for the next. A decoder that has stopped reading is replaced, with
    /// what it was given (decision 47); a fetcher's or the phone's lost
    /// input is theirs to notice; the interface is the browser.
    fn flush(&mut self) -> Result<(), String> {
        let stuck: Vec<u32> = self.decoders.iter_mut().filter_map(|(&id, decoder)| decoder.input.flush().is_err().then_some(id)).collect();
        for id in stuck {
            eprintln!("neoteletext: the decoder for channel {id} stopped reading and is replaced");
            self.put_to_sleep(id);
        }
        for fetcher in self.fetchers.values_mut() {
            if let Some(input) = fetcher.input.as_mut() {
                let _ = input.flush();
            }
        }
        #[cfg(feature = "phone-keyboard")]
        if let Some((_, input)) = self.phone.as_mut() {
            let _ = input.flush();
        }
        if let Some((_, input)) = self.sound.as_mut()
            && input.flush().is_err()
        {
            eprintln!("neoteletext: no sound: the sound helper stopped reading");
            self.sound = None;
        }
        self.to_interface.flush().map_err(|error| format!("cannot write to the interface: {error}"))?;
        // A replaced decoder starts again at once if its tab is awake.
        if self.decoders.len() < self.session.tabs.len() {
            self.match_decoders()?;
        }
        Ok(())
    }

    /// Whether anything is done at the loading pace (`LOAD_INTERVAL`): a
    /// channel's file still being fed, a channel being waited on to take
    /// keys, or the console open to be sent what comes.
    fn loading_wanted(&self) -> bool {
        let feeding = self.session.tabs.iter().any(|tab| self.decoders.contains_key(&tab.channel.id) && tab.channel.source.has_more());
        #[cfg(feature = "interaction")]
        let typing = self.typing.waiting();
        #[cfg(not(feature = "interaction"))]
        let typing = false;
        feeding || typing || self.console.is_open()
    }

    /// When the trusted core next has timed work, if it has any: it waits
    /// for an event until then, and with none, for an event alone
    /// (decision 46).
    fn next_wake(&self) -> Option<std::time::Instant> {
        let mut next = Some(self.next_sleep_check);
        if self.loading_wanted() {
            next = next.map(|at| at.min(self.next_load));
        }
        if let Some((_, until)) = &self.notice {
            next = next.map(|at| at.min(*until));
        }
        if let Some((_, still)) = self.window_moved {
            next = next.map(|at| at.min(still));
        }
        if let Some(due) = self.cookies.due {
            next = next.map(|at| at.min(due));
        }
        next
    }

    /// The timed work now due.
    fn tick(&mut self) -> Result<(), String> {
        let now = std::time::Instant::now();
        if now >= self.next_load && self.loading_wanted() {
            self.next_load = now.checked_add(LOAD_INTERVAL).unwrap_or(now);
            self.load();
            self.typing_tick()?;
            self.send_console()?;
        }
        self.notice_tick()?;
        if now >= self.next_sleep_check {
            self.next_sleep_check = now.checked_add(SLEEP_CHECK).unwrap_or(now);
            self.match_decoders()?;
        }
        if self.window_moved.is_some_and(|(_, still)| now >= still) {
            self.keep_window();
        }
        if self.cookies.due.is_some_and(|due| now >= due) {
            self.keep_cookies();
        }
        Ok(())
    }

    /// Passes a checked voice operation or the settings to the sound
    /// helper; with none, the browser is silent and they go nowhere.
    fn tell_sound(&mut self, message: &ToSound) {
        if let Some((_, input)) = self.sound.as_mut() {
            let _ = write_to_sound(input, message);
        }
    }

    /// Which channels sound, as the sound helper said: a sounding tab stays
    /// awake (decision 47) and shows its speaker.
    fn sounding(&mut self, channels: Vec<u32>) -> Result<(), String> {
        self.sleep.set_sounding(&channels);
        self.send(&ToInterface::Sounding(channels))
    }

    /// The broker, which a test's core has none of.
    fn broker(&self) -> Result<&Arc<Broker>, String> {
        self.broker.as_ref().ok_or_else(|| String::from("no broker: this trusted core starts nothing"))
    }

    /// Starts the sound helper, whose messages arrive as events; when it
    /// cannot start, as on a Linux without the system's sound library, the
    /// browser runs silent (requirement 16.6).
    fn start_sound(&mut self) {
        let Ok(broker) = self.broker().cloned() else { return };
        let mut child = match spawn(&broker, Program::Sound, &[], false) {
            Ok(child) => child,
            Err(error) => {
                eprintln!("neoteletext: no sound: {error}");
                return;
            }
        };
        let (Some(output), Some(input)) = (child.stdout.take(), child.stdin.take()) else { return };
        let events = self.events.clone();
        thread::spawn(move || {
            let mut output = BufReader::new(output);
            let end = loop {
                match read_from_sound(&mut output) {
                    Ok(Some(message)) => {
                        if events.send(Event::Sound(message)).is_err() {
                            return;
                        }
                    }
                    Ok(None) => break None,
                    Err(error) => break Some(error.to_string()),
                }
            };
            let _ = events.send(Event::SoundEnded(end));
        });
        self.sound = Some((child, Outbox::new(input)));
        let message = ToSound::Settings { sound: self.settings.sound, volume: self.settings.volume };
        self.tell_sound(&message);
    }

    /// The settings as the panel shows them, with the last refusal; the
    /// sound helper is told the sound and the volume (requirement 19.4).
    fn send_settings(&mut self) -> Result<(), String> {
        let message = ToSound::Settings { sound: self.settings.sound, volume: self.settings.volume };
        self.tell_sound(&message);
        let proxy = match &self.settings.proxy {
            None => ProxyView::None,
            Some(stored::Proxy::Http { host, port }) => ProxyView::Http { host: clean(host, protocol::settings::MAX_PROXY_HOST), port: *port },
            Some(stored::Proxy::Socks5 { host, port }) => ProxyView::Socks5 { host: clean(host, protocol::settings::MAX_PROXY_HOST), port: *port },
        };
        let view = SettingsView {
            home: clean(&self.settings.home, protocol::MAX_ADDRESS),
            sound: self.settings.sound,
            volume: self.settings.volume,
            proxy,
            local: self.settings.local.iter().map(ToString::to_string).collect(),
            restore_session: self.settings.restore,
            cookies: cookie_control::mode_view(self.settings.cookies),
            refusal: self.refusal.as_deref().map(|refusal| clean(refusal, protocol::settings::MAX_REFUSAL)),
        };
        self.send(&ToInterface::Settings(view))
    }

    /// Writes the settings, if there is a place for them; a failure is
    /// logged and said in the panel.
    fn save_settings(&mut self) {
        let Some(folder) = &self.settings_folder else { return };
        if let Err(error) = self.settings.save(folder) {
            eprintln!("neoteletext: the settings could not be saved in {}: {error}", folder.display());
            self.refusal = Some("The settings could not be saved".into());
        }
    }

    /// A change the panel asked for, checked as strip input (requirement
    /// 19.3): kept and saved when the rules accept it, said in the panel
    /// when not; a change of a security setting is said in the strip and
    /// logged.
    fn change_setting(&mut self, change: SettingChange) -> Result<(), String> {
        let current = self.session.active_tab().and_then(|tab| tab.address()).map(|address| address.channel_text().to_owned());
        let cookies = if let SettingChange::Cookies(mode) = change { Some(mode) } else { None };
        match checked_change(&mut self.settings, change, current.as_deref()) {
            Ok(security) => {
                self.refusal = None;
                self.save_settings();
                if let Some(mode) = cookies {
                    self.cookie_mode_changed(mode);
                }
                if let Some(text) = security {
                    self.say(text)?;
                }
            }
            Err(reason) => self.refusal = Some(reason),
        }
        self.send_settings()?;
        self.send_history()
    }
}

/// Applies one change to `settings` if its setting's rule accepts it;
/// returns what the strip says of a change of a security setting, or why
/// the change was refused. `current` is the open channel's address.
fn checked_change(settings: &mut Settings, change: SettingChange, current: Option<&str>) -> Result<Option<String>, String> {
    const NOT_LOCAL: &str = "Not an address on this computer or the local network";
    match change {
        SettingChange::Home(typed) => {
            let address = neoteletext::address::parse(&typed)?;
            address.channel_text().clone_into(&mut settings.home);
            Ok(None)
        }
        SettingChange::HomeIsThisPage => {
            current.ok_or("No channel is open")?.clone_into(&mut settings.home);
            Ok(None)
        }
        SettingChange::Sound(on) => {
            settings.sound = on;
            Ok(None)
        }
        SettingChange::Volume(volume) if volume <= stored::MAX_VOLUME => {
            settings.volume = volume;
            Ok(None)
        }
        SettingChange::Volume(_) => Err("The volume is from 0 to 100".into()),
        SettingChange::Proxy(ProxyView::None) => {
            let was = settings.proxy.take();
            Ok(was.map(|_| "No proxy: channels opened from now connect directly".to_owned()))
        }
        SettingChange::Proxy(view) => {
            let (kind, host, port) = match &view {
                ProxyView::Http { host, port } => ("http", host, *port),
                ProxyView::Socks5 { host, port } => ("socks5", host, *port),
                ProxyView::None => return Ok(None),
            };
            let proxy = stored::Proxy::from_parts(kind, host.trim(), port).ok_or("Not a proxy: a host and a port from 1 to 65535")?;
            let text = format!("Proxy {}: channels opened from now go through it", proxy.url());
            settings.proxy = Some(proxy);
            Ok(Some(text))
        }
        SettingChange::AddLocal(typed) => {
            let address: std::net::IpAddr = typed.trim().parse().map_err(|_| NOT_LOCAL)?;
            if !stored::is_local(address) {
                return Err(NOT_LOCAL.into());
            }
            if settings.local.contains(&address) {
                return Err(format!("{address} is allowed already"));
            }
            if settings.local.len() >= stored::MAX_LOCAL {
                return Err(format!("At most {} local addresses can be allowed", stored::MAX_LOCAL));
            }
            settings.local.push(address);
            Ok(Some(format!("{address} on the local network allowed for channels opened from now")))
        }
        SettingChange::RemoveLocal(index) => {
            let index = usize::from(index);
            if index >= settings.local.len() {
                return Err("No such address".into());
            }
            let address = settings.local.remove(index);
            Ok(Some(format!("{address} on the local network no longer allowed")))
        }
        // Restoring the last session is the user's privacy, not a security
        // setting the strip says (requirement 20.24).
        SettingChange::RestoreSession(on) => {
            settings.restore = on;
            Ok(None)
        }
        // And what is kept of cookies (requirement 21.7).
        SettingChange::Cookies(mode) => {
            settings.cookies = match mode {
                protocol::settings::CookieMode::Keep => neoteletext::cookies::Mode::Keep,
                protocol::settings::CookieMode::Close => neoteletext::cookies::Mode::Close,
                protocol::settings::CookieMode::Block => neoteletext::cookies::Mode::Block,
            };
            Ok(None)
        }
    }
}

/// The tabs at start (`history-and-bookmarks.md` section 15.3): the last
/// session's, when it is restored, then the command line's addresses, the
/// last of them active; with neither, the home address. Without a session
/// restored, the first tab is active, as before.
fn open_at_start(session: &mut Session, saved: Option<&Saved>, addresses: &[String], home: &str) -> Result<(), String> {
    session.now = neoteletext::calendar::utc_now();
    let restored = saved.is_some_and(|saved| session.restore(saved) > 0);
    for address in addresses {
        if let Err(reason) = session.open(address) {
            eprintln!("neoteletext: {address}: {reason}");
        }
    }
    if session.tabs.is_empty() {
        open_home(session, home)?;
    }
    if !restored {
        session.active = 0;
    }
    Ok(())
}

/// Opens the home address in a new tab; one that cannot be opened, as a
/// file gone, opens the start page instead.
fn open_home(session: &mut Session, home: &str) -> Result<(), String> {
    if let Err(reason) = session.open(home) {
        eprintln!("neoteletext: {home}: {reason}");
        session.open(stored::START_PAGE)?;
    }
    Ok(())
}

/// The phone keyboard (INPUT.md section 10, REQ-INT-138): a
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
        let mut arguments = Vec::new();
        // Without one, the keyboard process uses qrkeyboard's own relay. Only
        // an address is passed on: anything else would be read as a flag.
        if let Ok(relay) = std::env::var(RELAY_VARIABLE)
            && (relay.starts_with("https://") || relay.starts_with("http://"))
        {
            arguments.push(relay);
        }
        // The proxy, given only at its start (requirement 19.12).
        if let Some(proxy) = &self.settings.proxy {
            arguments.push(String::from("--proxy"));
            arguments.push(proxy.url());
        }
        let mut child = spawn(self.broker()?, Program::Keyboard, &arguments, true)?;
        let output = child.stdout.take().ok_or("the phone keyboard has no output")?;
        let input = Outbox::new(child.stdin.take().ok_or("the phone keyboard has no input")?);
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
                // A link to the page shown reloads it, as in Chrome.
                let shown = self.session.active_tab().and_then(|tab| tab.address()).is_some_and(|shown| shown.page == Some(page));
                if shown {
                    return self.reload();
                }
                if self.session.show_page(page).is_some() {
                    self.send(&ToInterface::Strip(self.strip()))?;
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
        match self.broker()?.open_web(address) {
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
    fn new(
        session: Session,
        allow_local: Vec<String>,
        kept: (Settings, Option<PathBuf>),
        events: Sender<Event>,
        to_interface: Outbox,
        broker: Option<Arc<Broker>>,
    ) -> Self {
        let (settings, settings_folder) = kept;
        Self {
            session,
            decoders: BTreeMap::new(),
            broker,
            fetchers: BTreeMap::new(),
            allow_local,
            settings,
            settings_folder,
            refusal: None,
            notice: None,
            console: Console::default(),
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
            started: std::time::Instant::now(),
            next_load: std::time::Instant::now(),
            next_sleep_check: std::time::Instant::now().checked_add(SLEEP_CHECK).unwrap_or_else(std::time::Instant::now),
            #[cfg(feature = "interaction")]
            typing: neoteletext::typing::Typing::default(),
            #[cfg(feature = "interaction")]
            opened: None,
            #[cfg(feature = "phone-keyboard")]
            phone: None,
            #[cfg(feature = "phone-keyboard")]
            phone_view: protocol::keyboard::PhoneView { state: protocol::keyboard::PhoneState::Finished, qr: None, pin: None },
            to_interface,
            sound: None,
            library: library::Library::default(),
            window: None,
            window_moved: None,
            cookies: cookie_control::Cookies::default(),
            controls: page_controls::State::default(),
            kept_session: String::new(),
        }
    }
}

/// The settings kept between runs, and where (requirement 19.1); lines
/// dropped by their rules are logged (requirement 19.2).
fn load_settings() -> (Settings, Option<PathBuf>) {
    let folder = stored::folder();
    let settings = match &folder {
        Some(folder) => match Settings::load(folder) {
            Ok((settings, dropped)) => {
                for line in dropped {
                    eprintln!("neoteletext: {line}");
                }
                settings
            }
            Err(error) => {
                eprintln!("neoteletext: the settings in {} could not be read: {error}", folder.display());
                Settings::default()
            }
        },
        None => Settings::default(),
    };
    (settings, folder)
}

/// The bookmarks kept between runs (requirement 20.19); lines dropped by
/// their rules are logged (requirement 20.20).
fn load_bookmarks(folder: Option<&Path>) -> Bookmarks {
    let Some(folder) = folder else { return Bookmarks::default() };
    match Bookmarks::load(folder) {
        Ok((bookmarks, dropped)) => {
            for line in dropped {
                eprintln!("neoteletext: {line}");
            }
            bookmarks
        }
        Err(error) => {
            eprintln!("neoteletext: the bookmarks in {} could not be read: {error}", folder.display());
            Bookmarks::default()
        }
    }
}

/// History kept between runs (requirement 20.2); lines dropped by their
/// rules are logged (requirement 20.3).
fn load_history(folder: Option<&Path>) -> History {
    let Some(folder) = folder else { return History::default() };
    match History::load(folder) {
        Ok((history, dropped)) => {
            for line in dropped {
                eprintln!("neoteletext: {line}");
            }
            history
        }
        Err(error) => {
            eprintln!("neoteletext: the history in {} could not be read: {error}", folder.display());
            History::default()
        }
    }
}

/// The session kept from the last run (`history-and-bookmarks.md` section
/// 15); lines dropped by their rules are logged (requirement 20.23).
fn load_session(folder: Option<&Path>) -> Option<Saved> {
    match Saved::load(folder?) {
        Ok((saved, dropped)) => {
            for line in dropped {
                eprintln!("neoteletext: {line}");
            }
            saved
        }
        Err(error) => {
            eprintln!("neoteletext: the last session could not be read: {error}");
            None
        }
    }
}

/// Handles one event; `Ok(false)` once the browser is to end.
fn handle(core: &mut Core, event: Event) -> Result<bool, String> {
    match event {
        Event::Decoded(channel, generation, message) => {
            if core.decoders.get(&channel).map(|decoder| decoder.generation) == Some(generation) {
                core.decoded(channel, message)?;
            }
        }
        Event::Fetched(channel, generation, message) => {
            if core.fetchers.get(&channel).map(|fetcher| fetcher.generation) == Some(generation) {
                core.fetched(channel, message)?;
            }
        }
        Event::FetcherEnded(channel, generation, reason) => {
            if core.fetchers.get(&channel).map(|fetcher| fetcher.generation) == Some(generation) {
                eprintln!("neoteletext: the fetcher for channel {channel} stopped{}", reason.map(|reason| format!(": {reason}")).unwrap_or_default());
                if let Some(mut fetcher) = core.fetchers.remove(&channel) {
                    let _ = fetcher.child.wait();
                }
            }
        }
        Event::Intent(intent) => {
            // What the intent changes in a tab's list is stamped with now.
            core.session.now = neoteletext::calendar::utc_now();
            return core.intent(intent);
        }
        Event::DecoderEnded(channel, generation, reason) => core.decoder_ended(channel, generation, reason),
        #[cfg(feature = "phone-keyboard")]
        Event::Phone(message) => core.phone_said(message)?,
        #[cfg(feature = "phone-keyboard")]
        Event::PhoneEnded(reason) => core.phone_ended(reason)?,
        Event::Sound(FromSound::Sounding(channels)) => core.sounding(channels)?,
        Event::SoundEnded(reason) => {
            eprintln!("neoteletext: no sound: the sound helper ended{}", reason.map(|reason| format!(": {reason}")).unwrap_or_default());
            core.sound = None;
            core.sounding(Vec::new())?;
        }
        Event::InterfaceEnded(None) => return Ok(false),
        Event::InterfaceEnded(Some(reason)) => return Err(format!("the interface broke the protocol and was closed: {reason}")),
    }
    Ok(true)
}

fn run(broker: &Arc<Broker>, given: impl Iterator<Item = String>) -> Result<(), String> {
    let mut addresses = Vec::new();
    let mut allow_local = Vec::new();
    let mut arguments = given;
    while let Some(argument) = arguments.next() {
        if argument == "--allow-local" {
            allow_local.push(arguments.next().ok_or("--allow-local needs an address")?);
        } else {
            addresses.push(argument);
        }
    }
    let mut interface = spawn(broker, Program::Interface, &[], false)?;
    let (events, incoming) = mpsc::channel();
    read_interface(&mut interface, events.clone())?;

    let (settings, settings_folder) = load_settings();
    let bookmarks = load_bookmarks(settings_folder.as_deref());
    let saved = load_session(settings_folder.as_deref()).filter(|_| settings.restore);
    // The window's place first: the interface opens its window with the
    // first message it is sent (section 15.3).
    let mut to_interface = Outbox::new(interface.stdin.take().ok_or("the interface has no input")?);
    let window = saved.as_ref().and_then(|saved| saved.window);
    if let Some(window) = window {
        write_to_interface(&mut to_interface, &ToInterface::Window(window)).map_err(|error| format!("cannot write to the interface: {error}"))?;
        to_interface.flush().map_err(|error| format!("cannot write to the interface: {error}"))?;
    }
    // File channels are read by the broker: the core reads no file but its
    // settings, history and bookmarks (decision 56).
    let reader = Arc::clone(broker);
    let mut session = Session::with_reader(bookmarks, Box::new(move |path: &Path| reader.read_source(path).map(source::Source::from_bytes)));
    open_at_start(&mut session, saved.as_ref(), &addresses, &settings.home)?;
    let history = load_history(settings_folder.as_deref());
    let mut core = Core::new(session, allow_local, (settings, settings_folder), events, to_interface, Some(Arc::clone(broker)));
    core.library.history = history;
    core.window = window;
    core.cookies_at_start();
    core.keep_session();
    core.start_sound();
    core.send_settings()?;
    core.send_all()?;
    core.send_bar()?;
    core.flush()?;

    let result = loop {
        // An event, or the time for timed work, whichever is first.
        let event = match core.next_wake() {
            Some(at) => match incoming.recv_timeout(at.saturating_duration_since(std::time::Instant::now())) {
                Ok(event) => Some(event),
                Err(mpsc::RecvTimeoutError::Timeout) => None,
                Err(mpsc::RecvTimeoutError::Disconnected) => break Ok(()),
            },
            None => match incoming.recv() {
                Ok(event) => Some(event),
                Err(_) => break Ok(()),
            },
        };
        let Some(event) = event else {
            if let Err(error) = core.tick().and_then(|()| core.flush()) {
                break Err(error);
            }
            continue;
        };
        // Timed work falls due while events are handled too: a feed that
        // just started, a notice that expired, the sleep check.
        match handle(&mut core, event).and_then(|go| core.tick().and_then(|()| core.flush()).map(|()| go)) {
            Ok(true) => {}
            Ok(false) => break Ok(()),
            Err(error) => break Err(error),
        }
    };

    // A move not yet kept, and cookies not yet written or to remove.
    core.keep_window();
    core.cookies_at_exit();
    for (_, mut fetcher) in std::mem::take(&mut core.fetchers) {
        drop(fetcher.child.stdin.take());
        drop(fetcher.input.take());
        let _ = fetcher.child.kill();
        let _ = fetcher.child.wait();
    }
    for (_, decoder) in std::mem::take(&mut core.decoders) {
        stop_decoder(decoder);
    }
    if let Some((mut helper, mut input)) = core.sound.take() {
        let _ = write_to_sound(&mut input, &ToSound::Close);
        let _ = input.flush();
        drop(input);
        let _ = helper.kill();
        let _ = helper.wait();
    }
    let _ = write_to_interface(&mut core.to_interface, &ToInterface::Close);
    let _ = core.to_interface.flush();
    drop(core);
    if result.is_err() {
        let _ = interface.kill();
    }
    let status = interface.wait().map_err(|error| format!("cannot wait for the interface: {error}"))?;
    ended(result, &status)
}

/// The browser's outcome once the interface has ended: an interface that
/// crashed or failed is a failure even when it closed its pipe cleanly, as
/// a crashing process does.
fn ended(result: Result<(), String>, interface: &Exit) -> Result<(), String> {
    result?;
    if interface.success { Ok(()) } else { Err(format!("the interface ended abnormally ({interface})")) }
}

/// The trusted core's own flag: the broker starts this program again
/// with it (decision 56).
const CORE_FLAG: &str = "--core";

/// Folders under the home folder whose files are the user's secrets: the
/// broker never reads a file channel there for the core, however it is
/// named (decision 56).
const SECRET_FOLDERS: &[&str] = &[".ssh", ".gnupg", ".aws", ".kube", ".password-store", ".config/gcloud", ".docker", "Library/Keychains", "Library/Cookies"];

/// The trusted core: confined first (decision 56), then the browser.
fn core() -> Result<(), String> {
    let report = confine::apply(Role::Core, stored::folder().as_deref())?;
    eprintln!("neoteletext: the trusted core is confined: {report}");
    let broker = Broker::from_parent().map_err(|error| error.to_string())?;
    run(&broker, std::env::args().skip(2))
}

/// What the cookie file's key is kept under in the platform's secret store
/// (`cookies.md` section 7): a name a person reading the store recognises.
const KEY_SERVICE: &str = "NeoTeletext cookie file";
const KEY_ACCOUNT: &str = "cookies.bin";

/// The cookie file's key, read by the broker from the platform's secret
/// store, or made there the first time from the system's random source
/// (requirement 21.4). The trusted core cannot reach the store.
fn cookie_key() -> Result<Vec<u8>, String> {
    let entry = keyring::Entry::new(KEY_SERVICE, KEY_ACCOUNT).map_err(|error| format!("no secret store: {error}"))?;
    match entry.get_secret() {
        Ok(key) => Ok(key),
        Err(keyring::Error::NoEntry) => {
            use ring::rand::SecureRandom as _;
            let mut key = vec![0_u8; confine::broker::COOKIE_KEY_BYTES];
            ring::rand::SystemRandom::new().fill(&mut key).map_err(|_| String::from("no random source"))?;
            entry.set_secret(&key).map_err(|error| format!("the secret store refused the key: {error}"))?;
            Ok(key)
        }
        Err(error) => Err(format!("the secret store refused: {error}")),
    }
}

/// A file channel's bytes, read by the broker for the core: never from a
/// folder of the user's secrets, and bounded as `Source::read` bounds it.
fn read_for_core(path: &Path) -> Result<Vec<u8>, String> {
    let canonical = path.canonicalize().map_err(|error| format!("{}: {error}", path.display()))?;
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).map(PathBuf::from);
    if let Some(home) = home.and_then(|home| home.canonicalize().ok())
        && SECRET_FOLDERS.iter().any(|secret| canonical.starts_with(home.join(secret)))
    {
        return Err(format!("{}: not a channel", path.display()));
    }
    source::Source::read(&canonical).map(source::Source::into_bytes)
}

/// The broker (decision 56): the program the user starts, and the one
/// process not confined. It starts the trusted core confined, then serves
/// its requests until it ends.
fn broker() -> Result<(), String> {
    let program = std::env::current_exe().map_err(|error| format!("cannot find this program: {error}"))?;
    let programs = program.parent().ok_or("this program has no directory")?.to_path_buf();
    let settings = stored::folder().ok_or("there is no folder for the settings")?;
    std::fs::create_dir_all(&settings).map_err(|error| format!("{}: {error}", settings.display()))?;
    let mut arguments = vec![OsString::from(CORE_FLAG)];
    arguments.extend(std::env::args_os().skip(1));
    let environment = confine::environment(Role::Core);
    let core = confine::broker::start_core(&program, &arguments, &settings, &environment).map_err(|error| format!("cannot start the trusted core: {error}"))?;
    eprintln!("neoteletext: the trusted core started confined: {}", core.report);
    #[cfg(feature = "links")]
    let open_web = |address: &str| web_opener::open(address).map_err(|error| error.to_string());
    // With links compiled out the core never asks; if it does, it is refused.
    #[cfg(not(feature = "links"))]
    let open_web = |_: &str| Err(String::from("links are compiled out"));
    let handlers = Handlers { read_source: &read_for_core, open_web: &open_web, environment: &confine::environment, cookie_key: &cookie_key };
    let exit = confine::broker::serve(core, &programs, &handlers).map_err(|error| format!("cannot wait for the trusted core: {error}"))?;
    if exit.success { Ok(()) } else { Err(format!("the trusted core ended abnormally ({exit})")) }
}

fn main() -> ExitCode {
    let step = if std::env::args().nth(1).as_deref() == Some(CORE_FLAG) { core() } else { broker() };
    match step {
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

    /// Section 15.3 and requirement 20.24: the tabs at start.
    #[test]
    fn the_tabs_at_start() {
        let addresses = |session: &Session| session.tabs.iter().map(|tab| tab.address().unwrap().text.clone()).collect::<Vec<_>>();
        let mut kept = Session::new(Bookmarks::default());
        kept.open("news.invalid").unwrap();
        kept.open("sport.invalid").unwrap();
        kept.active = 0;
        let saved = kept.saved(None);
        let given = ["weather.invalid".to_owned(), "tv.invalid".to_owned()];

        let mut session = Session::new(Bookmarks::default());
        open_at_start(&mut session, Some(&saved), &[], stored::START_PAGE).unwrap();
        assert_eq!((addresses(&session), session.active), (vec!["https://news.invalid/".to_owned(), "https://sport.invalid/".to_owned()], 0), "as it was");

        let mut session = Session::new(Bookmarks::default());
        open_at_start(&mut session, Some(&saved), &given, stored::START_PAGE).unwrap();
        assert_eq!(addresses(&session).len(), 4);
        assert_eq!(session.active, 3, "the last address given is active");

        let mut session = Session::new(Bookmarks::default());
        open_at_start(&mut session, None, &given, stored::START_PAGE).unwrap();
        assert_eq!((addresses(&session), session.active), (vec!["https://weather.invalid/".to_owned(), "https://tv.invalid/".to_owned()], 0), "as before");

        let mut session = Session::new(Bookmarks::default());
        open_at_start(&mut session, Some(&Saved::default()), &[], stored::START_PAGE).unwrap();
        assert_eq!(addresses(&session), [stored::START_PAGE], "nothing kept: the home address");
    }

    /// Requirement 19.3: each change is checked by its setting's rule; a
    /// refused one changes nothing and says why; a security setting's
    /// change is said.
    #[test]
    fn each_setting_change_is_checked() {
        let mut settings = Settings::default();
        let mut change = |change: SettingChange| checked_change(&mut settings, change, Some("https://open.example/"));
        assert_eq!(change(SettingChange::Home("news.example/teletext/#302".into())), Ok(None));
        assert!(change(SettingChange::Home("javascript:alert(1)".into())).is_err());
        assert!(change(SettingChange::Home("neoteletext:settings".into())).is_err());
        assert_eq!(change(SettingChange::Volume(101)), Err("The volume is from 0 to 100".into()));
        assert_eq!(change(SettingChange::Volume(30)), Ok(None));
        assert_eq!(change(SettingChange::Sound(false)), Ok(None));
        assert_eq!(change(SettingChange::Proxy(ProxyView::None)), Ok(None), "no proxy before: nothing to say");
        assert!(change(SettingChange::Proxy(ProxyView::Http { host: "bad host".into(), port: 8080 })).is_err());
        assert!(change(SettingChange::Proxy(ProxyView::Socks5 { host: "p.example".into(), port: 0 })).is_err());
        let said = change(SettingChange::Proxy(ProxyView::Socks5 { host: " 127.0.0.1 ".into(), port: 1080 })).unwrap().unwrap();
        assert!(said.contains("socks5://127.0.0.1:1080"), "{said}");
        assert!(change(SettingChange::Proxy(ProxyView::None)).unwrap().is_some(), "removing it is said");
        for refused in ["93.184.216.34", "not an address", "8.8.8.8", "2001:4860::8888"] {
            assert_eq!(change(SettingChange::AddLocal(refused.into())), Err("Not an address on this computer or the local network".into()), "{refused}");
        }
        assert!(change(SettingChange::AddLocal("192.168.1.20".into())).unwrap().is_some());
        assert!(change(SettingChange::AddLocal("192.168.1.20".into())).is_err(), "once");
        for n in 0..20 {
            let _ = change(SettingChange::AddLocal(format!("10.0.0.{n}")));
        }
        assert!(change(SettingChange::RemoveLocal(16)).is_err());
        assert!(change(SettingChange::RemoveLocal(0)).unwrap().is_some());
        assert_eq!(change(SettingChange::HomeIsThisPage), Ok(None));
        // A privacy setting, not a security one: nothing said in the strip.
        assert_eq!(change(SettingChange::RestoreSession(false)), Ok(None));
        assert!(!settings.restore);
        assert_eq!(settings.home, "https://open.example/");
        assert_eq!((settings.volume, settings.sound, settings.proxy.is_none()), (30, false, true));
        assert_eq!(settings.local.len(), stored::MAX_LOCAL - 1);
        assert!(!settings.local.contains(&"192.168.1.20".parse().unwrap()));
        assert_eq!(checked_change(&mut settings, SettingChange::HomeIsThisPage, None), Err("No channel is open".into()));
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

    #[test]
    fn a_helper_program_has_this_platforms_file_name() {
        let expected = if cfg!(windows) { "neoteletext-interface.exe" } else { "neoteletext-interface" };
        assert_eq!(Program::Interface.file_name(), expected);
    }

    /// Decision 56: the broker reads no file channel in a folder of the
    /// user's secrets for the core, however the path is written.
    #[cfg(unix)]
    #[test]
    fn the_broker_reads_no_secret_for_the_core() {
        let Some(home) = std::env::var_os("HOME").map(PathBuf::from) else { return };
        let ssh = home.join(".ssh");
        if !ssh.is_dir() {
            return;
        }
        let refused = read_for_core(&ssh.join("..").join(".ssh")).unwrap_err();
        assert!(refused.ends_with("not a channel"), "{refused}");
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
        let clean = Exit::from(std::process::ExitStatus::from_raw(0));
        assert_eq!(ended(Ok(()), &clean), Ok(()));
        let crashed = Exit::from(std::process::ExitStatus::from_raw(SEGMENTATION_FAULT));
        assert!(ended(Ok(()), &crashed).is_err_and(|error| error.contains("abnormally")), "a crash is not a clean exit");
        let failed = Exit::from(std::process::ExitStatus::from_raw(1 << EXIT_CODE_SHIFT));
        assert!(ended(Ok(()), &failed).is_err());
        assert_eq!(ended(Err("first".into()), &crashed), Err("first".into()), "the first error is the one reported");
    }
    /// A trusted core for a test: no tabs, no helpers, the interface a
    /// buffer.
    fn test_core() -> Core {
        let (events, _incoming) = mpsc::channel();
        Core::new(Session::new(Bookmarks::default()), Vec::new(), (Settings::default(), None), events, Outbox::new(Vec::new()), None)
    }

    /// Decision 53: the trusted core waits for an event until its next
    /// deadline, which is the sleep check alone while nothing is fed, no
    /// typing waits and the console is closed; the console open, it is
    /// the loading pace. Before this, two threads woke it every 200 ms and
    /// every 60 s for the life of the process.
    #[test]
    fn the_core_wakes_only_for_work_it_has() {
        let mut core = test_core();
        let now = std::time::Instant::now();
        let idle = core.next_wake().unwrap();
        assert!(idle >= now + SLEEP_CHECK.saturating_sub(Duration::from_secs(1)), "the sleep check alone: {:?}", idle - now);
        core.console.set_open(true);
        assert!(core.next_wake().unwrap() <= std::time::Instant::now(), "the console open: due now");
        core.tick().unwrap();
        let paced = core.next_wake().unwrap();
        assert!(paced > std::time::Instant::now() && paced <= std::time::Instant::now() + LOAD_INTERVAL, "then at the loading pace");
        core.console.set_open(false);
        assert!(core.next_wake().unwrap() >= now + SLEEP_CHECK.saturating_sub(Duration::from_secs(1)), "closed: the sleep check alone");
    }

    /// Decision 53: a decoder that stops reading is given up once 16 MB
    /// wait for it, and killed; the trusted core's thread never waits on
    /// it. The decoder here is `sleep`, which reads nothing. Before this,
    /// the core blocked on the pipe and every tab with it.
    #[cfg(unix)]
    #[test]
    fn a_decoder_that_stops_reading_is_killed_and_forgotten() {
        let mut core = test_core();
        let program = PathBuf::from("/bin/sleep");
        let arguments = [OsString::from("60")];
        let mut child = Helper::local(
            confine::spawn(&confine::Spawn {
                program: &program,
                arguments: &arguments,
                role: Role::Decoder,
                environment: &[],
                local_network: false,
                writable: None,
            })
            .unwrap(),
        );
        let input = Outbox::new(child.stdin.take().unwrap());
        core.decoders.insert(1, Decoder { child, input, generation: 1 });
        let started = std::time::Instant::now();
        let chunk = vec![0_u8; MAX_CHUNK - 1];
        let chunks = neoteletext::outbox::MAX_QUEUED_BYTES / chunk.len() + 2;
        for _ in 0..chunks {
            core.tell(1, ToDecoder::Data(chunk.clone()));
        }
        core.flush().unwrap();
        assert!(!core.decoders.contains_key(&1), "given up");
        assert!(started.elapsed() < Duration::from_secs(5), "never waited on it: {:?}", started.elapsed());
    }
}
