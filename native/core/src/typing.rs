//! Typing into a page (`neoteletext-common/INPUT.md` sections 6 to 9,
//! REQ-INT-130 to REQ-INT-137): the trusted core's rules for when the
//! viewer's input goes to a channel, the input session it keeps with each
//! channel, and the cursor it passes back.
//!
//! Input goes to a channel only while its tab is active, the page the
//! viewer opened is that channel's, the window has the focus, the address
//! is not being edited, and the channel's push WebSocket is open with its
//! descriptor declaring input and its session welcomed. The interface
//! sends intents; this decides, and alone encodes the protocol
//! (REQ-INT-132). Nothing a channel or a decoder sends can start it.

use std::borrow::Cow;
use std::collections::{BTreeMap, VecDeque};

use interface_core::canvas::PageKey;
use neoteletext_input::caps::{CURSOR_CAP, ECHO_CAP, PAGE_CAP};
use neoteletext_input::cursor::Cursor;
use neoteletext_input::event::{self, Event, Item};
use neoteletext_input::session::{Session, State};
use neoteletext_input::{MAX_MESSAGE, MAX_ROWS};
use protocol::interaction::{CursorState, InputIntent, Intent, Typing as TypingState};

/// The interface's numbers of this many unacknowledged intents are kept
/// for echo, and never what they were (REQ-INT-137).
pub const MAX_UNACKNOWLEDGED: usize = neoteletext_input::session::REMEMBERED;
/// What NeoTeletext offers in HELLO (INPUT.md section 6.2): it shows pages,
/// and draws the cursor and echo.
const OFFERED: u32 = PAGE_CAP | CURSOR_CAP | ECHO_CAP;
/// A teletext page's text area, in cells (section 5.4's RESIZE): 40
/// columns; the drawn size goes as 0, unknown, as it changes with the
/// window.
const PAGE_COLUMNS: u32 = 40;
/// The page none is open, as PAGE says it (section 5.4).
const NO_PAGE: u16 = 0;

/// A message for a process.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Send {
    /// An input message for the fetcher of a channel.
    Fetcher(u32, Vec<u8>),
    /// The typing state for the interface.
    Interface(TypingState),
}

/// One channel's input connection.
#[derive(Debug)]
struct Channel {
    open: bool,
    secure: bool,
    session: Session,
    /// Welcomed, and told the size and the focus (section 6.1).
    started: bool,
    /// The input count after each of the viewer's intents sent, with the
    /// interface's number of it, oldest first.
    sent: VecDeque<(u16, u32)>,
    /// The cursor the channel placed, and for which page.
    cursor: Option<Cursor>,
    acked_local: u32,
    round_trip_ms: Option<u16>,
    /// The page the channel was last told is open, 0 for none.
    told: Option<u16>,
    /// The focus the channel was last told.
    focus_told: Option<bool>,
    /// The page the channel's last cursor record named, when it named
    /// another than the one open: the host went there.
    went: Option<u16>,
}

impl Channel {
    fn new() -> Self {
        Self {
            open: false,
            secure: false,
            session: Session::new(OFFERED),
            started: false,
            sent: VecDeque::new(),
            cursor: None,
            acked_local: 0,
            round_trip_ms: None,
            told: None,
            focus_told: None,
            went: None,
        }
    }

    /// Encodes `item` and gives it to the channel's fetcher; the input
    /// count after it, if it went.
    fn send(&mut self, channel: u32, item: &Item<'_>, now_ms: u64, sends: &mut Vec<Send>) -> Option<u16> {
        let mut out = [0_u8; MAX_MESSAGE];
        match self.session.send(item, clock(now_ms), &mut out) {
            Ok(length @ 1..) => {
                sends.push(Send::Fetcher(channel, out.get(..length).unwrap_or(&[]).to_vec()));
                Some(self.session.count())
            }
            Ok(0) | Err(_) => None,
        }
    }
}

/// `text` in Unicode Normalization Form C, borrowed when it already is.
fn nfc(text: &str) -> Cow<'_, str> {
    use unicode_normalization::{IsNormalized, UnicodeNormalization, is_nfc_quick};
    if is_nfc_quick(text.chars()) == IsNormalized::Yes { Cow::Borrowed(text) } else { Cow::Owned(text.nfc().collect()) }
}

/// The session's clock: the core's milliseconds, wrapping.
fn clock(now_ms: u64) -> u32 {
    u32::try_from(now_ms & u64::from(u32::MAX)).unwrap_or(0)
}

/// RFC 1982 serial arithmetic on 16 bits: whether `count` is `acked` or
/// before it.
fn covered(count: u16, acked: u16) -> bool {
    acked.wrapping_sub(count) < SERIAL_HALF
}

/// Half the range of the 16-bit input count (RFC 1982 section 3.2).
const SERIAL_HALF: u16 = 0x8000;

/// The trusted core's typing state for every channel.
#[derive(Debug, Default)]
pub struct Typing {
    channels: BTreeMap<u32, Channel>,
    /// The active tab's channel.
    active: Option<u32>,
    /// The page the viewer opened in the active tab.
    opened: Option<PageKey>,
    /// The window does not have the keyboard focus.
    unfocused: bool,
    /// The viewer is editing the address: the keys are the strip's.
    editing: bool,
}

impl Typing {
    /// Whether the keys are away from the page: the window lost the focus,
    /// or the address is being edited.
    fn away(&self) -> bool {
        self.unfocused || self.editing
    }

    /// Whether `channel`'s page is open in the active tab, with its input
    /// socket open: what PAGE tells it.
    fn showing(&self, channel: u32) -> bool {
        self.active == Some(channel)
            && self.opened.is_some_and(|opened| opened.channel == channel)
            && self.channels.get(&channel).is_some_and(|state| state.open)
    }

    /// Whether input goes to `channel` now (INPUT.md section 9.1): its page
    /// showing and its session welcomed.
    fn on(&self, channel: u32) -> bool {
        self.showing(channel) && self.channels.get(&channel).is_some_and(|state| state.session.welcomed())
    }

    /// The state the interface draws: for the open page of the active
    /// channel, or nothing when no page is open.
    fn state(&self) -> Option<TypingState> {
        let opened = self.opened?;
        let state = self.channels.get(&opened.channel);
        let on = self.on(opened.channel);
        let cursor = state
            .and_then(|state| state.cursor)
            .filter(|cursor| cursor.page == opened.page)
            .and_then(|cursor| cursor.place.map(|place| CursorState { row: place.row, column: place.column, width: cursor.width, flags: cursor.flags }));
        Some(TypingState {
            channel: opened.channel,
            page: opened.page,
            on,
            secure: state.is_some_and(|state| state.secure),
            refused: state.is_some_and(|state| state.open && state.session.state() == State::Refused),
            cursor: cursor.filter(|_| on),
            acked: state.map_or(0, |state| state.acked_local),
            round_trip_ms: state.and_then(|state| state.round_trip_ms),
            show: state.and_then(|state| state.went).filter(|&page| page != opened.page && on),
        })
    }

    fn tell_interface(&self, sends: &mut Vec<Send>) {
        if let Some(state) = self.state() {
            sends.push(Send::Interface(state));
        }
    }

    /// Tells `channel` which of its pages is open now, and whether it has
    /// the focus, when either changed (section 9.1): PAGE and `FOCUS_IN` when
    /// input starts going, `FOCUS_OUT` and PAGE 0 when it stops, so the
    /// channel lets go of anything held (REQ-INT-108).
    fn tell(&mut self, channel: u32, now_ms: u64, sends: &mut Vec<Send>) {
        let showing = self.showing(channel);
        let page = if showing { self.opened.map_or(NO_PAGE, |opened| opened.page) } else { NO_PAGE };
        let focus = showing && !self.away();
        let Some(state) = self.channels.get_mut(&channel) else { return };
        if !state.started {
            return;
        }
        if state.focus_told != Some(focus) && !focus {
            state.focus_told = Some(focus);
            state.send(channel, &Item::Event(Event::FocusOut), now_ms, sends);
        }
        if state.told != Some(page) {
            state.told = Some(page);
            state.send(channel, &Item::Event(Event::Page(page)), now_ms, sends);
        }
        if state.focus_told != Some(focus) && focus {
            state.focus_told = Some(focus);
            state.send(channel, &Item::Event(Event::FocusIn), now_ms, sends);
        }
    }

    /// Once welcomed, the session's start (section 6.1): RESIZE, the focus,
    /// the page.
    fn start(&mut self, channel: u32, now_ms: u64, sends: &mut Vec<Send>) {
        let Some(state) = self.channels.get_mut(&channel) else { return };
        if state.started || !state.session.welcomed() {
            return;
        }
        state.started = true;
        let resize = Event::Resize { columns: PAGE_COLUMNS, rows: MAX_ROWS, width: 0, height: 0 };
        state.send(channel, &Item::Event(resize), now_ms, sends);
        self.tell(channel, now_ms, sends);
    }

    /// Whether a page is open for typing: the active tab's open page's
    /// channel takes input now (section 9.1). Only then do a phone's keys
    /// go beyond page navigation (decision 99).
    #[must_use]
    pub fn page_takes_keys(&self) -> bool {
        self.opened.is_some_and(|opened| self.on(opened.channel))
    }

    /// The viewer opened `opened` in the active tab, whose channel is
    /// `active`, or switched tabs.
    pub fn opened(&mut self, active: Option<u32>, opened: Option<PageKey>, now_ms: u64) -> Vec<Send> {
        let mut sends = Vec::new();
        let opened = opened.filter(|key| Some(key.channel) == active);
        if (active, opened) == (self.active, self.opened) {
            return sends;
        }
        let before = (self.active, self.opened);
        self.active = active;
        self.opened = opened;
        // The viewer chose: the host's earlier word on its page is spent.
        if let Some(state) = opened.and_then(|key| self.channels.get_mut(&key.channel)) {
            state.went = None;
        }
        for channel in [before.0, active].into_iter().flatten() {
            self.tell(channel, now_ms, &mut sends);
        }
        if opened.is_none()
            && let Some(before) = before.1
        {
            // The interface stops typing into the page it had.
            sends.push(Send::Interface(TypingState {
                channel: before.channel,
                page: before.page,
                on: false,
                secure: false,
                refused: false,
                cursor: None,
                acked: 0,
                round_trip_ms: None,
                show: None,
            }));
        }
        self.tell_interface(&mut sends);
        sends
    }

    /// A channel's input socket opened or closed. Each connection is a new
    /// session (REQ-INT-106), which starts with HELLO.
    pub fn input(&mut self, channel: u32, open: bool, secure: bool, now_ms: u64) -> Vec<Send> {
        let mut sends = Vec::new();
        let state = self.channels.entry(channel).or_insert_with(Channel::new);
        let was_open = state.open;
        state.open = open;
        state.secure = open && secure;
        if open && !was_open {
            state.started = false;
            state.told = None;
            state.focus_told = None;
            state.cursor = None;
            state.sent.clear();
            state.acked_local = 0;
            state.round_trip_ms = None;
            let hello = state.session.open(clock(now_ms), None);
            let mut out = [0_u8; MAX_MESSAGE];
            if let Ok(length) = event::write(&Item::Event(hello), &mut out) {
                sends.push(Send::Fetcher(channel, out.get(..length).unwrap_or(&[]).to_vec()));
            }
        } else if !open {
            state.session.close();
            state.started = false;
            state.told = None;
            state.focus_told = None;
        }
        self.tell_interface(&mut sends);
        sends
    }

    /// The window gained or lost the keyboard focus.
    pub fn focused(&mut self, focused: bool, now_ms: u64) -> Vec<Send> {
        self.unfocused = !focused;
        self.keys_moved(now_ms)
    }

    /// The viewer began or stopped editing the address, whose keys are
    /// the strip's, not the page's.
    pub fn editing(&mut self, editing: bool, now_ms: u64) -> Vec<Send> {
        self.editing = editing;
        self.keys_moved(now_ms)
    }

    fn keys_moved(&mut self, now_ms: u64) -> Vec<Send> {
        let mut sends = Vec::new();
        if let Some(channel) = self.active {
            self.tell(channel, now_ms, &mut sends);
        }
        sends
    }

    /// An intent from the interface (or the phone, through it): encoded
    /// and sent on only under every condition of REQ-INT-130, under the
    /// session's rate; otherwise dropped.
    pub fn input_intent(&mut self, input: &InputIntent, now_ms: u64) -> Vec<Send> {
        let mut sends = Vec::new();
        let channel = input.key.channel;
        if !self.on(channel) || self.opened != Some(input.key) || self.away() {
            return sends;
        }
        let Some(state) = self.channels.get_mut(&channel) else { return sends };
        // Text goes in Normalization Form C (section 5.7 rule 3,
        // REQ-INT-116), a KEY's text too; a paste goes as it was copied.
        let normalized = match &input.intent {
            Intent::Text(text) | Intent::Key { text: Some(text), .. } => nfc(text),
            _ => Cow::Borrowed(""),
        };
        // Every key goes as KEY, carrying its text (INPUT.md section 6.3);
        // raw text is only what does not come from a key.
        let item = match &input.intent {
            Intent::Text(_) => Item::Text(&normalized),
            Intent::Key { action, key, mods, text: Some(_), base } => {
                Item::Event(Event::Key(event::Key { action: *action, key: *key, mods: *mods, text: Some(&normalized), base: *base }))
            }
            intent => intent.item(),
        };
        if let Some(count) = state.send(channel, &item, now_ms, &mut sends) {
            state.sent.push_back((count, input.local));
            if state.sent.len() > MAX_UNACKNOWLEDGED {
                state.sent.pop_front();
            }
        }
        sends
    }

    /// Whether a channel is being waited on to answer HELLO, so
    /// [`Typing::tick`] has something to time.
    #[must_use]
    pub fn waiting(&self) -> bool {
        self.channels.values().any(|state| matches!(state.session.state(), State::Waiting(_)))
    }

    /// Time passes: a channel that has not answered HELLO within
    /// `WELCOME_TIMEOUT` takes no input, and the strip says so.
    pub fn tick(&mut self, now_ms: u64) -> Vec<Send> {
        let mut sends = Vec::new();
        let mut refused = false;
        for state in self.channels.values_mut() {
            let waiting = matches!(state.session.state(), State::Waiting(_));
            state.session.tick(clock(now_ms));
            refused |= waiting && state.session.state() == State::Refused;
        }
        if refused {
            self.tell_interface(&mut sends);
        }
        sends
    }

    /// A channel no tab shows any more.
    pub fn channel_gone(&mut self, channel: u32) {
        self.channels.remove(&channel);
    }
}

mod records;

#[cfg(test)]
mod tests;
