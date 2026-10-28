//! Typing into a page (`neoteletext-common/INPUT.md` sections 6 to 9,
//! REQ-INT-122 to REQ-INT-127 and REQ-INT-130 to REQ-INT-137): the trusted
//! core's rules for when the viewer's input goes to a channel, the input
//! session it keeps with each channel, the pointer's pace and contacts, and
//! the cursor it passes back.
//!
//! Input goes to a channel only while its tab is active, the page the
//! viewer opened is that channel's, the window has the focus, the address
//! is not being edited, and the channel's push WebSocket is open with its
//! descriptor declaring input and its session welcomed. The interface
//! sends intents; this decides, paces the pointer's moves, numbers the
//! contacts of a touch screen, and alone encodes the protocol (REQ-INT-132,
//! `RETURN-CHANNEL.md` REQ-RCH-036 and REQ-RCH-053). Nothing a channel or a
//! decoder sends can start it.
//!
//! Times are microseconds of the trusted core's clock: the pointer's
//! period of 1000/60 ms is not a whole number of milliseconds
//! (`neoteletext_input::pace`).

use std::borrow::Cow;
use std::collections::BTreeMap;

use interface_core::canvas::PageKey;
use neoteletext_input::MAX_CONTACTS;
use neoteletext_input::caps::{CURSOR_CAP, PAGE_CAP, RESUME_CAP, TOUCH_CAP};
use neoteletext_input::contacts::Contacts;
use neoteletext_input::cursor::{BLOCK, Cursor};
use neoteletext_input::event::{self, Event, Item, Mouse, MouseAction, Point, Touch};
use neoteletext_input::pace::Pacer;
use neoteletext_input::session::{ResumableSession, State};
use neoteletext_input::{MAX_MESSAGE, MAX_ROWS};
use protocol::interaction::{CursorState, InputIntent, Intent, Typing as TypingState};

/// What NeoTeletext offers in HELLO (INPUT.md section 6.2): it shows pages,
/// draws the cursor, and sends touch (REQ-RCH-041 to REQ-RCH-043); the
/// resumable session adds RESUME (section 6.4, REQ-RCH-013). It echoes
/// nothing: what is typed shows only as the channel draws it (the owner,
/// 28 October 2026).
const OFFERED: u32 = PAGE_CAP | CURSOR_CAP | TOUCH_CAP;
/// A teletext page's text area, in cells (section 5.4's RESIZE): 40
/// columns; the drawn size goes as 0, unknown, as it changes with the
/// window.
const PAGE_COLUMNS: u32 = 40;
/// The page none is open, as PAGE says it (section 5.4).
const NO_PAGE: u16 = 0;
/// Microseconds in the session's millisecond.
const MICROSECONDS_PER_MILLISECOND: u64 = 1_000;

/// A message for a process.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Send {
    /// An input message for the fetcher of a channel.
    Fetcher {
        /// The channel.
        channel: u32,
        /// The encoded message.
        bytes: Vec<u8>,
        /// The contact whose move it is, alone (0 the mouse's, 1 to 9 a
        /// TOUCH's): the fetcher holds it while the socket is congested
        /// (REQ-RCH-035).
        move_of: Option<u8>,
        /// More messages of the same moment follow for the same channel:
        /// written together, in one send (REQ-RCH-010).
        more: bool,
    },
    /// Measure the channel's round trip by ping and pong, or stop
    /// (REQ-RCH-016): on exactly while keys go to it.
    Measure {
        /// The channel.
        channel: u32,
        /// Measure.
        on: bool,
    },
    /// The typing state for the interface.
    Interface(TypingState),
}

/// One channel's input connection.
#[derive(Debug)]
struct Channel {
    open: bool,
    secure: bool,
    /// The session, kept across a lost connection when the channel set
    /// RESUME (INPUT.md section 6.4).
    session: ResumableSession,
    /// Welcomed, and told the size and the focus (section 6.1).
    started: bool,
    /// The cursor the channel placed, and for which page.
    cursor: Option<Cursor>,
    /// The push connection's last round trip, from the fetcher.
    round_trip_us: Option<u32>,
    /// The page the channel was last told is open, 0 for none.
    told: Option<u16>,
    /// The focus the channel was last told, which is whether keys go to it
    /// and the round trip is measured.
    focus_told: Option<bool>,
    /// The page the channel's last cursor record named, when it named
    /// another than the one open: the host went there.
    went: Option<u16>,
    /// The input count of the last PAGE sent, until a cursor record
    /// acknowledges it. A record the channel wrote before it read that
    /// PAGE names the page it had, which is no going of the host's: taken
    /// as one, the browser and the channel would send each other between
    /// the two pages for as long as records crossed.
    page_unacked: Option<u16>,
    /// The pointer's and each contact's moves, at most 60 a second
    /// (REQ-RCH-031 to REQ-RCH-033, REQ-RCH-045).
    pacer: Pacer,
    /// The contacts on a touch screen, numbered (REQ-RCH-043, REQ-RCH-053).
    contacts: Contacts,
    /// The system's identifier of the first contact, while it is down: MOUSE
    /// is then a finger's.
    finger: Option<u64>,
    /// The mouse's buttons held, as sent: bit `n` is button `n`.
    held: u8,
    /// Where each contact was last sent, 0 the mouse's: what its gone says
    /// (section 5.4), and whether the channel knows of it at all.
    last_at: [Option<Point>; MAX_CONTACTS],
}

impl Channel {
    fn new() -> Self {
        Self {
            open: false,
            secure: false,
            session: ResumableSession::new(OFFERED),
            started: false,
            cursor: None,
            round_trip_us: None,
            told: None,
            focus_told: None,
            went: None,
            page_unacked: None,
            pacer: Pacer::new(),
            contacts: Contacts::new(),
            finger: None,
            held: 0,
            last_at: [None; MAX_CONTACTS],
        }
    }

    /// Encodes `item` and gives it to the channel's fetcher; the input
    /// count after it, if it went. A move is tagged with its contact, and
    /// every pointer event's position is kept for its gone.
    fn send(&mut self, channel: u32, item: &Item<'_>, now_us: u64, sends: &mut Vec<Send>) -> Option<u16> {
        // MOUSE's and TOUCH's action 3, gone, came with the engine's E.133;
        // a channel built before it reads action 3 as a protocol error and
        // ends the session, so gone goes only to a channel that agreed a
        // capability of the same change, TOUCH or RESUME, and so knows it.
        if is_gone(item) && self.session.caps() & (TOUCH_CAP | RESUME_CAP) == 0 {
            return None;
        }
        let mut out = [0_u8; MAX_MESSAGE];
        match self.session.send(item, clock(now_us), &mut out) {
            Ok(length @ 1..) => {
                let move_of = match item {
                    Item::Event(event) => event::move_contact(event).and_then(|contact| u8::try_from(contact).ok()),
                    Item::Text(_) => None,
                };
                sends.push(Send::Fetcher { channel, bytes: out.get(..length).unwrap_or(&[]).to_vec(), move_of, more: false });
                self.sent_pointer(item);
                Some(self.session.count())
            }
            Ok(0) | Err(_) => None,
        }
    }

    /// Forgets the pointer and every contact, as the channel does when the
    /// connection is lost or it is told they have gone (section 8 rule 3).
    fn pointer_forgotten(&mut self) {
        self.pacer.clear();
        self.contacts.clear();
        self.finger = None;
        self.held = 0;
        self.last_at = [None; MAX_CONTACTS];
    }

    /// The connection opened: the session's first message, HELLO, with
    /// RESUME after it when the session may be resumed.
    fn hello(&mut self, channel: u32, now_us: u64, sends: &mut Vec<Send>) {
        let mut out = [0_u8; MAX_MESSAGE];
        if let Ok(length) = self.session.open(clock(now_us), None, &mut out) {
            sends.push(Send::Fetcher { channel, bytes: out.get(..length).unwrap_or(&[]).to_vec(), move_of: None, more: false });
        }
    }
}

/// Whether `item` is a pointer's or a contact's gone.
fn is_gone(item: &Item<'_>) -> bool {
    matches!(item, Item::Event(Event::Mouse(Mouse { action: MouseAction::Gone, .. }) | Event::Touch(Touch { action: MouseAction::Gone, .. })))
}

/// `text` in Unicode Normalization Form C, borrowed when it already is.
fn nfc(text: &str) -> Cow<'_, str> {
    use unicode_normalization::{IsNormalized, UnicodeNormalization, is_nfc_quick};
    if is_nfc_quick(text.chars()) == IsNormalized::Yes { Cow::Borrowed(text) } else { Cow::Owned(text.nfc().collect()) }
}

/// The session's clock: the core's milliseconds, wrapping.
fn clock(now_us: u64) -> u32 {
    let milliseconds = now_us.checked_div(MICROSECONDS_PER_MILLISECOND).unwrap_or(0);
    u32::try_from(milliseconds & u64::from(u32::MAX)).unwrap_or(0)
}

/// RFC 1982 serial arithmetic on 16 bits: whether `count` is `acked` or
/// before it.
fn covered(count: u16, acked: u16) -> bool {
    acked.wrapping_sub(count) < SERIAL_HALF
}

/// Half the range of the 16-bit input count (RFC 1982 section 3.2).
const SERIAL_HALF: u16 = 0x8000;

/// Marks every message to a channel's fetcher but its last in `sends` as
/// having more after it: the messages of one moment leave in one write
/// (REQ-RCH-010).
fn together(mut sends: Vec<Send>) -> Vec<Send> {
    let mut later: Vec<u32> = Vec::new();
    for send in sends.iter_mut().rev() {
        if let Send::Fetcher { channel, more, .. } = send {
            *more = later.contains(channel);
            if !*more {
                later.push(*channel);
            }
        }
    }
    sends
}

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
            .and_then(|cursor| cursor.place.map(|place| CursorState { row: place.row, column: place.column, flags: cursor.flags & BLOCK }));
        Some(TypingState {
            channel: opened.channel,
            page: opened.page,
            on,
            secure: state.is_some_and(|state| state.secure),
            refused: state.is_some_and(|state| state.open && state.session.state() == State::Refused),
            cursor: cursor.filter(|_| on),
            round_trip_us: state.and_then(|state| state.round_trip_us).filter(|_| on),
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
    /// channel lets go of anything held (REQ-INT-108). Before either, the
    /// waiting moves and then gone for the pointer and every contact, since
    /// the page they were over stopped taking keys (REQ-RCH-034); and the
    /// round trip is measured exactly while keys go (REQ-RCH-016).
    fn tell(&mut self, channel: u32, now_us: u64, sends: &mut Vec<Send>) {
        let showing = self.showing(channel);
        let page = if showing { self.opened.map_or(NO_PAGE, |opened| opened.page) } else { NO_PAGE };
        let focus = showing && !self.away();
        let Some(state) = self.channels.get_mut(&channel) else { return };
        if !state.started {
            return;
        }
        if state.focus_told != Some(focus) || state.told != Some(page) {
            state.let_go(channel, now_us, sends);
        }
        if state.focus_told != Some(focus) {
            sends.push(Send::Measure { channel, on: focus });
        }
        if state.focus_told != Some(focus) && !focus {
            state.focus_told = Some(focus);
            state.send(channel, &Item::Event(Event::FocusOut), now_us, sends);
        }
        if state.told != Some(page) {
            state.told = Some(page);
            state.page_unacked = state.send(channel, &Item::Event(Event::Page(page)), now_us, sends);
        }
        if state.focus_told != Some(focus) && focus {
            state.focus_told = Some(focus);
            state.send(channel, &Item::Event(Event::FocusIn), now_us, sends);
        }
    }

    /// Once welcomed, the session's start (section 6.1): RESIZE, the focus,
    /// the page.
    fn start(&mut self, channel: u32, now_us: u64, sends: &mut Vec<Send>) {
        let Some(state) = self.channels.get_mut(&channel) else { return };
        if state.started || !state.session.welcomed() {
            return;
        }
        state.started = true;
        let resize = Event::Resize { columns: PAGE_COLUMNS, rows: MAX_ROWS, width: 0, height: 0 };
        state.send(channel, &Item::Event(resize), now_us, sends);
        self.tell(channel, now_us, sends);
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
    pub fn opened(&mut self, active: Option<u32>, opened: Option<PageKey>, now_us: u64) -> Vec<Send> {
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
            self.tell(channel, now_us, &mut sends);
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
                round_trip_us: None,
                show: None,
            }));
        }
        self.tell_interface(&mut sends);
        together(sends)
    }

    /// A channel's input socket opened or closed. A connection begins with
    /// HELLO, and RESUME after it when the channel set RESUME and the
    /// session may be resumed (INPUT.md section 6.4, REQ-RCH-013); a lost
    /// connection keeps what a resumption needs (`close`).
    pub fn input(&mut self, channel: u32, open: bool, secure: bool, now_us: u64) -> Vec<Send> {
        let mut sends = Vec::new();
        let state = self.channels.entry(channel).or_insert_with(Channel::new);
        let was_open = state.open;
        state.open = open;
        state.secure = open && secure;
        // The channel takes every held button and contact as released when
        // the connection goes (section 6.4 item 4), resumed or not.
        state.pointer_forgotten();
        if open && !was_open {
            state.started = false;
            state.told = None;
            state.page_unacked = None;
            state.focus_told = None;
            state.cursor = None;
            state.round_trip_us = None;
            state.hello(channel, now_us, &mut sends);
        } else if !open {
            state.session.close();
            state.started = false;
            state.told = None;
            state.page_unacked = None;
            state.focus_told = None;
            state.round_trip_us = None;
        }
        self.tell_interface(&mut sends);
        sends
    }

    /// The window gained or lost the keyboard focus.
    pub fn focused(&mut self, focused: bool, now_us: u64) -> Vec<Send> {
        self.unfocused = !focused;
        self.keys_moved(now_us)
    }

    /// The viewer began or stopped editing the address, whose keys are
    /// the strip's, not the page's.
    pub fn editing(&mut self, editing: bool, now_us: u64) -> Vec<Send> {
        self.editing = editing;
        self.keys_moved(now_us)
    }

    fn keys_moved(&mut self, now_us: u64) -> Vec<Send> {
        let mut sends = Vec::new();
        if let Some(channel) = self.active {
            self.tell(channel, now_us, &mut sends);
        }
        together(sends)
    }

    /// An intent from the interface (or the phone, through it): encoded
    /// and sent on only under every condition of REQ-INT-130, under the
    /// session's rate, a move at the pointer's pace; otherwise dropped.
    pub fn input_intent(&mut self, input: &InputIntent, now_us: u64) -> Vec<Send> {
        let mut sends = Vec::new();
        let channel = input.key.channel;
        if !self.on(channel) || self.opened != Some(input.key) || self.away() {
            return sends;
        }
        let Some(state) = self.channels.get_mut(&channel) else { return sends };
        if matches!(input.intent, Intent::Mouse { .. } | Intent::Gone | Intent::Touch { .. }) {
            state.pointer(channel, &input.intent, now_us, &mut sends);
            return together(sends);
        }
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
            intent => match intent.item() {
                Some(item) => item,
                None => return sends,
            },
        };
        // The waiting moves go before any other event (REQ-RCH-033).
        state.flush(channel, now_us, &mut sends);
        state.send(channel, &item, now_us, &mut sends);
        together(sends)
    }

    /// The push connection's round trip, as the channel's fetcher measured
    /// it (REQ-RCH-016): shown in the strip while keys go to the channel.
    pub fn round_trip(&mut self, channel: u32, microseconds: u64) -> Vec<Send> {
        let mut sends = Vec::new();
        let Some(state) = self.channels.get_mut(&channel).filter(|state| state.open && state.focus_told == Some(true)) else { return sends };
        state.round_trip_us = Some(u32::try_from(microseconds).unwrap_or(u32::MAX));
        if self.on(channel) {
            self.tell_interface(&mut sends);
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
    pub fn tick(&mut self, now_us: u64) -> Vec<Send> {
        let mut sends = Vec::new();
        let mut refused = false;
        for state in self.channels.values_mut() {
            let waiting = matches!(state.session.state(), State::Waiting(_));
            state.session.tick(clock(now_us));
            refused |= waiting && state.session.state() == State::Refused;
        }
        if refused {
            self.tell_interface(&mut sends);
        }
        sends
    }

    /// A channel no tab shows any more: its session is forgotten whole, so
    /// another connection to it is a new session (REQ-INT-125).
    pub fn channel_gone(&mut self, channel: u32) {
        if let Some(mut state) = self.channels.remove(&channel) {
            state.session.forget();
        }
    }
}

mod pointer;
mod records;

#[cfg(test)]
mod tests;
