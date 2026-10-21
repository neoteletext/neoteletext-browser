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
use neoteletext_input::control::{Control, read_body};
use neoteletext_input::cursor::{self, Cursor};
use neoteletext_input::event::{self, ALT, Action, CONTROL, Event, HYPER, Item, META, SUPER};
use neoteletext_input::modes::{ALL_KEYS, ALL_MODES, CURSOR_CAP, ECHO_CAP, PAGE_CAP};
use neoteletext_input::session::{Session, State};
use neoteletext_input::{MAX_MESSAGE, MAX_ROWS};
use protocol::interaction::{CursorState, InputIntent, Intent, Typing as TypingState};

/// The interface's numbers of this many unacknowledged intents are kept
/// for echo, and never what they were (REQ-INT-137).
pub const MAX_UNACKNOWLEDGED: usize = neoteletext_input::session::REMEMBERED;
/// What NeoTeletext offers in HELLO (INPUT.md section 6.2): it shows pages,
/// draws the cursor and echo, and can send every mode.
const OFFERED: u32 = PAGE_CAP | CURSOR_CAP | ECHO_CAP | ALL_MODES;
/// The modifiers that make a key a command rather than text (section 5.3).
const COMMANDS: u8 = CONTROL | ALT | SUPER | HYPER | META;
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

    /// Encodes `item` under the session's modes on `open` and gives it to
    /// the channel's fetcher; the input count after it, if it went.
    fn send(&mut self, channel: u32, item: &Item<'_>, open: Option<u16>, now_ms: u64, sends: &mut Vec<Send>) -> Option<u16> {
        let mut out = [0_u8; MAX_MESSAGE];
        match self.session.send(item, open, clock(now_ms), &mut out) {
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
            modes: state.filter(|_| on).map_or(0, |state| state.session.modes_on(Some(opened.page))),
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
            state.send(channel, &Item::Event(Event::FocusOut), None, now_ms, sends);
        }
        if state.told != Some(page) {
            state.told = Some(page);
            state.send(channel, &Item::Event(Event::Page(page)), Some(page), now_ms, sends);
        }
        if state.focus_told != Some(focus) && focus {
            state.focus_told = Some(focus);
            state.send(channel, &Item::Event(Event::FocusIn), None, now_ms, sends);
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
        state.send(channel, &Item::Event(resize), None, now_ms, sends);
        self.tell(channel, now_ms, sends);
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
                modes: 0,
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
    /// session's modes and rate; otherwise dropped.
    pub fn input_intent(&mut self, input: &InputIntent, now_ms: u64) -> Vec<Send> {
        let mut sends = Vec::new();
        let channel = input.key.channel;
        if !self.on(channel) || self.opened != Some(input.key) || self.away() {
            return sends;
        }
        let Some(state) = self.channels.get_mut(&channel) else { return sends };
        let open = Some(input.key.page);
        let all_keys = state.session.modes_on(open) & ALL_KEYS != 0;
        // Text goes in Normalization Form C (section 5.7 rule 3,
        // REQ-INT-116), a KEY's text too; a paste goes as it was copied.
        let normalized = match &input.intent {
            Intent::Text(text) | Intent::Key { text: Some(text), .. } => nfc(text),
            _ => Cow::Borrowed(""),
        };
        let item = match &input.intent {
            // A key that types text, with no command modifier, is the text
            // (section 5.3), unless the channel asked for every key.
            Intent::Key { action: Action::Press | Action::Repeat, text: Some(text), mods, .. }
                if !all_keys && mods & COMMANDS == 0 && text.chars().all(event::printable) =>
            {
                Item::Text(&normalized)
            }
            Intent::Text(_) => Item::Text(&normalized),
            Intent::Key { action, key, mods, text: Some(_), base } => {
                Item::Event(Event::Key(event::Key { action: *action, key: *key, mods: *mods, text: Some(&normalized), base: *base }))
            }
            intent => intent.item(),
        };
        if let Some(count) = state.send(channel, &item, open, now_ms, &mut sends) {
            state.sent.push_back((count, input.local));
            if state.sent.len() > MAX_UNACKNOWLEDGED {
                state.sent.pop_front();
            }
        }
        sends
    }

    /// A cursor record a channel's decoder found: checked here by the
    /// engine's rules (REQ-INT-134), kept for its page; its
    /// acknowledgement clears echo and gives the round trip (REQ-INT-110).
    pub fn cursor(&mut self, channel: u32, body: [u8; cursor::BODY], now_ms: u64) -> Vec<Send> {
        let mut sends = Vec::new();
        let Ok(cursor) = cursor::from_body(body) else { return sends };
        let Some(state) = self.channels.get_mut(&channel).filter(|state| state.open && state.session.welcomed()) else {
            return sends;
        };
        if let Some(round_trip) = state.session.acknowledged(cursor.acked, clock(now_ms)) {
            state.round_trip_ms = Some(u16::try_from(round_trip).unwrap_or(u16::MAX));
        }
        let acked = state.session.acked();
        while let Some(&(count, local)) = state.sent.front() {
            if !covered(count, acked) {
                break;
            }
            state.sent.pop_front();
            state.acked_local = state.acked_local.max(local);
        }
        // A host goes to a page by sending it (REQ-INT-113): the browser
        // shows that page of the same channel.
        state.went = Some(cursor.page);
        state.cursor = Some(cursor);
        if self.on(channel) {
            self.tell_interface(&mut sends);
        }
        sends
    }

    /// A control record a channel's decoder found, or a page's `IM` line:
    /// read here (REQ-INT-111), and applied to the session: WELCOME starts
    /// it, the rest change its modes (REQ-INT-107, REQ-INT-114).
    pub fn control(&mut self, channel: u32, body: &[u8], now_ms: u64) -> Vec<Send> {
        let mut sends = Vec::new();
        let Ok(message) = read_body(body) else { return sends };
        let Some(state) = self.channels.get_mut(&channel).filter(|state| state.open) else { return sends };
        // A WELCOME that does not fit leaves the session refused, which the
        // strip says.
        let _ = state.session.control(&message);
        if matches!(message, Control::Welcome { .. }) {
            self.start(channel, now_ms, &mut sends);
        }
        if self.opened.is_some_and(|opened| opened.channel == channel) {
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

#[cfg(test)]
mod tests {
    use super::*;
    use neoteletext_input::control::PAGE_MODES;
    use neoteletext_input::event::{KeyCode, MouseAction, Point, read_message};
    use neoteletext_input::modes::{KEY_RELEASE, MOUSE_BUTTONS};
    use neoteletext_input::{VERSION, WELCOME_TIMEOUT_MS};

    const PAGE: PageKey = PageKey { channel: 7, page: 0x200 };

    fn key(character: char, local: u32) -> InputIntent {
        let intent = Intent::Key { action: Action::Press, key: KeyCode::Character(character), mods: 0, text: Some(character.to_string()), base: None };
        InputIntent { key: PAGE, local, intent }
    }

    /// Every item the sends gave the fetcher, as the channel reads them.
    fn items(sends: &[Send]) -> Vec<String> {
        let mut out = Vec::new();
        for send in sends {
            if let Send::Fetcher(_, bytes) = send {
                read_message(bytes, &mut |item| out.push(format!("{item:?}"))).unwrap();
            }
        }
        out
    }

    fn welcome(caps: u32) -> Vec<u8> {
        let mut body = [0_u8; neoteletext_input::control::MAX_BODY];
        let length = neoteletext_input::control::write_body(&Control::Welcome { version: VERSION, caps }, &mut body).unwrap();
        body[..length].to_vec()
    }

    fn ready(caps: u32) -> Typing {
        let mut typing = Typing::default();
        typing.input(PAGE.channel, true, false, 0);
        typing.opened(Some(PAGE.channel), Some(PAGE), 0);
        typing.control(PAGE.channel, &welcome(caps), 0);
        typing
    }

    /// REQ-INT-106: HELLO first, nothing before WELCOME, then RESIZE, the
    /// focus and the page.
    #[test]
    fn a_session_starts_with_hello_and_waits_for_welcome() {
        let mut typing = Typing::default();
        let opened = typing.input(PAGE.channel, true, true, 0);
        assert_eq!(items(&opened), [format!("Event(Hello {{ version: 2, caps: {OFFERED}, agent: None }})")]);
        typing.opened(Some(PAGE.channel), Some(PAGE), 0);
        assert_eq!(items(&typing.input_intent(&key('a', 1), 0)), [] as [String; 0], "nothing before WELCOME");
        let welcomed = typing.control(PAGE.channel, &welcome(PAGE_CAP | CURSOR_CAP), 5);
        assert_eq!(
            items(&welcomed),
            ["Event(Resize { columns: 40, rows: 25, width: 0, height: 0 })".to_owned(), "Event(Page(512))".to_owned(), "Event(FocusIn)".to_owned(),]
        );
        assert!(welcomed.iter().any(|send| matches!(send, Send::Interface(state) if state.on && state.secure)));
        assert_eq!(items(&typing.input_intent(&key('a', 1), 6)), ["Text(\"a\")"]);
    }

    #[test]
    fn a_channel_that_does_not_answer_is_refused_and_the_strip_says_so() {
        let mut typing = Typing::default();
        typing.input(PAGE.channel, true, false, 0);
        typing.opened(Some(PAGE.channel), Some(PAGE), 0);
        assert_eq!(typing.tick(u64::from(WELCOME_TIMEOUT_MS) - 1), []);
        let sends = typing.tick(u64::from(WELCOME_TIMEOUT_MS));
        assert!(matches!(sends.last(), Some(Send::Interface(state)) if state.refused && !state.on));
        assert_eq!(items(&typing.control(PAGE.channel, &welcome(PAGE_CAP), 10_001)), [] as [String; 0], "too late");
    }

    /// REQ-INT-130: input goes only under every condition.
    #[test]
    fn input_goes_only_under_every_condition() {
        let mut typing = Typing::default();
        assert_eq!(items(&typing.input_intent(&key('a', 1), 0)), [] as [String; 0], "no socket, no page");
        typing.input(PAGE.channel, true, false, 0);
        typing.control(PAGE.channel, &welcome(PAGE_CAP), 0);
        assert_eq!(items(&typing.input_intent(&key('a', 1), 0)), [] as [String; 0], "no page open");
        typing.opened(Some(PAGE.channel), Some(PageKey { page: 0x201, ..PAGE }), 0);
        assert_eq!(items(&typing.input_intent(&key('a', 1), 0)), [] as [String; 0], "another page open");
        typing.opened(Some(8), None, 0);
        assert_eq!(items(&typing.input_intent(&key('a', 1), 0)), [] as [String; 0], "another tab");
        typing.opened(Some(PAGE.channel), Some(PAGE), 0);
        assert_eq!(items(&typing.input_intent(&key('a', 1), 0)), ["Text(\"a\")"]);
        let away = typing.focused(false, 0);
        assert_eq!(items(&away), ["Event(FocusOut)"]);
        assert_eq!(items(&typing.input_intent(&key('b', 2), 0)), [] as [String; 0], "the window has no focus");
        assert_eq!(items(&typing.focused(true, 0)), ["Event(FocusIn)"]);
        typing.input(PAGE.channel, false, false, 0);
        assert_eq!(items(&typing.input_intent(&key('c', 3), 0)), [] as [String; 0], "the socket closed");
    }

    /// Section 9.1: leaving the page tells the channel the focus went and
    /// no page is open, so it lets go of what is held (REQ-INT-108).
    #[test]
    fn leaving_the_page_says_focus_out_and_page_zero() {
        let mut typing = ready(PAGE_CAP);
        let leaving = typing.opened(Some(PAGE.channel), None, 5);
        assert_eq!(items(&leaving), ["Event(FocusOut)", "Event(Page(0))"]);
        assert!(leaving.iter().any(|send| matches!(send, Send::Interface(state) if !state.on)));
        let back = typing.opened(Some(PAGE.channel), Some(PAGE), 6);
        assert_eq!(items(&back), ["Event(Page(512))", "Event(FocusIn)"]);
    }

    #[test]
    fn editing_the_address_takes_the_keys_from_the_page_and_says_so() {
        let mut typing = ready(PAGE_CAP);
        assert_eq!(items(&typing.editing(true, 1)), ["Event(FocusOut)"]);
        assert_eq!(items(&typing.input_intent(&key('b', 2), 2)), [] as [String; 0], "the keys are the strip's");
        // The window losing the focus while editing changes nothing for the
        // page, nor does it coming back.
        assert_eq!(items(&typing.focused(false, 3)), [] as [String; 0]);
        assert_eq!(items(&typing.focused(true, 4)), [] as [String; 0]);
        assert_eq!(items(&typing.editing(false, 5)), ["Event(FocusIn)"]);
        assert_eq!(items(&typing.input_intent(&key('c', 3), 6)).len(), 1);
    }

    /// REQ-INT-106: a reconnection is a new session.
    #[test]
    fn a_new_connection_is_a_new_session() {
        let mut typing = ready(PAGE_CAP | MOUSE_BUTTONS);
        typing.input(PAGE.channel, false, false, 1);
        let again = typing.input(PAGE.channel, true, true, 2);
        assert_eq!(items(&again).len(), 1);
        assert!(items(&again)[0].starts_with("Event(Hello"));
        assert_eq!(items(&typing.input_intent(&key('a', 1), 3)), [] as [String; 0], "waiting for WELCOME again");
    }

    /// Section 5.3: a key that types text is text; with a command
    /// modifier, or in `ALL_KEYS`, it is a KEY.
    #[test]
    fn text_goes_as_text_and_commands_as_keys() {
        let mut typing = ready(PAGE_CAP | ALL_KEYS);
        let control_c = Intent::Key { action: Action::Press, key: KeyCode::Character('c'), mods: CONTROL, text: Some("c".into()), base: None };
        let sends = typing.input_intent(&InputIntent { key: PAGE, local: 1, intent: control_c }, 0);
        assert_eq!(items(&sends), ["Event(Key(Key { action: Press, key: Character('c'), mods: 4, text: None, base: None }))"]);
        let release = Intent::Key { action: Action::Release, key: KeyCode::Character('c'), mods: 0, text: None, base: None };
        assert_eq!(items(&typing.input_intent(&InputIntent { key: PAGE, local: 2, intent: release }, 0)), [] as [String; 0], "no KEY_RELEASE");
        typing.control(PAGE.channel, &[0xFF, 0x02, u8::try_from(ALL_KEYS).unwrap()], 0);
        assert_eq!(
            items(&typing.input_intent(&key('a', 3), 0)),
            ["Event(Key(Key { action: Press, key: Character('a'), mods: 0, text: Some(\"a\"), base: None }))"],
            "every key a KEY in ALL_KEYS"
        );
    }

    /// REQ-INT-116 rule 3: a decomposed `e` and combining accent go as
    /// U+00E9, as text and as a KEY's text.
    #[test]
    fn text_goes_in_normalization_form_c() {
        let mut typing = ready(PAGE_CAP | ALL_KEYS);
        let text = InputIntent { key: PAGE, local: 1, intent: Intent::Text("e\u{301}".into()) };
        assert_eq!(items(&typing.input_intent(&text, 0)), ["Text(\"\u{e9}\")"]);
        typing.control(PAGE.channel, &[0xFF, 0x02, u8::try_from(ALL_KEYS).unwrap()], 0);
        let key = Intent::Key { action: Action::Press, key: KeyCode::Character('e'), mods: 0, text: Some("e\u{301}".into()), base: None };
        let sent = items(&typing.input_intent(&InputIntent { key: PAGE, local: 2, intent: key }, 0));
        assert_eq!(sent, ["Event(Key(Key { action: Press, key: Character('e'), mods: 0, text: Some(\"\u{e9}\"), base: None }))"]);
        let paste = InputIntent { key: PAGE, local: 3, intent: Intent::Paste("e\u{301}".into()) };
        assert_eq!(items(&typing.input_intent(&paste, 0)), [r#"Event(Paste("e\u{301}"))"#], "a paste as copied");
    }

    /// REQ-INT-115: the mouse goes only in a mouse mode, and the page's own
    /// modes come with the page (REQ-INT-114).
    #[test]
    fn the_mouse_goes_only_when_the_page_or_channel_asks() {
        let mut typing = ready(PAGE_CAP | MOUSE_BUTTONS | KEY_RELEASE);
        let click = InputIntent {
            key: PAGE,
            local: 1,
            intent: Intent::Mouse { action: MouseAction::Press, button: 0, at: Point { column: 10, row: 5, pixels: None, mods: 0 } },
        };
        assert_eq!(items(&typing.input_intent(&click, 0)), [] as [String; 0]);
        let page_modes = [0xFF, PAGE_MODES, 0x80, 0x04, u8::try_from(MOUSE_BUTTONS).unwrap()];
        let told = typing.control(PAGE.channel, &page_modes, 0);
        assert!(matches!(told.last(), Some(Send::Interface(state)) if state.modes == MOUSE_BUTTONS), "the interface knows");
        assert_eq!(
            items(&typing.input_intent(&click, 0)),
            ["Event(Mouse(Mouse { action: Press, button: 0, at: Point { column: 10, row: 5, pixels: None, mods: 0 } }))"]
        );
    }

    /// REQ-INT-110: the acknowledgement is the input count; it clears
    /// echo by the interface's numbers and gives the round trip
    /// (REQ-INT-137).
    #[test]
    fn the_acknowledgement_clears_echo_and_measures_the_round_trip() {
        let mut typing = ready(PAGE_CAP);
        // HELLO, RESIZE, PAGE and FOCUS_IN were 4.
        typing.input_intent(&key('a', 11), 100);
        typing.input_intent(&key('b', 12), 110);
        let body = |row: u8, acked: u8| [0x02, 0x00, 0, acked, row, 4, 10, cursor::ECHO];
        assert_eq!(typing.cursor(PAGE.channel, body(24, 5), 150), [], "row 24 is refused");
        let sends = typing.cursor(PAGE.channel, body(5, 5), 150);
        let Some(Send::Interface(state)) = sends.last() else { panic!("{sends:?}") };
        assert_eq!(state.cursor, Some(CursorState { row: 5, column: 4, width: 10, flags: cursor::ECHO }));
        assert_eq!((state.acked, state.round_trip_ms), (11, Some(50)));
        assert_eq!(state.show, None, "the open page's own record");
        let elsewhere = |acked: u8| [0x02, 0x01, 0, acked, 255, 0, 0, 0];
        let sends = typing.cursor(PAGE.channel, elsewhere(6), 155);
        assert!(matches!(sends.last(), Some(Send::Interface(state)) if state.show == Some(0x201) && state.acked == 12), "the host went to 201");
        let sends = typing.opened(Some(PAGE.channel), Some(PageKey { page: 0x201, ..PAGE }), 156);
        assert!(matches!(sends.last(), Some(Send::Interface(state)) if state.show.is_none()), "and the browser followed");
    }

    /// REQ-INT-137: no code that handles the viewer's input prints or logs
    /// anything, so no key can reach a log. The files are the trusted
    /// core's typing, the interface's keys and intents, and the protocol's
    /// intents; the keyboard process prints only why it stopped.
    #[test]
    fn no_input_code_logs() {
        let sources = [
            ("core/src/typing.rs", include_str!("typing.rs")),
            ("interface-core/src/browser/terminal_ui.rs", include_str!("../../interface-core/src/browser/terminal_ui.rs")),
            ("interface-core/src/input.rs", include_str!("../../interface-core/src/input.rs")),
            ("interface/src/keys.rs", include_str!("../../interface/src/keys.rs")),
            ("protocol/src/interaction.rs", include_str!("../../protocol/src/interaction.rs")),
        ];
        // The keyboard process prints one thing: why it stopped, which is
        // the relay's or the pipe's error, never a key.
        let keyboard = include_str!("../../keyboard/src/keyboard.rs");
        assert_eq!(keyboard.matches(concat!("eprint", "ln!")).count(), 1);
        assert!(keyboard.contains(concat!("eprint", "ln!(\"neoteletext-keyboard: {error}\")")));
        // Spelled in pieces, so this test's own text does not match.
        let calls =
            [concat!("print", "ln!"), concat!("eprint", "ln!"), concat!("print", "!("), concat!("eprint", "!("), concat!("dbg", "!("), concat!("log", "::")];
        for (path, source) in sources {
            for call in calls {
                assert!(!source.contains(call), "{path} has {call}");
            }
        }
    }

    /// REQ-INT-109: 120 units a second.
    #[test]
    fn the_rate_is_limited() {
        let mut typing = ready(PAGE_CAP);
        let mut sent = 0;
        for n in 0..200_u32 {
            sent += items(&typing.input_intent(&key(char::from(b'a' + u8::try_from(n % 26).unwrap()), n), 0)).len();
        }
        assert_eq!(sent, usize::try_from(neoteletext_input::MAX_PER_SECOND).unwrap() - 3, "RESIZE, PAGE and FOCUS_IN counted");
    }
}
