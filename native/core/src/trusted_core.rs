//! The trusted core itself (design section 5, decision 56): the `Core`
//! that keeps the tabs and their channels, its helper processes, its
//! event loop, and every concern it drives, one module each below. It
//! runs confined, started by the broker (`main.rs`) with `--core`.

mod channels;
mod console;
mod cookies;
mod decoder_messages;
mod fetcher_messages;
#[cfg(test)]
mod hostile_tests;
mod intents;
mod library;
#[cfg(feature = "links")]
mod links;
mod page_controls;
#[cfg(feature = "phone-keyboard")]
mod phone;
mod processes;
mod run;
mod settings;
mod sound;
mod startup;
mod strip;
#[cfg(test)]
mod test_support;
mod timing;
#[cfg(feature = "interaction")]
mod typing;
mod updates;
mod windows;

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::Sender;
use std::time::Duration;

use confine::broker::{Broker, Helper};
#[cfg(feature = "interaction")]
use interface_core::canvas::PageKey;
use interface_core::library::WindowGeometry;
use neoteletext::console::Console;
use neoteletext::frames;
use neoteletext::hearing::{Hearing, Send};
#[cfg(feature = "links")]
use neoteletext::links::Links;
use neoteletext::outbox::Outbox;
use neoteletext::settings::Settings;
use neoteletext::sleep::Sleep;
use protocol::decoder::{BookmarkEntry, FromDecoder};
use protocol::fetcher::FromFetcher;
use protocol::sound::FromSound;
use protocol::{ToCore, ToInterface, write_to_interface};

use crate::session::Session;

use processes::{Decoder, Fetcher};
pub(crate) use run::run;
use windows::Shown;

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
    /// A decoder read another browser's bookmark file (decision 71).
    BookmarksRead(Result<(Vec<BookmarkEntry>, u32), String>),
    /// The updater said something, untrusted (`installation-and-updates.md`).
    Updater(update::wire::FromUpdater),
    /// The updater ended.
    UpdaterEnded(Option<String>),
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
    /// A window's place and size the interface reported, kept with the
    /// session once the window is still: a drag is one write
    /// (`history-and-bookmarks.md` section 15.2).
    window_moved: Option<(u32, WindowGeometry, std::time::Instant)>,
    /// The window the interface was last told the messages are for
    /// (`multiple-windows.md`).
    targeted: u32,
    /// What each window set aside shows: the frames sent, the channel told
    /// which pages are shown, and those pages.
    shown_elsewhere: BTreeMap<u32, Shown>,
    /// Windows whose views of what every window shares (the star, the
    /// bar, the settings, the remote) changed while they were set aside:
    /// sent again when next at hand.
    stale: std::collections::BTreeSet<u32>,
    /// The interface's intents name a window that is not open, as one just
    /// closed: they are not acted on until it names one that is.
    from_closed: bool,
    /// Every cookie, and the cookies panel (`cookies.md`).
    cookies: cookies::Cookies,
    /// The open page's controls (`page-controls.md`, decision 67).
    controls: page_controls::State,
    /// The session's text last written, so an unchanged one is not.
    kept_session: String,
    /// Updates (`installation-and-updates.md`).
    updates: updates::Updates,
}

impl Core {
    /// Milliseconds since the trusted core started.
    fn now(&self) -> u64 {
        u64::try_from(self.started.elapsed().as_millis()).unwrap_or(u64::MAX)
    }

    /// Sends `message` to the window at hand, telling the interface first
    /// when the window changed (`multiple-windows.md`).
    fn send(&mut self, message: &ToInterface) -> Result<(), String> {
        let window = self.session.window;
        if window != self.targeted {
            self.send_untargeted(&ToInterface::Target(window))?;
            self.targeted = window;
        }
        self.send_untargeted(message)
    }

    /// Sends a message that is no window's: one that opens or closes a
    /// window.
    fn send_untargeted(&mut self, message: &ToInterface) -> Result<(), String> {
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

    fn active_channel(&self) -> Option<u32> {
        self.session.active_tab().map(|tab| tab.channel.id)
    }

    /// The broker, which a test's core has none of.
    fn broker(&self) -> Result<&Arc<Broker>, String> {
        self.broker.as_ref().ok_or_else(|| String::from("no broker: this trusted core starts nothing"))
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
            window_moved: None,
            targeted: crate::session::FIRST_WINDOW,
            shown_elsewhere: BTreeMap::new(),
            stale: std::collections::BTreeSet::new(),
            from_closed: false,
            cookies: cookies::Cookies::default(),
            controls: page_controls::State::default(),
            kept_session: String::new(),
            updates: updates::Updates::default(),
        }
    }
}
