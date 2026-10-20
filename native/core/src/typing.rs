//! Typing into a page (`interaction.md` sections 5 to 8, requirements 18.2
//! to 18.8): the trusted core's rules for when the viewer's keys go to a
//! channel, the records it sends, and the cursor it passes back.
//!
//! A key goes to a channel only while its tab is active, the page the
//! viewer opened is that channel's, and the channel's push WebSocket is
//! open with its descriptor declaring input. The interface asks; this
//! decides. Nothing a channel or a decoder sends can start it.

use std::collections::{BTreeMap, VecDeque};

use interface_core::canvas::PageKey;
use neoteletext_input::cursor::{self, Cursor};
use neoteletext_input::key::{self, Event, Limiter, MAX_RECORD, Press, Record};
use protocol::interaction::{CursorState, KeyIntent, Typing as TypingState};

/// The send times of this many unacknowledged records are kept for the
/// round trip (requirement 18.8), and never the keys.
pub const MAX_UNACKNOWLEDGED: usize = 256;

/// A message for a process.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Send {
    /// A key record for the fetcher of a channel.
    Fetcher(u32, Vec<u8>),
    /// The typing state for the interface.
    Interface(TypingState),
}

/// A record sent and not yet acknowledged.
#[derive(Clone, Copy, Debug)]
struct Sent {
    sequence: u16,
    /// The interface's number of the key, for a record of the viewer's.
    local: Option<u32>,
    at_ms: u64,
}

/// One channel's input connection.
#[derive(Debug)]
struct Channel {
    open: bool,
    secure: bool,
    /// The last sequence number used; 0 before the first.
    sequence: u16,
    /// A connection was made before: the next one starts with a resume.
    connected_before: bool,
    sent: VecDeque<Sent>,
    /// The cursor the channel placed, and for which page.
    cursor: Option<Cursor>,
    acked_local: u32,
    round_trip_ms: Option<u16>,
    limiter: Limiter,
    /// Keys down and not yet up, released when the keys leave the page.
    held: Vec<Press>,
    /// The page the channel was last told is open.
    told: Option<u16>,
    /// The page the channel's last cursor record named, when it named
    /// another than the one open: the host went there.
    went: Option<u16>,
}

impl Channel {
    fn new() -> Self {
        Self {
            open: false,
            secure: false,
            sequence: 0,
            connected_before: false,
            sent: VecDeque::new(),
            cursor: None,
            acked_local: 0,
            round_trip_ms: None,
            limiter: Limiter::new(),
            held: Vec::new(),
            told: None,
            went: None,
        }
    }

    /// The next sequence number: from 1, wrapping past 0.
    fn next_sequence(&mut self) -> u16 {
        self.sequence = match self.sequence.wrapping_add(1) {
            0 => 1,
            next => next,
        };
        self.sequence
    }
}

/// The clock a record carries: the core's milliseconds, wrapping.
fn clock(now_ms: u64) -> u32 {
    u32::try_from(now_ms & u64::from(u32::MAX)).unwrap_or(0)
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
    /// Whether the keys are away from the page (the engine's key record
    /// kind 6): the window lost the focus, or the address is being edited.
    fn away(&self) -> bool {
        self.unfocused || self.editing
    }

    /// Whether keys go to `channel` now: its tab active, its page open, its
    /// input socket open.
    fn on(&self, channel: u32) -> bool {
        self.active == Some(channel)
            && self.opened.is_some_and(|opened| opened.channel == channel)
            && self.channels.get(&channel).is_some_and(|state| state.open)
    }

    /// The state the interface draws: for the open page of the active
    /// channel, or nothing when no page is open.
    fn state(&self) -> Option<TypingState> {
        let opened = self.opened?;
        let state = self.channels.get(&opened.channel);
        let cursor = state
            .and_then(|state| state.cursor)
            .filter(|cursor| cursor.page == opened.page)
            .and_then(|cursor| cursor.place.map(|place| CursorState { row: place.row, column: place.column, width: cursor.width, flags: cursor.flags }));
        Some(TypingState {
            channel: opened.channel,
            page: opened.page,
            on: self.on(opened.channel),
            secure: state.is_some_and(|state| state.secure),
            cursor: cursor.filter(|_| self.on(opened.channel)),
            acked: state.map_or(0, |state| state.acked_local),
            round_trip_ms: state.and_then(|state| state.round_trip_ms),
            show: state.and_then(|state| state.went).filter(|&page| page != opened.page && self.on(opened.channel)),
        })
    }

    fn tell_interface(&self, sends: &mut Vec<Send>) {
        if let Some(state) = self.state() {
            sends.push(Send::Interface(state));
        }
    }

    /// Writes a record of the core's own (a page change, focus, a resume,
    /// a release) on `channel`.
    fn send_own(&mut self, channel: u32, event: Event<'_>, now_ms: u64, sends: &mut Vec<Send>) {
        let Some(state) = self.channels.get_mut(&channel) else { return };
        let sequence = state.next_sequence();
        let mut out = [0_u8; MAX_RECORD];
        let Some(length) = (Record { sequence, clock_ms: clock(now_ms), event }).write(&mut out) else { return };
        remember(state, Sent { sequence, local: None, at_ms: now_ms });
        sends.push(Send::Fetcher(channel, out.get(..length).unwrap_or(&[]).to_vec()));
    }

    /// Releases every key still down on `channel`, so the host never sees
    /// a key held after the keys have left (`interaction.md` section 6).
    fn release(&mut self, channel: u32, now_ms: u64, sends: &mut Vec<Send>) {
        let held = self.channels.get_mut(&channel).map(|state| core::mem::take(&mut state.held)).unwrap_or_default();
        for press in held {
            self.send_own(channel, Event::Up(Press { modifiers: press.modifiers & !key::REPEAT, ..press }), now_ms, sends);
        }
    }

    /// Tells `channel` which of its pages is open now, if that changed.
    fn tell_page(&mut self, channel: u32, now_ms: u64, sends: &mut Vec<Send>) {
        let page = if self.on(channel) { self.opened.map(|opened| opened.page) } else { None };
        let Some(state) = self.channels.get_mut(&channel) else { return };
        if !state.open || state.told == page {
            return;
        }
        state.told = page;
        self.send_own(channel, Event::Page(page), now_ms, sends);
    }

    /// The viewer opened `opened` in the active tab, whose channel is
    /// `active`, or switched tabs.
    pub fn opened(&mut self, active: Option<u32>, opened: Option<PageKey>, now_ms: u64) -> Vec<Send> {
        let mut sends = Vec::new();
        let was = self.active.filter(|&channel| self.on(channel));
        let opened = opened.filter(|key| Some(key.channel) == active);
        if (active, opened) == (self.active, self.opened) {
            return sends;
        }
        let page_before = self.opened;
        self.active = active;
        self.opened = opened;
        // The viewer chose: the host's earlier word on its page is spent.
        if let Some(state) = opened.and_then(|key| self.channels.get_mut(&key.channel)) {
            state.went = None;
        }
        if let Some(channel) = was
            && (Some(channel) != active || opened != page_before)
        {
            self.release(channel, now_ms, &mut sends);
            self.tell_page(channel, now_ms, &mut sends);
        }
        if let Some(channel) = active {
            self.tell_page(channel, now_ms, &mut sends);
        }
        if opened.is_none()
            && let Some(before) = page_before
        {
            // The interface stops typing into the page it had.
            sends.push(Send::Interface(TypingState {
                channel: before.channel,
                page: before.page,
                on: false,
                secure: false,
                cursor: None,
                acked: 0,
                round_trip_ms: None,
                show: None,
            }));
        }
        self.tell_interface(&mut sends);
        sends
    }

    /// A channel's input socket opened or closed. A new connection starts
    /// with a resume record when there was one before, then the open page.
    pub fn input(&mut self, channel: u32, open: bool, secure: bool, now_ms: u64) -> Vec<Send> {
        let mut sends = Vec::new();
        let state = self.channels.entry(channel).or_insert_with(Channel::new);
        let was_open = state.open;
        state.open = open;
        state.secure = open && secure;
        if open && !was_open {
            state.told = None;
            state.held.clear();
            state.cursor = None;
            state.sent.clear();
            if state.connected_before {
                let last = state.sequence;
                self.send_own(channel, Event::Resume(last), now_ms, &mut sends);
            }
            if let Some(state) = self.channels.get_mut(&channel) {
                state.connected_before = true;
            }
            self.tell_page(channel, now_ms, &mut sends);
            if self.on(channel) && self.away() {
                self.send_own(channel, Event::Focus(false), now_ms, &mut sends);
            }
        } else if !open && let Some(state) = self.channels.get_mut(&channel) {
            state.held.clear();
            state.told = None;
        }
        self.tell_interface(&mut sends);
        sends
    }

    /// The window gained or lost the keyboard focus.
    pub fn focused(&mut self, focused: bool, now_ms: u64) -> Vec<Send> {
        let was_away = self.away();
        self.unfocused = !focused;
        self.keys_moved(was_away, now_ms)
    }

    /// The viewer began or stopped editing the address, whose keys are
    /// the strip's, not the page's.
    pub fn editing(&mut self, editing: bool, now_ms: u64) -> Vec<Send> {
        let was_away = self.away();
        self.editing = editing;
        self.keys_moved(was_away, now_ms)
    }

    /// When the keys left the page or came back to it, the channel being
    /// typed into is told, and keys still down are released.
    fn keys_moved(&mut self, was_away: bool, now_ms: u64) -> Vec<Send> {
        let mut sends = Vec::new();
        let away = self.away();
        if away == was_away {
            return sends;
        }
        if let Some(channel) = self.active.filter(|&channel| self.on(channel)) {
            if away {
                self.release(channel, now_ms, &mut sends);
            }
            self.send_own(channel, Event::Focus(!away), now_ms, &mut sends);
        }
        sends
    }

    /// A key from the interface (or the phone, through it): sent on, with
    /// the core's sequence number, only under every condition of
    /// requirement 18.2; otherwise dropped.
    pub fn key(&mut self, intent: &KeyIntent, now_ms: u64) -> Vec<Send> {
        let mut sends = Vec::new();
        let channel = intent.key.channel;
        if !self.on(channel) || self.opened != Some(intent.key) || self.away() {
            return sends;
        }
        let Ok(record) = key::read(&intent.record) else { return sends };
        let Some(state) = self.channels.get_mut(&channel) else { return sends };
        let key_up = matches!(record.event, Event::Up(_));
        if !state.limiter.allow(clock(now_ms), key_up) {
            return sends;
        }
        match record.event {
            Event::Down(press) => {
                if !state.held.iter().any(|held| held.key == press.key && held.physical == press.physical) {
                    state.held.push(press);
                }
            }
            Event::Up(press) => state.held.retain(|held| !(held.key == press.key && held.physical == press.physical)),
            Event::Text(_) | Event::Click { .. } => {}
            // The interface cannot send these; the protocol refused them.
            Event::Page(_) | Event::Focus(_) | Event::Resume(_) => return sends,
        }
        let sequence = state.next_sequence();
        let mut out = [0_u8; MAX_RECORD];
        let Some(length) = (Record { sequence, ..record }).write(&mut out) else { return sends };
        remember(state, Sent { sequence, local: Some(intent.local), at_ms: now_ms });
        sends.push(Send::Fetcher(channel, out.get(..length).unwrap_or(&[]).to_vec()));
        sends
    }

    /// A cursor record a channel's decoder found: checked here by the
    /// engine's rules (requirement 18.6), kept for its page, and its
    /// acknowledgement closes the round trip.
    pub fn cursor(&mut self, channel: u32, body: [u8; cursor::BODY], now_ms: u64) -> Vec<Send> {
        let mut sends = Vec::new();
        let Ok(cursor) = cursor::from_body(body) else { return sends };
        if !self.channels.get(&channel).is_some_and(|state| state.open) {
            return sends;
        }
        let Some(state) = self.channels.get_mut(&channel) else { return sends };
        if cursor.acked != 0 {
            acknowledge(state, cursor.acked, now_ms);
        }
        // A videotex host goes to a page by sending it: the browser shows
        // that page of the same channel, as a terminal shows what its host
        // sends (`interaction.md` section 5.1).
        state.went = Some(cursor.page);
        state.cursor = Some(cursor);
        if self.on(channel) {
            self.tell_interface(&mut sends);
        }
        sends
    }

    /// A channel no tab shows any more.
    pub fn channel_gone(&mut self, channel: u32) {
        self.channels.remove(&channel);
    }
}

fn remember(state: &mut Channel, sent: Sent) {
    state.sent.push_back(sent);
    if state.sent.len() > MAX_UNACKNOWLEDGED {
        state.sent.pop_front();
    }
}

/// RFC 1982 serial arithmetic over 16 bits: `candidate` is `than` or
/// less than half the range after it.
fn at_or_after(candidate: u16, than: u16) -> bool {
    candidate.wrapping_sub(than) < SERIAL_HALF
}

/// Half the range of a 16-bit sequence number (RFC 1982 section 3.2).
const SERIAL_HALF: u16 = 0x8000;

/// The channel acted on records up to `acked`: those are forgotten, and
/// the last of them gives the round trip and the echo to clear.
fn acknowledge(state: &mut Channel, acked: u16, now_ms: u64) {
    let mut last = None;
    while let Some(&sent) = state.sent.front() {
        if !at_or_after(acked, sent.sequence) {
            break;
        }
        state.sent.pop_front();
        last = Some(sent);
        if let Some(local) = sent.local {
            state.acked_local = state.acked_local.max(local);
        }
    }
    if let Some(sent) = last {
        state.round_trip_ms = Some(u16::try_from(now_ms.saturating_sub(sent.at_ms)).unwrap_or(u16::MAX));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PAGE: PageKey = PageKey { channel: 7, page: 0x200 };

    fn down(character: u8, local: u32) -> KeyIntent {
        let mut out = [0; MAX_RECORD];
        let event = Event::Down(Press { key: u16::from(character), modifiers: 0, physical: 0 });
        let length = Record { sequence: 0, clock_ms: 9, event }.write(&mut out).unwrap();
        KeyIntent { key: PAGE, local, record: out[..length].to_vec() }
    }

    fn records(sends: &[Send]) -> Vec<Record<'_>> {
        sends.iter().filter_map(|send| if let Send::Fetcher(_, bytes) = send { key::read(bytes).ok() } else { None }).collect()
    }

    fn ready() -> Typing {
        let mut typing = Typing::default();
        typing.input(PAGE.channel, true, false, 0);
        typing.opened(Some(PAGE.channel), Some(PAGE), 0);
        typing
    }

    #[test]
    fn keys_go_only_under_every_condition() {
        let mut typing = Typing::default();
        assert!(records(&typing.key(&down(b'a', 1), 0)).is_empty(), "no socket, no page");
        typing.input(PAGE.channel, true, false, 0);
        assert!(records(&typing.key(&down(b'a', 1), 0)).is_empty(), "no page open");
        typing.opened(Some(PAGE.channel), Some(PageKey { page: 0x201, ..PAGE }), 0);
        assert!(records(&typing.key(&down(b'a', 1), 0)).is_empty(), "another page open");
        typing.opened(Some(8), None, 0);
        assert!(records(&typing.key(&down(b'a', 1), 0)).is_empty(), "another tab");
        let sends = typing.opened(Some(PAGE.channel), Some(PAGE), 0);
        assert_eq!(records(&sends).iter().map(|record| record.event).collect::<Vec<_>>(), [Event::Page(Some(0x200))]);
        let sent = typing.key(&down(b'a', 1), 0);
        assert!(matches!(records(&sent)[0].event, Event::Down(press) if press.key == u16::from(b'a')));
        typing.focused(false, 0);
        assert!(records(&typing.key(&down(b'b', 2), 0)).is_empty(), "the window has no focus");
        typing.input(PAGE.channel, false, false, 0);
        assert!(records(&typing.key(&down(b'c', 3), 0)).is_empty(), "the socket closed");
    }

    #[test]
    fn records_are_numbered_and_held_keys_are_released_on_leaving() {
        let mut typing = ready();
        let first = typing.key(&down(b'a', 1), 0);
        assert_eq!(records(&first)[0].sequence, 2, "the page record was 1");
        let leaving = typing.opened(Some(PAGE.channel), None, 5);
        let events: Vec<Event> = records(&leaving).iter().map(|record| record.event).collect();
        assert_eq!(events, [Event::Up(Press { key: u16::from(b'a'), modifiers: 0, physical: 0 }), Event::Page(None)]);
        assert!(leaving.iter().any(|send| matches!(send, Send::Interface(state) if !state.on)));
    }

    #[test]
    fn editing_the_address_takes_the_keys_from_the_page_and_says_so() {
        let mut typing = ready();
        typing.key(&down(b'a', 1), 0);
        let editing = typing.editing(true, 1);
        let events: Vec<Event> = records(&editing).iter().map(|record| record.event).collect();
        assert_eq!(events, [Event::Up(Press { key: u16::from(b'a'), modifiers: 0, physical: 0 }), Event::Focus(false)]);
        assert!(records(&typing.key(&down(b'b', 2), 2)).is_empty(), "the keys are the strip's");
        // The window losing the focus while editing changes nothing for the
        // page, nor does it coming back.
        assert_eq!(typing.focused(false, 3), []);
        assert_eq!(typing.focused(true, 4), []);
        let done = typing.editing(false, 5);
        assert_eq!(records(&done).iter().map(|record| record.event).collect::<Vec<_>>(), [Event::Focus(true)]);
        assert_eq!(records(&typing.key(&down(b'c', 3), 6)).len(), 1);
    }

    #[test]
    fn a_new_connection_resumes_with_the_last_number_and_the_open_page() {
        let mut typing = ready();
        typing.key(&down(b'a', 1), 0);
        typing.input(PAGE.channel, false, false, 1);
        let again = typing.input(PAGE.channel, true, true, 2);
        let events: Vec<(u16, Event)> = records(&again).iter().map(|record| (record.sequence, record.event)).collect();
        assert_eq!(events, [(3, Event::Resume(2)), (4, Event::Page(Some(0x200)))]);
    }

    #[test]
    fn the_cursor_is_checked_and_its_acknowledgement_measures_the_round_trip() {
        let mut typing = ready();
        typing.key(&down(b'a', 11), 100);
        typing.key(&down(b'b', 12), 110);
        let body = |row: u8, acked: u8| [0x02, 0x00, 0, acked, row, 4, 10, cursor::ECHO];
        assert!(typing.cursor(PAGE.channel, body(24, 2), 150).is_empty(), "row 24 is refused");
        let sends = typing.cursor(PAGE.channel, body(5, 2), 150);
        let Some(Send::Interface(state)) = sends.last() else { panic!("{sends:?}") };
        assert_eq!(state.cursor, Some(CursorState { row: 5, column: 4, width: 10, flags: cursor::ECHO }));
        assert_eq!((state.acked, state.round_trip_ms), (11, Some(50)));
        assert_eq!(state.show, None, "the open page's own record");
        let elsewhere = |acked: u8| [0x02, 0x01, 0, acked, 255, 0, 0, 0];
        let sends = typing.cursor(PAGE.channel, elsewhere(2), 155);
        assert!(matches!(sends.last(), Some(Send::Interface(state)) if state.show == Some(0x201)), "the host went to 201");
        let sends = typing.opened(Some(PAGE.channel), Some(PageKey { page: 0x201, ..PAGE }), 156);
        assert!(matches!(sends.last(), Some(Send::Interface(state)) if state.show.is_none()), "and the browser followed");
        typing.opened(Some(PAGE.channel), Some(PAGE), 157);
        typing.cursor(PAGE.channel, body(5, 2), 158);
        // An acknowledgement out of order changes nothing.
        let sends = typing.cursor(PAGE.channel, body(5, 1), 160);
        assert!(matches!(sends.last(), Some(Send::Interface(state)) if state.acked == 11));
    }

    #[test]
    fn the_rate_is_limited_but_never_a_key_up() {
        let mut typing = ready();
        let mut sent = 0;
        for n in 0..200_u32 {
            sent += records(&typing.key(&down(b'a' + (n % 26) as u8, n), 1000)).len();
        }
        assert_eq!(sent, usize::try_from(key::MAX_PER_SECOND).unwrap(), "the core's own records are not counted");
        let mut out = [0; MAX_RECORD];
        let up = Event::Up(Press { key: u16::from(b'a'), modifiers: 0, physical: 0 });
        let length = Record { sequence: 0, clock_ms: 0, event: up }.write(&mut out).unwrap();
        assert_eq!(records(&typing.key(&KeyIntent { key: PAGE, local: 999, record: out[..length].to_vec() }, 1000)).len(), 1);
    }
}
