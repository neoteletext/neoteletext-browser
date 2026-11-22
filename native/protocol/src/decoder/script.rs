//! What a decoder and the trusted core say to each other about a
//! channel's scripts (the script API's specification sections 3.1, 4.6,
//! 11.2 and 12.5; DEC-7266, TODO-7293): the trusted core gives a decoder
//! the scripts its channel's files name and what only the core knows of
//! the channel, and the decoder reports what each script claims, what
//! became of each input event, and what a script asked of the browser.
//!
//! These are not the script link (`crate::script`), which the trusted
//! core relays between a decoder and its script host unread: every
//! message here is read whole by the side it is for.
//!
//! - **A script reaches a decoder as bytes and a name**, never as an
//!   address: the decoder reads its manifest, the one process that parses
//!   outside input (section 11.2).
//! - **A report is untrusted**, as everything a decoder says is. Every
//!   field has its bound, by name.

use neoteletext_core::consts::MAX_SCRIPT_NAME;

use crate::rules::PageSub;
use crate::script::{
    ContextId, FailReason, MAX_EVENT_PAYLOAD_BYTES, MAX_KEY_ASKS, MAX_KEY_NAME_BYTES, MAX_LABEL_BYTES, MAX_LOG_BYTES, MAX_SCRIPT_BYTES, MAX_SOURCE_PIECE_BYTES,
    MAX_TITLE_BYTES, MAX_ZONE_MINUTES, check_address, check_log, check_plain,
};
use crate::web::MAX_URL_BYTES;
use crate::{Fields, ProtocolError, push_bytes, push_count};

/// The highest level of the developer log a script's line has (the batch's
/// `MAX_LOG_LEVEL`).
pub const MAX_LOG_LEVEL: u8 = crate::script::batch::MAX_LOG_LEVEL;

// The forms of [`ScriptOrder`].
const ORDER_CHANNEL: u8 = 1;
const ORDER_SOURCE: u8 = 2;
const ORDER_NO_SCRIPT: u8 = 3;
const ORDER_SOUND_ALLOWED: u8 = 4;
const ORDER_TEXT: u8 = 5;
const ORDER_WHEEL: u8 = 6;
const ORDER_POINTER: u8 = 7;
const ORDER_NAVIGATING: u8 = 8;

// The forms of [`ScriptReport`].
const REPORT_CLAIMS: u8 = 1;
const REPORT_VERDICT: u8 = 2;
const REPORT_NAVIGATE: u8 = 3;
const REPORT_TITLE: u8 = 4;
const REPORT_LOG: u8 = 5;
const REPORT_STOPPED: u8 = 6;
const REPORT_LABEL: u8 = 7;
const REPORT_WANTED: u8 = 8;

/// What only the trusted core knows of a channel, which every context of
/// its scripts is told (section 3.3 and the fields of `CONTEXT_OPEN`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ChannelFacts {
    /// The channel's address: its folder, ending in `/`; empty for a file
    /// opened from disk.
    pub channel_url: String,
    /// The title the channel's file gives, at most [`MAX_TITLE_BYTES`].
    pub channel_title: String,
    /// The channel is a file opened from disk (section 12.7).
    pub from_disk: bool,
    /// Seconds since 1970 as this message is written: the decoder counts
    /// on from it by its own clock.
    pub epoch_seconds: u64,
    /// The viewer's zone, minutes east of UTC: a confined decoder cannot
    /// read it.
    pub zone_minutes: i16,
}

impl ChannelFacts {
    fn check(&self) -> Result<(), ProtocolError> {
        check_address(&self.channel_url, "channel address")?;
        check_plain(&self.channel_title, MAX_TITLE_BYTES, "channel title")?;
        if (-MAX_ZONE_MINUTES..=MAX_ZONE_MINUTES).contains(&self.zone_minutes) { Ok(()) } else { Err(ProtocolError::BadValue("zone")) }
    }
}

/// A piece of a script's bytes, as the fetcher or the broker read them
/// (section 12.5): untrusted, and passed on by the trusted core unread.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourcePiece {
    /// The level of the script: the channel's, or a page's.
    pub level: ContextId,
    /// The page and sub-code the script is for; for the channel's script,
    /// the page it came with, which the decoder does not read.
    pub page: PageSub,
    /// The script's name (section 12.2), at most the engine's
    /// `MAX_SCRIPT_NAME`: what its address in the channel's folder ends in.
    pub name: String,
    /// The length of the whole script, 1 to [`MAX_SCRIPT_BYTES`].
    pub total: u32,
    /// Where in the script these bytes begin. The pieces come in order
    /// from 0; the script is whole when `offset` and the bytes reach
    /// `total`.
    pub offset: u32,
    /// The bytes, 1 to [`MAX_SOURCE_PIECE_BYTES`].
    pub bytes: Vec<u8>,
}

impl SourcePiece {
    fn check(&self) -> Result<(), ProtocolError> {
        let total = usize::try_from(self.total).map_err(|_| ProtocolError::BadValue("script length"))?;
        let end = usize::try_from(self.offset).ok().and_then(|offset| offset.checked_add(self.bytes.len()));
        if total == 0 || total > MAX_SCRIPT_BYTES || self.bytes.is_empty() || self.bytes.len() > MAX_SOURCE_PIECE_BYTES || end.is_none_or(|end| end > total) {
            return Err(ProtocolError::BadValue("script piece"));
        }
        check_name(&self.name)
    }
}

/// A script name as the specification's section 12.2 bounds it. Its form
/// is the engine's to check (`ScriptName::parse`); here it is held to
/// plain ASCII of the engine's length, so that it can end an address.
fn check_name(name: &str) -> Result<(), ProtocolError> {
    if !name.is_empty() && name.len() <= MAX_SCRIPT_NAME && name.bytes().all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.')) {
        Ok(())
    } else {
        Err(ProtocolError::BadValue("script name"))
    }
}

/// From the trusted core to a channel's decoder, about its scripts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ScriptOrder {
    /// What the channel's contexts are told, before its first script.
    Channel(ChannelFacts),
    /// A piece of a script.
    Source(SourcePiece),
    /// The page and sub-code has no script of its own, or its script could
    /// not be loaded: one it was given before is forgotten.
    NoScript(PageSub),
    /// The viewer has pressed a key or the pointer in the tab: its
    /// scripts' sound may sound from now on (section 3.5; DEC-7282 item 7).
    SoundAllowed,
    /// Text the viewer typed, pasted or composed on the open page, for a
    /// script that claims text (section 4.2): the page rule table's input
    /// carries no text, since a table never reads it, but a script's
    /// `text` event does.
    Text {
        /// The trusted core's number for the event, which a verdict names.
        sequence: u32,
        /// The text, at most [`MAX_EVENT_PAYLOAD_BYTES`].
        text: String,
    },
    /// The wheel over the open page, for a script that claims it (section
    /// 4.2): with its amounts, which the rule table's input does not carry.
    Wheel {
        /// The trusted core's number for the event.
        sequence: u32,
        /// The pixel of the page's picture.
        x: u16,
        /// The pixel of the page's picture.
        y: u16,
        /// Lines across, right positive.
        delta_x: i16,
        /// Lines down, down positive.
        delta_y: i16,
    },
    /// The pointer on the open page, for a script that claims it (section
    /// 4.2): the mouse, or a contact of a touch screen or a pen, each with
    /// its number and kind.
    Pointer(ScriptPointer),
    /// The tab is about to go somewhere by something other than a script
    /// of the page: the contexts hear `navigate` first (section 4.2).
    Navigating {
        /// What caused it.
        cause: NavigationCause,
        /// Where: a page of the channel as `NNN` or `NNN/SSSS`, or an
        /// address, at most [`MAX_URL_BYTES`].
        target: String,
    },
}

/// The most pointers followed at once on a script's page (section 4.4,
/// the specification's `MAX_POINTERS`): the mouse and the contacts of a
/// touch screen together; a further contact is not reported.
pub const MAX_POINTERS: u8 = 4;

/// What kind of pointer an event is of (section 4.2's `pointerType`), by
/// the number the script library packs it as.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PointerKind {
    /// The mouse.
    Mouse,
    /// A contact of a touch screen.
    Touch,
    /// A pen.
    Pen,
}

impl PointerKind {
    /// The number of the kind in a pointer event's first number (the
    /// script library's packing, bits 8 and 9).
    #[must_use]
    pub const fn code(self) -> u8 {
        match self {
            Self::Mouse => 0,
            Self::Touch => 1,
            Self::Pen => 2,
        }
    }

    const fn of(code: u8) -> Result<Self, ProtocolError> {
        match code {
            0 => Ok(Self::Mouse),
            1 => Ok(Self::Touch),
            2 => Ok(Self::Pen),
            _ => Err(ProtocolError::BadValue("pointer kind")),
        }
    }
}

/// What caused a navigation the trusted core tells a channel's scripts of
/// (section 4.2's `cause`), by the script library's number for it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NavigationCause {
    /// A script's own `nt.navigate`, for the context that did not call it.
    Script,
    /// A link of the page.
    Link,
    /// The default action of a key the page claimed.
    Key,
    /// The browser's own controls: never cancelable.
    Browser,
}

impl NavigationCause {
    /// The script library's number for the cause (its `NavigateCause`).
    #[must_use]
    pub const fn code(self) -> u8 {
        match self {
            Self::Script => 1,
            Self::Link => 2,
            Self::Key => 3,
            Self::Browser => 4,
        }
    }

    const fn of(code: u8) -> Result<Self, ProtocolError> {
        match code {
            1 => Ok(Self::Script),
            2 => Ok(Self::Link),
            3 => Ok(Self::Key),
            4 => Ok(Self::Browser),
            _ => Err(ProtocolError::BadValue("navigation cause")),
        }
    }
}

/// A pointer event of the viewer for a script's page (section 4.2), with
/// what the rule table's input does not carry: which pointer and of what
/// kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ScriptPointer {
    /// The trusted core's number for the event, which a verdict names.
    pub sequence: u32,
    /// What the pointer did.
    pub action: crate::rules::PointerAction,
    /// On a press or release the button; on a move the buttons held.
    pub button: u8,
    /// The pixel of the page's picture.
    pub x: u16,
    /// The pixel of the page's picture.
    pub y: u16,
    /// Which pointer, 1 to [`MAX_POINTERS`]: the same number from its
    /// press to its release.
    pub pointer: u8,
    /// What kind it is.
    pub kind: PointerKind,
    /// It is the primary pointer of its kind: the mouse, or the first
    /// contact down.
    pub primary: bool,
}

impl ScriptPointer {
    fn check(&self) -> Result<(), ProtocolError> {
        if (1..=MAX_POINTERS).contains(&self.pointer) { Ok(()) } else { Err(ProtocolError::BadValue("pointer number")) }
    }
}

const fn pointer_action_code(action: crate::rules::PointerAction) -> u8 {
    use crate::rules::PointerAction;
    match action {
        PointerAction::Press => 0,
        PointerAction::Release => 1,
        PointerAction::Move => 2,
        PointerAction::Gone => 3,
    }
}

const fn pointer_action_of(code: u8) -> Result<crate::rules::PointerAction, ProtocolError> {
    use crate::rules::PointerAction;
    match code {
        0 => Ok(PointerAction::Press),
        1 => Ok(PointerAction::Release),
        2 => Ok(PointerAction::Move),
        3 => Ok(PointerAction::Gone),
        _ => Err(ProtocolError::BadValue("pointer action")),
    }
}

/// What a script's manifest claims of the pointer.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub enum PointerClaim {
    /// Nothing.
    #[default]
    None,
    /// Presses, releases, enters, leaves and cancels.
    Press,
    /// Those and moves.
    Move,
}

impl PointerClaim {
    const fn code(self) -> u8 {
        match self {
            Self::None => 0,
            Self::Press => 1,
            Self::Move => 2,
        }
    }

    const fn of(code: u8) -> Result<Self, ProtocolError> {
        match code {
            0 => Ok(Self::None),
            1 => Ok(Self::Press),
            2 => Ok(Self::Move),
            _ => Err(ProtocolError::BadValue("pointer claim")),
        }
    }
}

/// What the scripts of the open page claim, the channel's and the page's
/// together (section 4.6): told when a context is created or ends, and
/// unchanged while the contexts live.
#[derive(Clone, Debug, PartialEq, Eq)]
#[expect(clippy::struct_excessive_bools, reason = "four independent claims, each a flag of its own on the wire and named where it is read")]
pub struct ScriptClaims {
    /// The open page and sub-code the claims are for.
    pub page: PageSub,
    /// A script runs: false when the last context has ended, with every
    /// other field empty.
    pub running: bool,
    /// A context has the network (section 11.1): the trusted core relays
    /// a script host's requests only then.
    pub network: bool,
    /// Typed text is claimed.
    pub text: bool,
    /// What of the pointer is claimed.
    pub pointer: PointerClaim,
    /// The wheel is claimed.
    pub wheel: bool,
    /// The keys claimed, by the W3C UI Events key value, at most
    /// [`MAX_KEY_ASKS`] of at most [`MAX_KEY_NAME_BYTES`] each. There is
    /// no "any key".
    pub keys: Vec<String>,
}

impl ScriptClaims {
    /// No script runs on `page`: nothing is claimed.
    #[must_use]
    pub const fn none(page: PageSub) -> Self {
        Self { page, running: false, network: false, text: false, pointer: PointerClaim::None, wheel: false, keys: Vec::new() }
    }

    fn check(&self) -> Result<(), ProtocolError> {
        if self.keys.len() > MAX_KEY_ASKS {
            return Err(ProtocolError::BadValue("too many keys"));
        }
        self.keys
            .iter()
            .try_for_each(|key| if key.is_empty() { Err(ProtocolError::BadValue("key name")) } else { check_plain(key, MAX_KEY_NAME_BYTES, "key name") })
    }
}

/// From a channel's decoder to the trusted core, about its scripts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ScriptReport {
    /// What the open page's scripts claim.
    Claims(ScriptClaims),
    /// What became of an input event the trusted core gave (section 4.1):
    /// its default action is taken unless a handler cancelled it. An event
    /// the scripts never answered is reported as not cancelled.
    Verdict {
        /// The trusted core's number for the event.
        sequence: u32,
        /// A handler cancelled the default action.
        cancelled: bool,
        /// A listener heard the event.
        handled: bool,
    },
    /// A script asked the browser to go somewhere (section 3.3): a page
    /// of the channel, or an address, as the script gave it. The trusted
    /// core decides.
    Navigate {
        /// The page the script that asked is on.
        page: PageSub,
        /// Where, at most [`MAX_URL_BYTES`].
        target: String,
    },
    /// The page context named its page (section 3.3).
    Title {
        /// The page.
        page: PageSub,
        /// The title, at most [`MAX_TITLE_BYTES`].
        title: String,
    },
    /// A line of the developer log.
    Log {
        /// The level, at most [`MAX_LOG_LEVEL`].
        level: u8,
        /// The context that wrote it.
        context: ContextId,
        /// The line.
        text: String,
    },
    /// The decoder stopped the channel's scripts for good: their host
    /// passed a rate or a bound the decoder holds, or its message was
    /// refused (section 10.5). The trusted core ends the script host and
    /// starts none until the channel is reloaded.
    Stopped {
        /// Why.
        reason: FailReason,
        /// What to say in the developer log.
        text: String,
    },
    /// The label of a thing of the open page whose record carries none
    /// (the batch's section LABELS), for the screen reader.
    Label {
        /// The page.
        page: PageSub,
        /// The thing's handle.
        handle: i32,
        /// The label, at most [`MAX_LABEL_BYTES`]; empty when it has none
        /// any more.
        label: String,
    },
    /// A page file names a script the decoder does not hold, by a TTI
    /// `JS` line or a `SCRIPT_REF` record (section 12.5 step 2): the
    /// script's name and never an address. The trusted core has it
    /// fetched from the channel's own folder, or read from the folder of
    /// a file opened from disk.
    Wanted {
        /// The page and sub-code the script is for.
        page: PageSub,
        /// The script's name (section 12.2).
        name: String,
    },
}

/// Writes a piece of a script: the same fields wherever one travels, from
/// a fetcher to the trusted core and from the core to a decoder.
pub(crate) fn push_piece(payload: &mut Vec<u8>, piece: &SourcePiece) -> Result<(), ProtocolError> {
    piece.check()?;
    payload.push(piece.level.code());
    push_named(payload, piece.page, &piece.name)?;
    payload.extend_from_slice(&piece.total.to_be_bytes());
    payload.extend_from_slice(&piece.offset.to_be_bytes());
    push_bytes(payload, &piece.bytes)
}

/// Reads a piece of a script, within its bounds.
pub(crate) fn read_piece(fields: &mut Fields<'_>) -> Result<SourcePiece, ProtocolError> {
    let level = ContextId::of(fields.u8()?)?;
    let (page, name) = read_named(fields)?;
    let piece =
        SourcePiece { level, page, name, total: fields.u32()?, offset: fields.u32()?, bytes: fields.bytes(MAX_SOURCE_PIECE_BYTES, "script piece")?.to_vec() };
    piece.check()?;
    Ok(piece)
}

/// Writes a page and the name of a script for it.
pub(crate) fn push_named(payload: &mut Vec<u8>, page: PageSub, name: &str) -> Result<(), ProtocolError> {
    check_name(name)?;
    crate::rules::push_page_sub(payload, page)?;
    push_bytes(payload, name.as_bytes())
}

/// Reads a page and the name of a script for it.
pub(crate) fn read_named(fields: &mut Fields<'_>) -> Result<(PageSub, String), ProtocolError> {
    let (page, name) = (crate::rules::read_page_sub(fields)?, fields.string(MAX_SCRIPT_NAME, "script name")?);
    check_name(&name)?;
    Ok((page, name))
}

pub(super) fn push_order(payload: &mut Vec<u8>, order: &ScriptOrder) -> Result<(), ProtocolError> {
    match order {
        ScriptOrder::Channel(facts) => {
            facts.check()?;
            payload.push(ORDER_CHANNEL);
            push_bytes(payload, facts.channel_url.as_bytes())?;
            push_bytes(payload, facts.channel_title.as_bytes())?;
            payload.push(u8::from(facts.from_disk));
            payload.extend_from_slice(&facts.epoch_seconds.to_be_bytes());
            payload.extend_from_slice(&facts.zone_minutes.to_be_bytes());
        }
        ScriptOrder::Source(piece) => {
            payload.push(ORDER_SOURCE);
            push_piece(payload, piece)?;
        }
        ScriptOrder::NoScript(page) => {
            payload.push(ORDER_NO_SCRIPT);
            crate::rules::push_page_sub(payload, *page)?;
        }
        ScriptOrder::SoundAllowed => payload.push(ORDER_SOUND_ALLOWED),
        ScriptOrder::Text { sequence, text } => {
            if text.len() > MAX_EVENT_PAYLOAD_BYTES {
                return Err(ProtocolError::TooLong(text.len()));
            }
            payload.push(ORDER_TEXT);
            payload.extend_from_slice(&sequence.to_be_bytes());
            push_bytes(payload, text.as_bytes())?;
        }
        ScriptOrder::Wheel { sequence, x, y, delta_x, delta_y } => {
            payload.push(ORDER_WHEEL);
            payload.extend_from_slice(&sequence.to_be_bytes());
            for field in [x.to_be_bytes(), y.to_be_bytes(), delta_x.to_be_bytes(), delta_y.to_be_bytes()] {
                payload.extend_from_slice(&field);
            }
        }
        ScriptOrder::Pointer(pointer) => {
            pointer.check()?;
            payload.push(ORDER_POINTER);
            payload.extend_from_slice(&pointer.sequence.to_be_bytes());
            payload.extend_from_slice(&[pointer_action_code(pointer.action), pointer.button]);
            payload.extend_from_slice(&pointer.x.to_be_bytes());
            payload.extend_from_slice(&pointer.y.to_be_bytes());
            payload.extend_from_slice(&[pointer.pointer, pointer.kind.code(), u8::from(pointer.primary)]);
        }
        ScriptOrder::Navigating { cause, target } => {
            check_plain(target, MAX_URL_BYTES, "navigation target")?;
            payload.extend_from_slice(&[ORDER_NAVIGATING, cause.code()]);
            push_bytes(payload, target.as_bytes())?;
        }
    }
    Ok(())
}

pub(super) fn read_order(fields: &mut Fields<'_>) -> Result<ScriptOrder, ProtocolError> {
    Ok(match fields.u8()? {
        ORDER_CHANNEL => {
            let facts = ChannelFacts {
                channel_url: fields.string(MAX_URL_BYTES, "channel address")?,
                channel_title: fields.string(MAX_TITLE_BYTES, "channel title")?,
                from_disk: fields.flag("from disk")?,
                epoch_seconds: fields.u64()?,
                zone_minutes: fields.i16()?,
            };
            facts.check()?;
            ScriptOrder::Channel(facts)
        }
        ORDER_SOURCE => ScriptOrder::Source(read_piece(fields)?),
        ORDER_NO_SCRIPT => ScriptOrder::NoScript(crate::rules::read_page_sub(fields)?),
        ORDER_SOUND_ALLOWED => ScriptOrder::SoundAllowed,
        ORDER_TEXT => ScriptOrder::Text { sequence: fields.u32()?, text: fields.string(MAX_EVENT_PAYLOAD_BYTES, "text")? },
        ORDER_WHEEL => ScriptOrder::Wheel { sequence: fields.u32()?, x: fields.u16()?, y: fields.u16()?, delta_x: fields.i16()?, delta_y: fields.i16()? },
        ORDER_POINTER => {
            let pointer = ScriptPointer {
                sequence: fields.u32()?,
                action: pointer_action_of(fields.u8()?)?,
                button: fields.u8()?,
                x: fields.u16()?,
                y: fields.u16()?,
                pointer: fields.u8()?,
                kind: PointerKind::of(fields.u8()?)?,
                primary: fields.flag("primary pointer")?,
            };
            pointer.check()?;
            ScriptOrder::Pointer(pointer)
        }
        ORDER_NAVIGATING => {
            let (cause, target) = (NavigationCause::of(fields.u8()?)?, fields.string(MAX_URL_BYTES, "navigation target")?);
            check_plain(&target, MAX_URL_BYTES, "navigation target")?;
            ScriptOrder::Navigating { cause, target }
        }
        _ => return Err(ProtocolError::BadValue("script order")),
    })
}

fn check_report_log(level: u8, text: &str) -> Result<(), ProtocolError> {
    if level > MAX_LOG_LEVEL {
        return Err(ProtocolError::BadValue("log level"));
    }
    check_log(text)
}

pub(super) fn push_report(payload: &mut Vec<u8>, report: &ScriptReport) -> Result<(), ProtocolError> {
    match report {
        ScriptReport::Claims(claims) => {
            claims.check()?;
            payload.push(REPORT_CLAIMS);
            crate::rules::push_page_sub(payload, claims.page)?;
            payload.extend_from_slice(&[
                u8::from(claims.running),
                u8::from(claims.network),
                u8::from(claims.text),
                claims.pointer.code(),
                u8::from(claims.wheel),
            ]);
            push_count(payload, claims.keys.len(), MAX_KEY_ASKS, "too many keys")?;
            for key in &claims.keys {
                push_bytes(payload, key.as_bytes())?;
            }
        }
        ScriptReport::Verdict { sequence, cancelled, handled } => {
            payload.push(REPORT_VERDICT);
            payload.extend_from_slice(&sequence.to_be_bytes());
            payload.extend_from_slice(&[u8::from(*cancelled), u8::from(*handled)]);
        }
        ScriptReport::Navigate { page, target } => {
            check_plain(target, MAX_URL_BYTES, "navigation target")?;
            payload.push(REPORT_NAVIGATE);
            crate::rules::push_page_sub(payload, *page)?;
            push_bytes(payload, target.as_bytes())?;
        }
        ScriptReport::Title { page, title } => {
            check_plain(title, MAX_TITLE_BYTES, "title")?;
            payload.push(REPORT_TITLE);
            crate::rules::push_page_sub(payload, *page)?;
            push_bytes(payload, title.as_bytes())?;
        }
        ScriptReport::Log { level, context, text } => {
            check_report_log(*level, text)?;
            payload.extend_from_slice(&[REPORT_LOG, *level, context.code()]);
            push_bytes(payload, text.as_bytes())?;
        }
        ScriptReport::Stopped { reason, text } => {
            check_log(text)?;
            payload.extend_from_slice(&[REPORT_STOPPED, reason.code()]);
            push_bytes(payload, text.as_bytes())?;
        }
        ScriptReport::Wanted { page, name } => {
            payload.push(REPORT_WANTED);
            push_named(payload, *page, name)?;
        }
        ScriptReport::Label { page, handle, label } => {
            check_plain(label, MAX_LABEL_BYTES, "label")?;
            payload.push(REPORT_LABEL);
            crate::rules::push_page_sub(payload, *page)?;
            payload.extend_from_slice(&handle.to_be_bytes());
            push_bytes(payload, label.as_bytes())?;
        }
    }
    Ok(())
}

pub(super) fn read_report(fields: &mut Fields<'_>) -> Result<ScriptReport, ProtocolError> {
    Ok(match fields.u8()? {
        REPORT_CLAIMS => {
            let page = crate::rules::read_page_sub(fields)?;
            let (running, network, text) = (fields.flag("running")?, fields.flag("network")?, fields.flag("text")?);
            let (pointer, wheel) = (PointerClaim::of(fields.u8()?)?, fields.flag("wheel")?);
            let count = fields.count_u16(MAX_KEY_ASKS, "too many keys")?;
            let mut keys = Vec::with_capacity(count);
            for _ in 0..count {
                keys.push(fields.string(MAX_KEY_NAME_BYTES, "key name")?);
            }
            let claims = ScriptClaims { page, running, network, text, pointer, wheel, keys };
            claims.check()?;
            ScriptReport::Claims(claims)
        }
        REPORT_VERDICT => ScriptReport::Verdict { sequence: fields.u32()?, cancelled: fields.flag("cancelled")?, handled: fields.flag("handled")? },
        REPORT_NAVIGATE => {
            let (page, target) = (crate::rules::read_page_sub(fields)?, fields.string(MAX_URL_BYTES, "navigation target")?);
            check_plain(&target, MAX_URL_BYTES, "navigation target")?;
            ScriptReport::Navigate { page, target }
        }
        REPORT_TITLE => {
            let (page, title) = (crate::rules::read_page_sub(fields)?, fields.string(MAX_TITLE_BYTES, "title")?);
            check_plain(&title, MAX_TITLE_BYTES, "title")?;
            ScriptReport::Title { page, title }
        }
        REPORT_LOG => {
            let (level, context, text) = (fields.u8()?, ContextId::of(fields.u8()?)?, fields.string(MAX_LOG_BYTES, "log line")?);
            check_report_log(level, &text)?;
            ScriptReport::Log { level, context, text }
        }
        REPORT_STOPPED => {
            let (reason, text) = (FailReason::of(fields.u8()?)?, fields.string(MAX_LOG_BYTES, "log line")?);
            check_log(&text)?;
            ScriptReport::Stopped { reason, text }
        }
        REPORT_LABEL => {
            let (page, handle, label) = (crate::rules::read_page_sub(fields)?, fields.i32()?, fields.string(MAX_LABEL_BYTES, "label")?);
            check_plain(&label, MAX_LABEL_BYTES, "label")?;
            ScriptReport::Label { page, handle, label }
        }
        REPORT_WANTED => {
            let (page, name) = read_named(fields)?;
            ScriptReport::Wanted { page, name }
        }
        _ => return Err(ProtocolError::BadValue("script report")),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::decoder::{FromDecoder, ToDecoder, read_from_core, read_from_decoder, write_to_core, write_to_decoder};

    fn page() -> PageSub {
        PageSub::new(0x1A0, 2)
    }

    fn pointer(number: u8) -> ScriptPointer {
        use crate::rules::PointerAction;
        ScriptPointer { sequence: 11, action: PointerAction::Release, button: 0, x: 479, y: 499, pointer: number, kind: PointerKind::Touch, primary: true }
    }

    fn orders() -> Vec<ScriptOrder> {
        let piece = SourcePiece { level: ContextId::Page, page: page(), name: "tides-2.js".into(), total: 9, offset: 4, bytes: b"hello".to_vec() };
        vec![
            ScriptOrder::Channel(ChannelFacts {
                channel_url: "https://example.test/harbour/".into(),
                channel_title: "Harbour".into(),
                from_disk: false,
                epoch_seconds: 1_795_000_000,
                zone_minutes: -660,
            }),
            ScriptOrder::Source(piece),
            ScriptOrder::NoScript(page()),
            ScriptOrder::SoundAllowed,
            ScriptOrder::Text { sequence: 9, text: "été".into() },
            ScriptOrder::Wheel { sequence: 10, x: 479, y: 12, delta_x: -3, delta_y: 2 },
            ScriptOrder::Pointer(pointer(MAX_POINTERS)),
            ScriptOrder::Navigating { cause: NavigationCause::Browser, target: "100/0002".into() },
            ScriptOrder::Navigating { cause: NavigationCause::Link, target: "https://example.test/harbour/200.tti".into() },
        ]
    }

    fn reports() -> Vec<ScriptReport> {
        let claims = ScriptClaims {
            page: page(),
            running: true,
            network: true,
            text: true,
            pointer: PointerClaim::Move,
            wheel: true,
            keys: vec!["1".into(), "ArrowLeft".into()],
        };
        vec![
            ScriptReport::Claims(claims),
            ScriptReport::Claims(ScriptClaims::none(page())),
            ScriptReport::Verdict { sequence: 7, cancelled: true, handled: true },
            ScriptReport::Navigate { page: page(), target: "100/0002".into() },
            ScriptReport::Title { page: page(), title: "Tides".into() },
            ScriptReport::Log { level: 2, context: ContextId::Channel, text: "a line".into() },
            ScriptReport::Stopped { reason: FailReason::Link, text: "MAX_SCRIPT_OUTPUT_BYTES_PER_SECOND".into() },
            ScriptReport::Label { page: page(), handle: -5, label: "Forecast".into() },
            ScriptReport::Wanted { page: page(), name: "boat.js".into() },
        ]
    }

    /// Sections 11.2 and 12.5 (TODO-7293): every order and every report
    /// is read back as it was written.
    #[test]
    fn every_order_and_report_round_trips() {
        for order in orders() {
            let mut wire = Vec::new();
            write_to_decoder(&mut wire, &ToDecoder::Script(order.clone())).unwrap();
            assert_eq!(read_from_core(&mut wire.as_slice()).unwrap(), Some(ToDecoder::Script(order)));
        }
        for report in reports() {
            let mut wire = Vec::new();
            write_to_core(&mut wire, &FromDecoder::Script(report.clone())).unwrap();
            assert_eq!(read_from_decoder(&mut wire.as_slice()).unwrap(), Some(FromDecoder::Script(report)));
        }
    }

    /// Section 12.5 (TODO-7304): a script's pieces and its failure from a
    /// fetcher, and the request for a script by its name, are read back
    /// as written; a name that could leave the channel's folder is
    /// refused both ways.
    #[test]
    fn a_fetchers_script_messages_round_trip_and_a_name_is_never_a_path() {
        use crate::fetcher::{FromFetcher, ToFetcher, read_from_fetcher, read_to_fetcher, write_to_core as write_from_fetcher, write_to_fetcher};
        let piece = SourcePiece { level: ContextId::Channel, page: page(), name: "channel.js".into(), total: 5, offset: 0, bytes: b"hello".to_vec() };
        for message in [FromFetcher::Script(piece), FromFetcher::ScriptFailed { level: ContextId::Page, page: page(), name: "boat.js".into() }] {
            let mut wire = Vec::new();
            write_from_fetcher(&mut wire, &message).unwrap();
            assert_eq!(read_from_fetcher(&mut wire.as_slice()).unwrap(), Some(message));
        }
        let fetch = ToFetcher::FetchScript { page: page(), name: "boat.js".into() };
        let mut wire = Vec::new();
        write_to_fetcher(&mut wire, &fetch).unwrap();
        assert_eq!(read_to_fetcher(&mut wire.as_slice()).unwrap(), Some(fetch));
        for name in ["../boat.js", "a/boat.js", "https://example.test/boat.js", "", "boat.js?x"] {
            assert!(write_to_fetcher(&mut Vec::new(), &ToFetcher::FetchScript { page: page(), name: name.into() }).is_err(), "{name}");
            assert!(write_to_core(&mut Vec::new(), &FromDecoder::Script(ScriptReport::Wanted { page: page(), name: name.into() })).is_err(), "{name}");
        }
        // A reader refuses what a writer would not write.
        let at = wire.iter().rposition(|byte| *byte == b'.').unwrap();
        wire[at] = b'/';
        assert!(read_to_fetcher(&mut wire.as_slice()).is_err());
    }

    /// Section 11.2: a field past its bound is refused by the writer and
    /// by the reader, and bytes left over are refused.
    #[test]
    fn a_field_out_of_its_bounds_is_refused_both_ways() {
        let long_key = ScriptClaims { keys: vec!["k".repeat(MAX_KEY_NAME_BYTES + 1)], ..ScriptClaims::none(page()) };
        let many_keys = ScriptClaims { keys: vec!["k".into(); MAX_KEY_ASKS + 1], ..ScriptClaims::none(page()) };
        let refused = [
            ScriptReport::Claims(long_key),
            ScriptReport::Claims(many_keys),
            ScriptReport::Title { page: page(), title: "t".repeat(MAX_TITLE_BYTES + 1) },
            ScriptReport::Title { page: page(), title: "a\nb".into() },
            ScriptReport::Log { level: MAX_LOG_LEVEL + 1, context: ContextId::Page, text: String::new() },
            ScriptReport::Navigate { page: PageSub::new(0x1A0, 0x3F7F), target: String::new() },
        ];
        for report in refused {
            assert!(write_to_core(&mut Vec::new(), &FromDecoder::Script(report.clone())).is_err(), "{report:?}");
        }
        let source = |total: u32, offset: u32, bytes: usize, name: &str| {
            ScriptOrder::Source(SourcePiece { level: ContextId::Page, page: page(), name: name.into(), total, offset, bytes: vec![b' '; bytes] })
        };
        let past = u32::try_from(MAX_SCRIPT_BYTES).unwrap() + 1;
        // Section 4.4: a pointer has a number from 1 to `MAX_POINTERS`;
        // section 4.2: a navigation's target is plain text.
        let pointers = [ScriptOrder::Pointer(pointer(0)), ScriptOrder::Pointer(pointer(MAX_POINTERS + 1))];
        let navigating = ScriptOrder::Navigating { cause: NavigationCause::Browser, target: "1\u{7}00".into() };
        for order in pointers.into_iter().chain([navigating]) {
            assert!(write_to_decoder(&mut Vec::new(), &ToDecoder::Script(order.clone())).is_err(), "{order:?}");
        }
        for order in [source(0, 0, 1, "a.js"), source(past, 0, 1, "a.js"), source(4, 2, 3, "a.js"), source(4, 0, 0, "a.js"), source(4, 0, 4, "../a.js")] {
            assert!(write_to_decoder(&mut Vec::new(), &ToDecoder::Script(order.clone())).is_err(), "{order:?}");
        }
        // A reader holds the same bounds: a verdict with a flag that is no
        // flag, and a report with a byte left over.
        let mut wire = Vec::new();
        write_to_core(&mut wire, &FromDecoder::Script(ScriptReport::Verdict { sequence: 1, cancelled: false, handled: false })).unwrap();
        let last = wire.len() - 1;
        wire[last] = 2;
        assert!(read_from_decoder(&mut wire.as_slice()).is_err());
        let mut wire = Vec::new();
        write_to_decoder(&mut wire, &ToDecoder::Script(ScriptOrder::SoundAllowed)).unwrap();
        wire[4] += 1;
        wire.push(0);
        assert!(read_from_core(&mut wire.as_slice()).is_err());
    }
}
