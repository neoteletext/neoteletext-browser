//! The phone keyboard (`interaction.md` section 9, requirement 18.9): the
//! messages between the trusted core and the `neoteletext-keyboard` process, which
//! alone holds the pairing secret, and the panel and keys the trusted core
//! passes on to the interface. Everything the keyboard process sends is
//! untrusted and bounded here.

use std::io::{Read, Write};

use super::{Fields, ProtocolError, push_text, read_frame, write_frame};
use crate::text::is_clean;

/// The largest QR code: version 40 is 177 modules a side (ISO/IEC 18004
/// Table 1).
pub const MAX_QR_SIZE: usize = 177;
/// The join address: qrkeyboard's is about 100 bytes.
pub const MAX_JOIN: usize = 512;
/// A key's W3C name or one character: the longest name in the engine's
/// table is 20 bytes.
pub const MAX_KEY_NAME: usize = 32;
/// Text the phone typed at once, within qrkeyboard's frame of 1 024 bytes
/// less its envelope; it goes on as one text intent, within the input
/// system's 4 000 (REQ-INT-138).
pub const MAX_PHONE_TEXT: usize = 995;
/// A PIN: four digits.
pub const MAX_PIN: usize = 8;

/// The bytes of a QR's modules at one bit each: a side of at most
/// [`MAX_QR_SIZE`], so the square never overflows.
fn bytes_of(size: usize) -> usize {
    size.saturating_mul(size).div_ceil(8)
}

// The interface pipe's kinds of the phone keyboard (`interface/kinds.rs`
// lists every kind of that pipe): strip input, then the trusted core's.
pub(crate) const KIND_PHONE_PANEL: u8 = 28;
pub(crate) const KIND_PHONE_FINISH: u8 = 29;
pub(crate) const KIND_PHONE: u8 = 41;
pub(crate) const KIND_PHONE_KEY: u8 = 42;

const KIND_JOIN: u8 = 112;
const KIND_STATE: u8 = 113;
const KIND_KEY: u8 = 114;
const KIND_TEXT: u8 = 115;
const KIND_FINISH: u8 = 120;

/// Where the phone keyboard stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PhoneState {
    /// Preparing: asking the relay for an instance.
    Preparing,
    /// The QR is shown; no phone yet.
    Scan,
    /// A phone is the keyboard.
    Connected,
    /// The phone went away and may come back.
    Waiting,
    /// A PIN was wrong; this many tries remain.
    PinFailed(u8),
    /// Too many wrong PINs.
    Locked,
    /// The session is over, or could not start.
    Finished,
}

impl PhoneState {
    fn code(self) -> (u8, u8) {
        match self {
            Self::Preparing => (0, 0),
            Self::Scan => (1, 0),
            Self::Connected => (2, 0),
            Self::Waiting => (3, 0),
            Self::PinFailed(left) => (4, left),
            Self::Locked => (5, 0),
            Self::Finished => (6, 0),
        }
    }

    fn from_code(code: u8, left: u8) -> Option<Self> {
        Some(match code {
            0 => Self::Preparing,
            1 => Self::Scan,
            2 => Self::Connected,
            3 => Self::Waiting,
            4 => Self::PinFailed(left),
            5 => Self::Locked,
            6 => Self::Finished,
            _ => return None,
        })
    }
}

/// A QR code's modules.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Qr {
    /// Modules a side.
    pub size: usize,
    /// Row by row, one bit a module, dark set, the first module the high
    /// bit of the first byte.
    pub bits: Vec<u8>,
}

impl Qr {
    /// Whether a module is dark.
    #[must_use]
    pub fn dark(&self, x: usize, y: usize) -> bool {
        let index = y.saturating_mul(self.size).saturating_add(x);
        x < self.size && y < self.size && self.bits.get(index / 8).is_some_and(|byte| byte & (0x80 >> (index % 8)) != 0)
    }
}

/// From the keyboard process to the trusted core: untrusted.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FromKeyboard {
    /// The instance is ready: its QR, and the PIN in PIN mode.
    Join {
        /// The QR's modules.
        qr: Qr,
        /// The PIN to show beside it.
        pin: Option<String>,
    },
    /// The keyboard's state changed.
    State(PhoneState),
    /// A key of the phone's layout went down or up, by its W3C name or as
    /// one character. A phone key carries no modifier.
    Key {
        /// Down, not up.
        down: bool,
        /// Its name.
        name: String,
    },
    /// Text the phone typed.
    Text(String),
}

/// From the trusted core to the keyboard process.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ToKeyboard {
    /// End the session (the strip's Finish): the QR stops working.
    Finish,
}

/// The phone keys that move between pages, by their W3C
/// `KeyboardEvent.key` names: the digits, the four arrows, and the four
/// colour keys of the phone's layout (qrkeyboard's terminal layout, which
/// the keyboard process asks for). While no page is open for typing the
/// trusted core passes on only these, and the interface gives them to the
/// page view alone (decision 99, finding F-20), so a compromised keyboard
/// process can choose pages and do nothing else to the browser.
pub const PAGE_NAVIGATION_KEYS: [&str; 18] = [
    "0",
    "1",
    "2",
    "3",
    "4",
    "5",
    "6",
    "7",
    "8",
    "9",
    "ArrowLeft",
    "ArrowRight",
    "ArrowUp",
    "ArrowDown",
    "ColorF0Red",
    "ColorF1Green",
    "ColorF2Yellow",
    "ColorF3Blue",
];

/// Whether a phone key moves between pages: one of
/// [`PAGE_NAVIGATION_KEYS`].
#[must_use]
pub fn navigates_pages(name: &str) -> bool {
    PAGE_NAVIGATION_KEYS.contains(&name)
}

/// A key name is refused where the interface's pipe would refuse it: a
/// control or any other hidden character ([`is_clean`]), so a hostile
/// keyboard process ends only its own pipe, never the trusted core's write
/// to the interface (finding F-28).
fn clean_name(name: &str) -> Result<(), ProtocolError> {
    let ok = !name.is_empty() && is_clean(name, MAX_KEY_NAME);
    if ok { Ok(()) } else { Err(ProtocolError::BadValue("phone key")) }
}

/// # Errors
/// When writing fails or the message breaks a bound.
pub fn write_to_core(out: &mut dyn Write, message: &FromKeyboard) -> Result<(), ProtocolError> {
    let mut payload = Vec::new();
    let kind = match message {
        FromKeyboard::Join { qr, pin } => {
            if qr.size == 0 || qr.size > MAX_QR_SIZE || qr.bits.len() != bytes_of(qr.size) {
                return Err(ProtocolError::BadValue("QR"));
            }
            payload.push(u8::try_from(qr.size).map_err(|_| ProtocolError::BadValue("QR"))?);
            payload.extend_from_slice(&qr.bits);
            if let Some(pin) = pin {
                push_text(&mut payload, pin, MAX_PIN)?;
            }
            KIND_JOIN
        }
        FromKeyboard::State(state) => {
            let (code, left) = state.code();
            payload.extend_from_slice(&[code, left]);
            KIND_STATE
        }
        FromKeyboard::Key { down, name } => {
            clean_name(name)?;
            payload.push(u8::from(*down));
            payload.extend_from_slice(name.as_bytes());
            KIND_KEY
        }
        FromKeyboard::Text(text) => {
            if text.len() > MAX_PHONE_TEXT || text.chars().any(char::is_control) {
                return Err(ProtocolError::BadValue("phone text"));
            }
            payload.extend_from_slice(text.as_bytes());
            KIND_TEXT
        }
    };
    write_frame(out, kind, &payload)
}

/// Reads one message from the keyboard process, or `None` at a clean end.
///
/// # Errors
/// On input errors, a kind it may not send, and anything out of bounds:
/// a QR of the wrong size, a key name that is long or has controls, text
/// over [`MAX_PHONE_TEXT`].
pub fn read_from_keyboard(input: &mut dyn Read) -> Result<Option<FromKeyboard>, ProtocolError> {
    let Some((kind, payload)) = read_frame(input)? else { return Ok(None) };
    let mut fields = Fields::new(&payload);
    let message = match kind {
        KIND_JOIN => {
            let size = usize::from(fields.u8()?);
            if size == 0 || size > MAX_QR_SIZE {
                return Err(ProtocolError::BadValue("QR"));
            }
            let bits = fields.take(bytes_of(size))?.to_vec();
            let pin = if fields.is_empty() { None } else { Some(fields.text(MAX_PIN)?) };
            if pin.as_ref().is_some_and(|pin| !pin.chars().all(|character| character.is_ascii_digit())) {
                return Err(ProtocolError::BadValue("PIN"));
            }
            FromKeyboard::Join { qr: Qr { size, bits }, pin }
        }
        KIND_STATE => {
            let (code, left) = (fields.u8()?, fields.u8()?);
            FromKeyboard::State(PhoneState::from_code(code, left).ok_or(ProtocolError::BadValue("phone state"))?)
        }
        KIND_KEY => {
            let down = fields.flag("phone key")?;
            let name = std::str::from_utf8(fields.take(fields.remaining())?).map_err(|_| ProtocolError::BadValue("phone key"))?.to_owned();
            clean_name(&name)?;
            FromKeyboard::Key { down, name }
        }
        KIND_TEXT => {
            let text = std::str::from_utf8(fields.take(fields.remaining())?).map_err(|_| ProtocolError::BadValue("phone text"))?;
            if text.len() > MAX_PHONE_TEXT || text.chars().any(char::is_control) {
                return Err(ProtocolError::BadValue("phone text"));
            }
            FromKeyboard::Text(text.to_owned())
        }
        other => return Err(ProtocolError::UnknownKind(other)),
    };
    fields.end()?;
    Ok(Some(message))
}

/// # Errors
/// When writing fails.
pub fn write_to_keyboard(out: &mut dyn Write, message: &ToKeyboard) -> Result<(), ProtocolError> {
    let ToKeyboard::Finish = message;
    write_frame(out, KIND_FINISH, &[])
}

/// Reads one message from the trusted core, or `None` at a clean end.
///
/// # Errors
/// On input errors and any other kind.
pub fn read_to_keyboard(input: &mut dyn Read) -> Result<Option<ToKeyboard>, ProtocolError> {
    let Some((kind, payload)) = read_frame(input)? else { return Ok(None) };
    if kind != KIND_FINISH || !payload.is_empty() {
        return Err(ProtocolError::UnknownKind(kind));
    }
    Ok(Some(ToKeyboard::Finish))
}

/// What the interface shows of the phone keyboard: the panel's QR and
/// PIN while one waits to be scanned, and the strip's state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PhoneView {
    /// Where it stands.
    pub state: PhoneState,
    /// The QR, while it may be scanned.
    pub qr: Option<Qr>,
    /// The PIN, in PIN mode.
    pub pin: Option<String>,
}

const VIEW_QR: u8 = 1;
const VIEW_PIN: u8 = 2;

pub(crate) fn push_view(payload: &mut Vec<u8>, view: &PhoneView) -> Result<(), ProtocolError> {
    let (code, left) = view.state.code();
    let flags = if view.qr.is_some() { VIEW_QR } else { 0 } | if view.pin.is_some() { VIEW_PIN } else { 0 };
    payload.extend_from_slice(&[code, left, flags]);
    if let Some(qr) = &view.qr {
        if qr.size == 0 || qr.size > MAX_QR_SIZE || qr.bits.len() != bytes_of(qr.size) {
            return Err(ProtocolError::BadValue("QR"));
        }
        payload.push(u8::try_from(qr.size).map_err(|_| ProtocolError::BadValue("QR"))?);
        payload.extend_from_slice(&qr.bits);
    }
    if let Some(pin) = &view.pin {
        push_text(payload, pin, MAX_PIN)?;
    }
    Ok(())
}

pub(crate) fn read_view(fields: &mut Fields<'_>) -> Result<PhoneView, ProtocolError> {
    let (code, left, flags) = (fields.u8()?, fields.u8()?, fields.u8()?);
    if flags & !(VIEW_QR | VIEW_PIN) != 0 {
        return Err(ProtocolError::BadValue("phone flags"));
    }
    let state = PhoneState::from_code(code, left).ok_or(ProtocolError::BadValue("phone state"))?;
    let qr = if flags & VIEW_QR != 0 {
        let size = usize::from(fields.u8()?);
        if size == 0 || size > MAX_QR_SIZE {
            return Err(ProtocolError::BadValue("QR"));
        }
        Some(Qr { size, bits: fields.take(bytes_of(size))?.to_vec() })
    } else {
        None
    };
    let pin = if flags & VIEW_PIN != 0 { Some(fields.text(MAX_PIN)?) } else { None };
    Ok(PhoneView { state, qr, pin })
}

// A phone key's tag on the interface's pipe: a key of the layout up or
// down, or text.
const PHONE_KEY_UP: u8 = 0;
const PHONE_KEY_DOWN: u8 = 1;
const PHONE_TEXT: u8 = 2;

/// A phone's key for the interface, which acts on it as on a key of its
/// own keyboard, with no modifier.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PhoneKey {
    /// A key of the layout, by its W3C name or as one character.
    Key {
        /// Down, not up.
        down: bool,
        /// Its name.
        name: String,
    },
    /// Text typed.
    Text(String),
}

pub(crate) fn push_phone_key(payload: &mut Vec<u8>, key: &PhoneKey) -> Result<(), ProtocolError> {
    match key {
        PhoneKey::Key { down, name } => {
            clean_name(name)?;
            payload.push(if *down { PHONE_KEY_DOWN } else { PHONE_KEY_UP });
            push_text(payload, name, MAX_KEY_NAME)
        }
        PhoneKey::Text(text) => {
            payload.push(PHONE_TEXT);
            push_text(payload, text, MAX_PHONE_TEXT)
        }
    }
}

pub(crate) fn read_phone_key(fields: &mut Fields<'_>) -> Result<PhoneKey, ProtocolError> {
    Ok(match fields.u8()? {
        kind @ (PHONE_KEY_UP | PHONE_KEY_DOWN) => {
            let name = fields.text(MAX_KEY_NAME)?;
            clean_name(&name)?;
            PhoneKey::Key { down: kind == PHONE_KEY_DOWN, name }
        }
        PHONE_TEXT => PhoneKey::Text(fields.text(MAX_PHONE_TEXT)?),
        _ => return Err(ProtocolError::BadValue("phone key")),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn qr() -> Qr {
        Qr { size: 21, bits: vec![0xA5; (21 * 21_usize).div_ceil(8)] }
    }

    #[test]
    fn every_message_round_trips() {
        for message in [
            FromKeyboard::Join { qr: qr(), pin: Some("1234".into()) },
            FromKeyboard::Join { qr: qr(), pin: None },
            FromKeyboard::State(PhoneState::PinFailed(3)),
            FromKeyboard::Key { down: true, name: "ColorF0Red".into() },
            FromKeyboard::Text("hé".into()),
        ] {
            let mut bytes = Vec::new();
            write_to_core(&mut bytes, &message).unwrap();
            assert_eq!(read_from_keyboard(&mut bytes.as_slice()).unwrap().unwrap(), message);
        }
        let mut bytes = Vec::new();
        write_to_keyboard(&mut bytes, &ToKeyboard::Finish).unwrap();
        assert_eq!(read_to_keyboard(&mut bytes.as_slice()).unwrap(), Some(ToKeyboard::Finish));
    }

    #[test]
    fn what_a_keyboard_may_not_send_is_refused() {
        let frame = |kind: u8, payload: &[u8]| {
            let mut bytes = vec![kind];
            bytes.extend_from_slice(&u32::try_from(payload.len()).unwrap().to_be_bytes());
            bytes.extend_from_slice(payload);
            read_from_keyboard(&mut bytes.as_slice())
        };
        assert!(frame(KIND_KEY, b"\x01Esc\x1b").is_err(), "a control in a name");
        assert!(frame(KIND_KEY, &[1; 40]).is_err(), "a long name");
        // Finding F-28: a name the interface's pipe would refuse is refused
        // here, so only the keyboard's pipe ends, never the browser.
        for hidden in ["a\u{200B}", "\u{202E}Enter", "\u{FEFF}", "1\u{2066}"] {
            let mut payload = vec![1];
            payload.extend_from_slice(hidden.as_bytes());
            assert!(frame(KIND_KEY, &payload).is_err(), "{hidden:?}");
            let key = PhoneKey::Key { down: true, name: hidden.into() };
            assert!(push_phone_key(&mut Vec::new(), &key).is_err(), "{hidden:?}");
        }
        assert!(frame(KIND_TEXT, &[b'a'; MAX_PHONE_TEXT + 1]).is_err());
        assert!(frame(KIND_JOIN, &[200]).is_err(), "a QR too big");
        assert!(frame(KIND_STATE, &[9, 0]).is_err());
        assert!(frame(KIND_FINISH, &[]).is_err(), "only the core sends Finish");
        assert!(matches!(frame(81, &[0]), Err(ProtocolError::UnknownKind(81))), "a fetcher's kind");
    }

    /// Decision 99: the keys that move between pages are names a phone
    /// may send, and Enter, Tab, Escape, Backspace and letters are not
    /// among them.
    #[test]
    fn the_page_navigation_keys_are_digits_arrows_and_colours() {
        for name in PAGE_NAVIGATION_KEYS {
            assert!(clean_name(name).is_ok() && navigates_pages(name), "{name}");
        }
        for name in ["Enter", "Tab", "Escape", "Backspace", "Delete", "a", "Z", "ChannelUp", "BrowserBack", "Exit", "", "11"] {
            assert!(!navigates_pages(name), "{name}");
        }
    }

    #[test]
    fn a_qr_reads_its_modules() {
        let mut bits = vec![0_u8; 1];
        bits[0] = 0b1000_0001;
        let small = Qr { size: 2, bits };
        assert!(small.dark(0, 0) && !small.dark(1, 0) && !small.dark(0, 1) && !small.dark(5, 5));
    }
}
