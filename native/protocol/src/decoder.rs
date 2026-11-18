//! Messages between the trusted core and a decoder process: the same
//! framing, bounds and strictness as the interface's messages, with kinds
//! of their own. The trusted core sends a channel's bytes as they are
//! fetched, never parsing them; the decoder sends back which pages have
//! arrived and the rendered pages it is asked for (design section 6).

use std::io::{Read, Write};

use interface_core::channel::PAGES;

use crate::frame::PAGE_DOTS;
use crate::subpages::{SubPageRange, SubPageRows, SubPageStep};
use crate::{Fields, MAX_PAGE_TITLE, ProtocolError, push_count, push_raw_text, raw_text, read_frame, write_frame};
use interface_core::controls::SUBPAGE_ROWS;
use kinds::{
    KIND_ARRIVED, KIND_ASK_SUBPAGE_ROWS, KIND_AUDIO, KIND_AUDIO_RECEIVER, KIND_BOOKMARK_FOLDER, KIND_BOOKMARK_FOLDER_END, KIND_BOOKMARK_LINK,
    KIND_BOOKMARKS_READ, KIND_CHANNEL_LOG, KIND_CLOSE, KIND_CONFINED, KIND_DATA, KIND_END, KIND_FRAME, KIND_MISSING, KIND_OPENED, KIND_PICTURE,
    KIND_PICTURE_CONVERTED, KIND_PICTURE_OUTPUT, KIND_PICTURE_REFUSED, KIND_REGION, KIND_REGIS, KIND_SHOWING, KIND_STORE_FULL, KIND_SUBPAGE_COUNT,
    KIND_SUBPAGE_ROWS, KIND_SUBPAGES, KIND_UPDATED, KIND_VIEW, KIND_WHOLE,
};
#[cfg(feature = "interaction")]
use kinds::{KIND_CURSOR, KIND_INPUT_CONTROL};
#[cfg(feature = "links")]
use kinds::{KIND_LINKS, KIND_LOOK};
use neoteletext_core::consts::NIBBLE_MASK;

mod bookmarks;
mod frame;
mod kinds;
#[cfg(test)]
mod tests;
mod view;

pub use bookmarks::{BookmarkEntry, MAX_BOOKMARK_ENTRIES, MAX_BOOKMARK_FILE, MAX_BOOKMARK_NAME, NETSCAPE_DOCTYPE, is_netscape_bookmarks};
// Every page is rendered in the engine's resolved colour mode: the Colour
// Map of Presentation Levels 2.5 and 3.5 (EN 300 706 clause 12.4) and the
// receiver's eight colours after it (the engine's DEC-2464).
pub use crate::frame::MAX_COLOURS;
pub use frame::{PageArea, write_frame_to_core};
pub use view::View;
pub(crate) use view::names_one_subpage;

/// The most bytes of a channel's data in one message.
pub const MAX_CHUNK: usize = 64 * 1024;
/// The most bytes of one whole file or message: the largest payload less
/// this message's own byte, above the engine's 256 KB page limit.
pub const MAX_WHOLE: usize = crate::MAX_PAYLOAD - 1;
/// The bytes of a rule input's message before the input: its page and
/// its sequence number.
const INPUT_HEAD_BYTES: usize = 2 + 2 + 4;
/// A whole page's flags, before its bytes.
const WHOLE_FLAGS_BYTES: usize = 1;

/// The longest text of a channel's log line: the engine's
/// `neoteletext_formats::log::MAX_LOG_TEXT_BYTES`, which the decoder
/// asserts this equals.
pub const MAX_CHANNEL_LOG: usize = 200;
/// A log line's page when it followed none.
pub(crate) const NO_PAGE: u16 = 0;

/// The pages and sub-pages a decoder first has room for: about 0.7 MB of
/// store, enough for most channels; a decoder that fills it is started
/// again with twice the room (decision 47). A budget, not a maximum: the
/// store is a cache, the least recently received sub-page forgotten first
/// (the engine's REQ-ACQ-005), and a channel may have every page and
/// sub-page the standard allows (decision 112 item 2).
pub const FIRST_STORE_PAGES: usize = 256;
/// The most a decoder's store holds, as the engine's own sessions keep
/// (`neoteletext-app`'s `DEFAULT_PAGES`): 10.5 MB.
pub const MAX_STORE_PAGES: usize = 4096;

/// From the trusted core to a decoder.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ToDecoder {
    /// More of the channel's bytes, in the order they were fetched.
    Data(Vec<u8>),
    /// The channel's data is complete.
    End,
    /// The pages to render: those on screen. The decoder draws them again
    /// when they change and, by its own clock, when they flash.
    Showing(Vec<u16>),
    /// The channel is closed: the decoder ends.
    Close,
    /// The page the user opened, or none: its sounds play while it is open.
    Opened(Option<u16>),
    /// A whole page file, bundle or pushed message: a TTI page, or whole
    /// 42-byte packets. Its pages are complete as it ends, so they show at
    /// once (`interaction.md` section 10); a fetched copy of a page the
    /// channel has pushed is older and is dropped.
    Whole {
        /// Packets, not a TTI page.
        packets: bool,
        /// Pushed by the channel, not fetched.
        pushed: bool,
        /// The bytes.
        bytes: Vec<u8>,
    },
    /// How a page is drawn: its reveal, hold, mix and sub-page. A page with
    /// no view is drawn as it arrives.
    View(View),
    /// The rows of a page's stored sub-pages the sub-page list asks for,
    /// with their titles (decision 112 item 3): answered by
    /// [`FromDecoder::SubPageRows`].
    SubPageRows {
        /// The page.
        page: u16,
        /// The rows, at most [`interface_core::controls::SUBPAGE_ROWS`].
        range: SubPageRange,
    },
    /// The choices for the picture whose bytes follow as data, to a decoder
    /// started to convert it (`--picture`, `docs/picture-tools.md`).
    Picture(interface_core::picture::PictureChoice),
    /// The group of a page that names none, as the channel's descriptor
    /// gives it, or none for the decoder's own (the engine's REQ-WEB-002).
    Region(Option<u8>),
    /// The look of one area of a page under the pointer, hovered or
    /// pressed, or none (DOC-1947, the engine's decision
    /// E.127): drawn on the page's own cells.
    #[cfg(feature = "links")]
    Look {
        /// The page.
        page: u16,
        /// The area and whether it is pressed.
        look: Option<(crate::links::Area, bool)>,
    },
    /// What the viewer's pointer is doing to a sub-page's ReGIS picture
    /// (REQ-6374): its looks are drawn, and its cursor drawing blinked.
    RegisViewer {
        /// The page.
        page: u16,
        /// The sub-code.
        sub: u16,
        /// The hovered and pressed drawings, and whether keys go to the
        /// channel.
        viewer: crate::regis::RegisViewer,
    },
    /// One input event of the viewer for the open page, whose table asked
    /// for its class (page-rules.md RULE-52 item 1): what the evaluator
    /// matches, under the sequence number a report names it by. The
    /// event's bytes stay with the trusted core.
    RuleInput {
        /// The open page and its sub-code (RULE-112).
        page: crate::rules::PageSub,
        /// The trusted core's number for the event.
        sequence: u32,
        /// The event.
        input: crate::rules::RuleInput,
    },
    /// The sound helper's newest metered report for the open page's watch
    /// list (RULE-33 item 4): one byte for each slot, in the list's order,
    /// at most [`crate::audio::MAX_WATCH_SLOTS`].
    Metered {
        /// The open page and its sub-code.
        page: crate::rules::PageSub,
        /// The values.
        values: Vec<u8>,
    },
    /// The page and sub-code of this channel that is the open page of the
    /// active tab: the one page keys would go to (RULE-58, RULE-112). Only
    /// its table is evaluated and only it plays a local sound. None is the
    /// pause of RULE-111 case 4: the tab is not the active one, and the
    /// page it has selected keeps its state, evaluating nothing.
    RulesPage(Option<crate::rules::PageSub>),
    /// The view's push connection ended (RULE-111 case 2): every local
    /// state of the page is dropped at once, with no `close` tick, and the
    /// picture stays as it stood.
    ConnectionEnded,
    /// The sound helper heard a cue of a sound on an audio channel of this
    /// view (RULE-109): its device buffer holding the cue's sample was
    /// rendered.
    CueHeard {
        /// The audio channel, 0 to 3.
        audio_channel: u8,
        /// The cue's number, 0 to `MAX_CUE`.
        number: u8,
    },
    /// The interface's frames (RULE-98 item 2): its refresh period and the
    /// time from this message's writing to its next composition, for the
    /// decoder to lock its presentation clock and its paint points to.
    FramePhase(crate::rules::FramePhase),
    /// The viewer's setting "Paint drawings gradually" (animation.md
    /// ANIM-15, DEC-6525 item 27): on, a ReGIS drawing with a paint rate
    /// appears at that rate and a NAPLPS WAIT delays what follows it;
    /// off, everything appears at once. A decoder begins with it on.
    PaintGradually(bool),
    /// The platform asks for reduced motion (REQ-754, renderer parity
    /// PAR-33): on, nothing only the receiver moves moves, while every
    /// state change the channel or the page's rules make still applies. A
    /// decoder begins with it off.
    ReducedMotion(bool),
    /// A state query the channel sent for a page (RULE-47): answered once
    /// by [`FromDecoder::PageState`] when that page is the open page.
    RulesQuery {
        /// The page and sub-code.
        page: crate::rules::PageSub,
        /// The query's identifier.
        identifier: u32,
    },
}

/// The figures of a decoder's audio receiver (the engine's REQ-AUD-017,
/// `neoteletext_audio::ReceiverLimits`): the bytes of its object store,
/// the objects it holds and the records it reassembles at once.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AudioReceiver {
    /// Bytes of the object store.
    pub store_bytes: u32,
    /// Objects held at once.
    pub objects: u32,
    /// Records reassembled at once.
    pub records_in_flight: u32,
}

/// From a decoder to the trusted core.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FromDecoder {
    /// It is confined and running: a channel's decoder says this first,
    /// before it reads anything, so the trusted core can tell one that
    /// could not start from one a channel's data ended (finding F-12).
    Confined,
    /// What its audio receiver holds, said once the receiver is built,
    /// for the developer console's Log tab (the engine's REQ-AUD-017: a
    /// port publishes its receiver's figures). Untrusted, as all it says.
    AudioReceiver(AudioReceiver),
    /// A page has been received completely for the first time, with a
    /// title taken from its text, not yet cleaned.
    Arrived {
        /// The page number.
        page: u16,
        /// Its title.
        title: String,
    },
    /// A page already reported as arrived has a picture's title it did
    /// not arrive with, or a new one (DEC-6638 item 3): a NAPLPS picture's
    /// title came after its page, or changed. Not yet cleaned.
    PictureTitle {
        /// The page number.
        page: u16,
        /// The picture's title.
        title: String,
    },
    /// A rendered page: palette indices, 480 by 500, into `palette`.
    Frame {
        /// The page number.
        page: u16,
        /// Red, green and blue of each colour the page uses.
        palette: Vec<[u8; 3]>,
        /// The palette index of the screen around the page (EN 300 706
        /// Table 4, Default Screen Colour; clause 12.3, Full Screen Colour).
        screen: u8,
        /// One palette index per dot, row by row.
        indices: Vec<u8>,
    },
    /// The rectangle of a teletext page that changed since the frame the
    /// decoder last sent of it (animation.md ANIM-5): laid over that
    /// frame, whose palette and screen colour it has.
    FrameArea {
        /// The page number.
        page: u16,
        /// Red, green and blue of each colour the page uses, as the frame's.
        palette: Vec<[u8; 3]>,
        /// The screen colour's index, as the frame's.
        screen: u8,
        /// Where, in dots of the page.
        area: PageArea,
        /// One palette index per dot of the area, row by row.
        indices: Vec<u8>,
    },
    /// A sub-page drawn by the ReGIS renderer (DEC-6335), which is shown in
    /// place of its teletext page: its picture, and its pick record, which
    /// the trusted core keeps to say what lay under a pointer.
    Regis {
        /// The page number.
        page: u16,
        /// The sub-code the channel sent the ReGIS for.
        sub: u16,
        /// The picture.
        picture: crate::regis::RegisPicture,
        /// Its pick record, when it differs from the last one sent for
        /// the sub-page (REQ-6399 item 4); `None` when that one still holds.
        pick: Option<crate::regis::RegisPick>,
    },
    /// The numbered drawings of a sub-page's ReGIS picture that are links,
    /// and the target each leads to (the engine's REQ-6370): every one the
    /// channel named, replacing those named before.
    #[cfg(feature = "links")]
    RegisLinks {
        /// The page.
        page: u16,
        /// The sub-code.
        sub: u16,
        /// Each drawing and its target, as read.
        links: Vec<(u16, String)>,
    },
    /// A page already reported has changed: a new version arrived.
    Updated {
        /// The page number.
        page: u16,
    },
    /// The page store is full: the channel has more pages and sub-pages
    /// than it has room for, and the oldest are being forgotten.
    StoreFull,
    /// The pages on screen, in the order they were named, that it has no
    /// entry of in the tab's cache, for the trusted core to select in turn
    /// (REQ-1559). Said when the list changes.
    Missing(Vec<u16>),
    /// Something found in a bookmark file the decoder was given to read
    /// (decision 71).
    Bookmark(BookmarkEntry),
    /// A log line the channel wrote for its author (the engine's
    /// REQ-INP-026, an `LG` line of a TTI page): untrusted, for the
    /// console's Log tab only (decision 70).
    ChannelLog {
        /// Its level.
        level: interface_core::console::Level,
        /// The page it was written on, when it followed a page's number.
        page: Option<u16>,
        /// Its text, at most [`MAX_CHANNEL_LOG`] bytes.
        text: String,
    },
    /// The bookmark file is read: what was found has been sent, and this
    /// many bookmarks and folders were left out.
    BookmarksRead {
        /// Left out: not addresses, too deep, or past the bound.
        skipped: u32,
    },
    /// A voice operation of the channel's audio receiver.
    Audio(crate::audio::Audio),
    /// The page a picture decoder made: its TTI page file and its T42
    /// packets, each at most [`crate::picture::MAX_PICTURE_PAGE`] bytes,
    /// and the converter's summary, untrusted (`docs/picture-tools.md`).
    PictureConverted {
        /// The TTI page file.
        tti: Vec<u8>,
        /// The same page as T42 packets.
        t42: Vec<u8>,
        /// What the converter used, not yet cleaned.
        summary: String,
    },
    /// Why a picture decoder made no page, not yet cleaned.
    PictureRefused(String),
    /// The picture a picture decoder converted, in a picture language, at
    /// most [`crate::picture::MAX_PICTURE_OUTPUT`] bytes, or why it made
    /// none, untrusted: sent after [`FromDecoder::PictureConverted`], once
    /// for ReGIS and once for NAPLPS (renderer parity PAR-36 item 2).
    PictureOutput {
        /// The language.
        language: interface_core::canvas::PictureLanguage,
        /// The bytes, or why there are none, not yet cleaned.
        output: Result<Vec<u8>, String>,
    },
    /// The links found in a page, sent before its image whenever they
    /// change (`hyperlinks.md` section 8.1): untrusted, for the trusted
    /// core to check and decide.
    #[cfg(feature = "links")]
    Links {
        /// The page number.
        page: u16,
        /// A new image of the page follows: its links are for that image.
        /// Otherwise only the links changed, and they are for the image the
        /// trusted core already has.
        with_image: bool,
        /// Page numbers in its text and the runs its `LT` lines declare.
        candidates: Vec<crate::links::Candidate>,
        /// Its `LK` lines.
        numbers: Vec<crate::links::NumberLink>,
        /// Its controls (DOC-1844), as the engine found them.
        controls: Vec<crate::links::Area>,
        /// Its coloured keys and Index, from packet X/27/0.
        fastext: crate::links::Fastext,
    },
    /// The sub-pages of a page on screen from the one drawn, whenever they
    /// change (REQ-NAV-005): how many, which is drawn, its place and its
    /// neighbours, never the list (decision 112 item 4).
    SubPages {
        /// The page number.
        page: u16,
        /// The count, the one drawn, its place and neighbours.
        step: SubPageStep,
    },
    /// How many sub-pages a page has stored, whenever it changes: for the
    /// page list's mark (decision 111). 0 or 1 means it has none.
    SubPageCount {
        /// The page number.
        page: u16,
        /// How many, at most the standard's 8,190.
        count: u16,
    },
    /// The rows of a page's stored sub-pages asked for, in ascending order,
    /// each with a title from its text, not yet cleaned.
    SubPageRows {
        /// The page number.
        page: u16,
        /// The page's count, the first row's place and the rows, at most
        /// [`interface_core::controls::SUBPAGE_ROWS`].
        rows: SubPageRows,
    },
    /// A cursor record the channel sent for a page, from a pushed packet or
    /// a page's `CU` line: untrusted, its body as sent, for the trusted
    /// core to check (DOC-1954).
    #[cfg(feature = "interaction")]
    Cursor {
        /// The record's body: page, acknowledgement, row, column, width
        /// and flags.
        body: [u8; neoteletext_input::cursor::BODY],
    },
    /// An input control record the channel sent (WELCOME, `PUSH_MODE`,
    /// `POP_MODE`, `PAGE_MODES`), or a page's `IM` line as `PAGE_MODES`:
    /// untrusted, its body as sent, at most
    /// `neoteletext_input::control::MAX_BODY` bytes, for the trusted core
    /// to read (REQ-INT-111, REQ-INT-114).
    #[cfg(feature = "interaction")]
    InputControl {
        /// The control message's body.
        body: Vec<u8>,
    },
    /// What the open page's table asks for (page-rules.md RULE-52 item 1,
    /// RULE-33 item 1, RULE-51 item 4): untrusted, bounded here.
    RulesOpen(crate::rules::RulesOpen),
    /// A report the decoder asks the trusted core to send for a rule's
    /// `send` or `echo` (RULE-52 item 3): it names an input event by its
    /// sequence number and never carries its bytes.
    Report(crate::rules::Report),
    /// The answer to a state query (RULE-47): the `PAGE_STATE` payload
    /// after its stamp, at most [`crate::rules::MAX_PAGE_STATE`] bytes,
    /// which begins with the query's identifier.
    PageState {
        /// The page and sub-code.
        page: crate::rules::PageSub,
        /// The query's identifier.
        identifier: u32,
        /// The payload after the stamp.
        body: Vec<u8>,
    },
    /// A fault of a page's table (RULE-48).
    RulesFault {
        /// The page and sub-code.
        page: crate::rules::PageSub,
        /// The table's generation.
        generation: u16,
        /// The fault's code, 1 to [`crate::rules::MAX_FAULT`].
        fault: u8,
        /// How many times since the last report.
        count: u16,
    },
    /// The open page's counts (RULE-72), for the console.
    RuleCounters(crate::rules::RuleCounters),
}

/// # Errors
/// When writing fails or the message breaks a bound.
pub fn write_to_decoder(out: &mut dyn Write, message: &ToDecoder) -> Result<(), ProtocolError> {
    let mut payload = Vec::new();
    let kind = match message {
        ToDecoder::Data(bytes) => {
            if bytes.len() > MAX_CHUNK {
                return Err(ProtocolError::TooLong(bytes.len()));
            }
            payload.reserve_exact(bytes.len());
            payload.extend_from_slice(bytes);
            KIND_DATA
        }
        ToDecoder::End => KIND_END,
        ToDecoder::Showing(pages) => {
            push_count(&mut payload, pages.len(), PAGES, "too many pages")?;
            for page in pages {
                // Only display pages, as the decoder reads them: any other
                // is refused there, and the decoder restarts (finding F-29).
                crate::fields::check_page(*page)?;
                payload.extend_from_slice(&page.to_be_bytes());
            }
            KIND_SHOWING
        }
        ToDecoder::Close => KIND_CLOSE,
        ToDecoder::Opened(page) => {
            if let Some(page) = page {
                crate::fields::check_page(*page)?;
                payload.extend_from_slice(&page.to_be_bytes());
            }
            KIND_OPENED
        }
        ToDecoder::Whole { packets, pushed, bytes } => {
            if bytes.len() > MAX_WHOLE {
                return Err(ProtocolError::TooLong(bytes.len()));
            }
            payload.reserve_exact(WHOLE_FLAGS_BYTES.saturating_add(bytes.len()));
            payload.push(u8::from(*packets) | (u8::from(*pushed) << 1));
            payload.extend_from_slice(bytes);
            KIND_WHOLE
        }
        ToDecoder::Region(region) => {
            push_region(&mut payload, *region)?;
            KIND_REGION
        }
        ToDecoder::Picture(choice) => {
            crate::picture::push_choice(&mut payload, *choice)?;
            KIND_PICTURE
        }
        ToDecoder::View(view) => {
            view::push_view(&mut payload, view)?;
            KIND_VIEW
        }
        ToDecoder::SubPageRows { page, range } => {
            crate::fields::check_page(*page)?;
            payload.extend_from_slice(&page.to_be_bytes());
            crate::subpages::push_range(&mut payload, *range, SUBPAGE_ROWS)?;
            KIND_ASK_SUBPAGE_ROWS
        }
        #[cfg(feature = "links")]
        ToDecoder::Look { page, look } => {
            crate::fields::check_page(*page)?;
            payload.extend_from_slice(&page.to_be_bytes());
            if let Some((area, pressed)) = look {
                crate::links::push_area(&mut payload, *area);
                payload.push(u8::from(*pressed));
            }
            KIND_LOOK
        }
        ToDecoder::RegisViewer { page, sub, viewer } => {
            crate::fields::check_page(*page)?;
            payload.extend_from_slice(&page.to_be_bytes());
            payload.extend_from_slice(&sub.to_be_bytes());
            crate::regis::push_viewer(&mut payload, *viewer);
            kinds::KIND_REGIS_VIEWER
        }
        ToDecoder::RuleInput { .. }
        | ToDecoder::Metered { .. }
        | ToDecoder::RulesQuery { .. }
        | ToDecoder::RulesPage(_)
        | ToDecoder::ConnectionEnded
        | ToDecoder::CueHeard { .. }
        | ToDecoder::FramePhase(_) => push_rules_to_decoder(&mut payload, message)?,
        ToDecoder::PaintGradually(gradual) => {
            payload.push(u8::from(*gradual));
            kinds::KIND_PAINT_GRADUALLY
        }
        ToDecoder::ReducedMotion(reduced) => {
            payload.push(u8::from(*reduced));
            kinds::KIND_REDUCED_MOTION
        }
    };
    write_frame(out, kind, &payload)
}

/// Writes a rule message for a decoder (page-rules.md RULE-63); its kind.
fn push_rules_to_decoder(payload: &mut Vec<u8>, message: &ToDecoder) -> Result<u8, ProtocolError> {
    Ok(match message {
        ToDecoder::RuleInput { page, sequence, input } => {
            crate::rules::push_page_sub(payload, *page)?;
            payload.extend_from_slice(&sequence.to_be_bytes());
            crate::rules::push_input(payload, input);
            // What would be refused on arrival is refused on sending.
            let mut written = Fields::new(payload.get(INPUT_HEAD_BYTES..).unwrap_or(&[]));
            crate::rules::read_input(&mut written)?;
            kinds::KIND_RULE_INPUT
        }
        ToDecoder::Metered { page, values } => {
            crate::rules::push_page_sub(payload, *page)?;
            crate::push_count_u8(payload, values.len(), crate::audio::MAX_WATCH_SLOTS, "metered values")?;
            payload.extend_from_slice(values);
            kinds::KIND_METERED
        }
        ToDecoder::RulesQuery { page, identifier } => crate::rules::push_query(payload, *page, *identifier).map(|()| kinds::KIND_RULES_QUERY)?,
        ToDecoder::RulesPage(page) => {
            if let Some(page) = page {
                crate::rules::push_page_sub(payload, *page)?;
            }
            kinds::KIND_RULES_PAGE
        }
        ToDecoder::ConnectionEnded => kinds::KIND_CONNECTION_ENDED,
        ToDecoder::CueHeard { audio_channel, number } => {
            crate::rules::check_cue(*audio_channel, *number)?;
            payload.extend_from_slice(&[*audio_channel, *number]);
            kinds::KIND_CUE_HEARD
        }
        ToDecoder::FramePhase(phase) => crate::rules::push_frame_phase(payload, *phase).map(|()| kinds::KIND_FRAME_PHASE)?,
        _ => return Err(ProtocolError::BadValue("not a rule message")),
    })
}

/// Reads a rule message for a decoder.
fn read_rules_to_decoder(kind: u8, fields: &mut Fields<'_>) -> Result<ToDecoder, ProtocolError> {
    Ok(match kind {
        kinds::KIND_RULE_INPUT => {
            let (page, sequence) = (crate::rules::read_page_sub(fields)?, fields.u32()?);
            ToDecoder::RuleInput { page, sequence, input: crate::rules::read_input(fields)? }
        }
        kinds::KIND_METERED => {
            let page = crate::rules::read_page_sub(fields)?;
            let count = fields.count_u8(crate::audio::MAX_WATCH_SLOTS, "metered values")?;
            ToDecoder::Metered { page, values: fields.take(count)?.to_vec() }
        }
        kinds::KIND_RULES_PAGE => ToDecoder::RulesPage(if fields.is_empty() { None } else { Some(crate::rules::read_page_sub(fields)?) }),
        kinds::KIND_CONNECTION_ENDED => ToDecoder::ConnectionEnded,
        kinds::KIND_CUE_HEARD => {
            let (audio_channel, number) = (fields.u8()?, fields.u8()?);
            crate::rules::check_cue(audio_channel, number)?;
            ToDecoder::CueHeard { audio_channel, number }
        }
        kinds::KIND_FRAME_PHASE => ToDecoder::FramePhase(crate::rules::read_frame_phase(fields)?),
        _ => ToDecoder::RulesQuery { page: crate::rules::read_page_sub(fields)?, identifier: fields.u32()? },
    })
}

/// Writes a rule message from a decoder (RULE-63); its kind.
fn push_rules_to_core(payload: &mut Vec<u8>, message: &FromDecoder) -> Result<u8, ProtocolError> {
    Ok(match message {
        FromDecoder::RulesOpen(open) => crate::rules::push_open(payload, open).map(|()| kinds::KIND_RULES_OPEN)?,
        FromDecoder::Report(report) => crate::rules::push_report(payload, report).map(|()| kinds::KIND_REPORT)?,
        FromDecoder::PageState { page, identifier, body } => {
            if body.len() > crate::rules::MAX_PAGE_STATE {
                return Err(ProtocolError::TooLong(body.len()));
            }
            crate::rules::push_page_sub(payload, *page)?;
            payload.extend_from_slice(&identifier.to_be_bytes());
            payload.extend_from_slice(body);
            kinds::KIND_PAGE_STATE
        }
        FromDecoder::RulesFault { page, generation, fault, count } => {
            if !crate::rules::is_fault(*fault) {
                return Err(ProtocolError::BadValue("rules fault"));
            }
            crate::rules::push_page_sub(payload, *page)?;
            payload.extend_from_slice(&generation.to_be_bytes());
            payload.push(*fault);
            payload.extend_from_slice(&count.to_be_bytes());
            kinds::KIND_RULES_FAULT
        }
        FromDecoder::RuleCounters(counters) => crate::rules::push_counters(payload, counters).map(|()| kinds::KIND_RULE_COUNTERS)?,
        _ => return Err(ProtocolError::BadValue("not a rule message")),
    })
}

/// Reads a rule message from a decoder, untrusted.
fn read_rules_from_decoder(kind: u8, fields: &mut Fields<'_>) -> Result<FromDecoder, ProtocolError> {
    Ok(match kind {
        kinds::KIND_RULES_OPEN => FromDecoder::RulesOpen(crate::rules::read_open(fields)?),
        kinds::KIND_REPORT => FromDecoder::Report(crate::rules::read_report(fields)?),
        kinds::KIND_PAGE_STATE => {
            let (page, identifier) = (crate::rules::read_page_sub(fields)?, fields.u32()?);
            let length = fields.remaining();
            if length > crate::rules::MAX_PAGE_STATE {
                return Err(ProtocolError::TooLong(length));
            }
            FromDecoder::PageState { page, identifier, body: fields.take(length)?.to_vec() }
        }
        kinds::KIND_RULES_FAULT => {
            let (page, generation, fault, count) = (crate::rules::read_page_sub(fields)?, fields.u16()?, fields.u8()?, fields.u16()?);
            if !crate::rules::is_fault(fault) {
                return Err(ProtocolError::BadValue("rules fault"));
            }
            FromDecoder::RulesFault { page, generation, fault, count }
        }
        _ => FromDecoder::RuleCounters(crate::rules::read_counters(fields)?),
    })
}

/// Writes a character set region, which a fetcher hands on as a decoder takes
/// it: four bits (the engine's decision E.4), or nothing for none.
pub(crate) fn push_region(payload: &mut Vec<u8>, region: Option<u8>) -> Result<(), ProtocolError> {
    if let Some(region) = region {
        if region > NIBBLE_MASK {
            return Err(ProtocolError::BadValue("region"));
        }
        payload.push(region);
    }
    Ok(())
}

/// Reads what [`push_region`] wrote.
pub(crate) fn read_region(fields: &mut Fields<'_>) -> Result<Option<u8>, ProtocolError> {
    if fields.is_empty() {
        return Ok(None);
    }
    let region = fields.u8()?;
    if region > NIBBLE_MASK {
        return Err(ProtocolError::BadValue("region"));
    }
    Ok(Some(region))
}

/// Reads one message from the trusted core, or `None` at a clean end.
///
/// # Errors
/// On input errors, a kind the trusted core may not send a decoder, and
/// any payload that is malformed or out of bounds.
pub fn read_from_core(input: &mut dyn Read) -> Result<Option<ToDecoder>, ProtocolError> {
    let Some((kind, payload)) = read_frame(input)? else { return Ok(None) };
    let mut fields = Fields::new(&payload);
    let message = match kind {
        // The payload is the data: it is moved, not copied.
        KIND_DATA => {
            if payload.len() > MAX_CHUNK {
                return Err(ProtocolError::TooLong(payload.len()));
            }
            return Ok(Some(ToDecoder::Data(payload)));
        }
        KIND_END => ToDecoder::End,
        KIND_SHOWING => {
            let count = fields.count_u16(PAGES, "too many pages")?;
            ToDecoder::Showing((0..count).map(|_| fields.channel_page()).collect::<Result<_, _>>()?)
        }
        KIND_CLOSE => ToDecoder::Close,
        KIND_OPENED => ToDecoder::Opened(if payload.is_empty() { None } else { Some(fields.channel_page()?) }),
        KIND_WHOLE => {
            let flags = fields.u8()?;
            if flags > 3 {
                return Err(ProtocolError::BadValue("whole form"));
            }
            let length = fields.remaining();
            if length > MAX_WHOLE {
                return Err(ProtocolError::TooLong(length));
            }
            return Ok(Some(ToDecoder::Whole { packets: flags & 1 != 0, pushed: flags & 2 != 0, bytes: crate::tail(payload, WHOLE_FLAGS_BYTES) }));
        }
        KIND_REGION => ToDecoder::Region(read_region(&mut fields)?),
        KIND_PICTURE => ToDecoder::Picture(crate::picture::read_choice(&mut fields)?),
        KIND_VIEW => ToDecoder::View(view::read_view(&mut fields)?),
        KIND_ASK_SUBPAGE_ROWS => {
            let page = fields.channel_page()?;
            ToDecoder::SubPageRows { page, range: crate::subpages::read_range(&mut fields, SUBPAGE_ROWS)? }
        }
        #[cfg(feature = "links")]
        KIND_LOOK => {
            let page = fields.channel_page()?;
            let look = if fields.is_empty() { None } else { Some((crate::links::read_area(&mut fields)?, fields.flag("pressed")?)) };
            ToDecoder::Look { page, look }
        }
        kinds::KIND_REGIS_VIEWER => {
            let (page, sub) = (fields.channel_page()?, fields.u16()?);
            ToDecoder::RegisViewer { page, sub, viewer: crate::regis::read_viewer(&mut fields)? }
        }
        kinds::KIND_RULE_INPUT
        | kinds::KIND_METERED
        | kinds::KIND_RULES_QUERY
        | kinds::KIND_RULES_PAGE
        | kinds::KIND_CONNECTION_ENDED
        | kinds::KIND_CUE_HEARD
        | kinds::KIND_FRAME_PHASE => read_rules_to_decoder(kind, &mut fields)?,
        kinds::KIND_PAINT_GRADUALLY => ToDecoder::PaintGradually(fields.flag("paint gradually")?),
        kinds::KIND_REDUCED_MOTION => ToDecoder::ReducedMotion(fields.flag("reduced motion")?),
        other => return Err(ProtocolError::UnknownKind(other)),
    };
    fields.end()?;
    Ok(Some(message))
}

/// A message that is its page's number alone: the number written, and the
/// message's kind.
fn page_alone(payload: &mut Vec<u8>, page: u16, kind: u8) -> u8 {
    payload.extend_from_slice(&page.to_be_bytes());
    kind
}

/// Writes a sub-page's drawing links to the trusted core (REQ-6376).
#[cfg(feature = "links")]
fn write_regis_links(out: &mut dyn Write, page: u16, sub: u16, links: &[(u16, String)]) -> Result<(), ProtocolError> {
    crate::fields::check_page(page)?;
    let mut payload = Vec::new();
    payload.extend_from_slice(&page.to_be_bytes());
    payload.extend_from_slice(&sub.to_be_bytes());
    crate::regis::push_links(&mut payload, links)?;
    write_frame(out, kinds::KIND_REGIS_LINKS, &payload)
}

/// # Errors
/// When writing fails or the message breaks a bound.
#[allow(clippy::too_many_lines, reason = "one arm a message of the pipe, each a line or two")]
pub fn write_to_core(out: &mut dyn Write, message: &FromDecoder) -> Result<(), ProtocolError> {
    let mut payload = Vec::new();
    let kind = match message {
        FromDecoder::Arrived { page, title } => {
            payload.extend_from_slice(&page.to_be_bytes());
            // The decoder's text is untrusted; it is checked for length
            // here and cleaned by the trusted core, not trusted to be clean.
            push_raw_text(&mut payload, title, MAX_PAGE_TITLE);
            KIND_ARRIVED
        }
        FromDecoder::PictureTitle { page, title } => {
            payload.extend_from_slice(&page.to_be_bytes());
            // Untrusted as an arrival's title: checked for length here and
            // cleaned by the trusted core.
            push_raw_text(&mut payload, title, MAX_PAGE_TITLE);
            kinds::KIND_PICTURE_TITLE
        }
        FromDecoder::Frame { page, palette, screen, indices } => return write_frame_to_core(out, *page, palette, *screen, indices),
        FromDecoder::FrameArea { page, palette, screen, area, indices } => {
            return frame::write_frame_area_to_core(out, *page, palette, *screen, *area, indices);
        }
        FromDecoder::Updated { page } => page_alone(&mut payload, *page, KIND_UPDATED),
        FromDecoder::Regis { page, sub, picture, pick } => return frame::write_regis_to_core(out, *page, *sub, picture, pick.as_ref()),
        #[cfg(feature = "links")]
        FromDecoder::RegisLinks { page, sub, links } => return write_regis_links(out, *page, *sub, links),
        FromDecoder::Audio(audio) => {
            crate::audio::push(&mut payload, audio, crate::audio::Sender::Decoder)?;
            KIND_AUDIO
        }
        #[cfg(feature = "links")]
        FromDecoder::Links { page, with_image, candidates, numbers, controls, fastext } => {
            payload.extend_from_slice(&page.to_be_bytes());
            payload.push(u8::from(*with_image));
            crate::links::push_found(&mut payload, (candidates, numbers, controls))?;
            crate::links::push_fastext(&mut payload, fastext)?;
            KIND_LINKS
        }
        FromDecoder::SubPages { page, step } => {
            crate::fields::check_page(*page)?;
            payload.extend_from_slice(&page.to_be_bytes());
            crate::subpages::push_step(&mut payload, step)?;
            KIND_SUBPAGES
        }
        FromDecoder::SubPageCount { page, count } => {
            crate::fields::check_page(*page)?;
            if usize::from(*count) > neoteletext_core::limits::MAX_SUBPAGES {
                return Err(ProtocolError::BadValue("too many sub-pages"));
            }
            payload.extend_from_slice(&page.to_be_bytes());
            payload.extend_from_slice(&count.to_be_bytes());
            KIND_SUBPAGE_COUNT
        }
        FromDecoder::SubPageRows { page, rows } => {
            crate::fields::check_page(*page)?;
            payload.extend_from_slice(&page.to_be_bytes());
            // The decoder's titles are untrusted: cut to length here and
            // cleaned by the trusted core.
            crate::subpages::push_rows(&mut payload, rows, SUBPAGE_ROWS, |payload, title| {
                push_raw_text(payload, title, MAX_PAGE_TITLE);
                Ok(())
            })?;
            KIND_SUBPAGE_ROWS
        }
        #[cfg(feature = "interaction")]
        FromDecoder::Cursor { body } => {
            payload.extend_from_slice(body);
            KIND_CURSOR
        }
        #[cfg(feature = "interaction")]
        FromDecoder::InputControl { body } => {
            if body.len() > neoteletext_input::control::MAX_BODY {
                return Err(ProtocolError::TooLong(body.len()));
            }
            payload.extend_from_slice(body);
            KIND_INPUT_CONTROL
        }
        FromDecoder::PictureConverted { tti, t42, summary } => {
            crate::picture::push_converted(&mut payload, tti, t42, summary)?;
            KIND_PICTURE_CONVERTED
        }
        FromDecoder::PictureRefused(reason) => {
            push_raw_text(&mut payload, reason, interface_core::picture::MAX_STATUS);
            KIND_PICTURE_REFUSED
        }
        FromDecoder::PictureOutput { language, output } => {
            crate::picture::push_output(&mut payload, *language, output)?;
            KIND_PICTURE_OUTPUT
        }
        FromDecoder::Confined => KIND_CONFINED,
        FromDecoder::AudioReceiver(receiver) => {
            for figure in [receiver.store_bytes, receiver.objects, receiver.records_in_flight] {
                payload.extend_from_slice(&figure.to_be_bytes());
            }
            KIND_AUDIO_RECEIVER
        }
        FromDecoder::StoreFull => KIND_STORE_FULL,
        FromDecoder::Missing(pages) => {
            push_count(&mut payload, pages.len(), PAGES, "too many pages")?;
            for page in pages {
                crate::fields::check_page(*page)?;
                payload.extend_from_slice(&page.to_be_bytes());
            }
            KIND_MISSING
        }
        FromDecoder::Bookmark(entry) => bookmarks::push_entry(&mut payload, entry),
        FromDecoder::BookmarksRead { skipped } => {
            payload.extend_from_slice(&skipped.to_be_bytes());
            KIND_BOOKMARKS_READ
        }
        FromDecoder::ChannelLog { level, page, text } => {
            payload.push(crate::console::level_byte(*level));
            payload.extend_from_slice(&page.unwrap_or(NO_PAGE).to_be_bytes());
            push_raw_text(&mut payload, text, MAX_CHANNEL_LOG);
            KIND_CHANNEL_LOG
        }
        // The rule messages (page-rules.md RULE-63).
        rules => push_rules_to_core(&mut payload, rules)?,
    };
    write_frame(out, kind, &payload)
}

/// Reads one message from a decoder, or `None` at a clean end. Everything
/// a decoder sends is untrusted: every bound and every index is checked.
///
/// # Errors
/// On input errors, a kind a decoder may not send, and any payload that is
/// malformed or out of bounds.
pub fn read_from_decoder(input: &mut dyn Read) -> Result<Option<FromDecoder>, ProtocolError> {
    let Some((kind, payload)) = read_frame(input)? else { return Ok(None) };
    let mut fields = Fields::new(&payload);
    let message = match kind {
        KIND_ARRIVED => {
            let page = fields.channel_page()?;
            FromDecoder::Arrived { page, title: raw_text(&mut fields, MAX_PAGE_TITLE, "title too long")? }
        }
        kinds::KIND_PICTURE_TITLE => {
            let page = fields.channel_page()?;
            FromDecoder::PictureTitle { page, title: raw_text(&mut fields, MAX_PAGE_TITLE, "picture title too long")? }
        }
        KIND_FRAME => {
            let (page, palette, screen) = frame::read_frame(&mut fields)?;
            // The indices are the rest of the payload: moved, not copied.
            let offset = payload.len().saturating_sub(PAGE_DOTS);
            return Ok(Some(FromDecoder::Frame { page, palette, screen, indices: crate::tail(payload, offset) }));
        }
        kinds::KIND_FRAME_AREA => frame::read_frame_area(&mut fields)?,
        KIND_UPDATED => FromDecoder::Updated { page: fields.channel_page()? },
        KIND_REGIS => {
            let page = fields.channel_page()?;
            let sub = fields.u16()?;
            if sub > neoteletext_core::limits::LAST_SUBPAGE_CODE || sub & !neoteletext_core::limits::SUBCODE_MASK != 0 {
                return Err(ProtocolError::BadValue("regis sub-code"));
            }
            let picture = crate::regis::read_picture(&mut fields)?;
            let pick = if fields.flag("a ReGIS pick record")? { Some(crate::regis::read_pick(&mut fields, picture.language)?) } else { None };
            FromDecoder::Regis { page, sub, picture, pick }
        }
        #[cfg(feature = "links")]
        kinds::KIND_REGIS_LINKS => {
            let (page, sub) = (fields.channel_page()?, fields.u16()?);
            FromDecoder::RegisLinks { page, sub, links: crate::regis::read_links(&mut fields)? }
        }
        KIND_AUDIO => FromDecoder::Audio(crate::audio::read(&mut fields, crate::audio::Sender::Decoder)?),
        #[cfg(feature = "links")]
        KIND_LINKS => {
            let page = fields.channel_page()?;
            let with_image = fields.flag("with image")?;
            let (candidates, numbers, controls) = crate::links::read_found(&mut fields)?;
            let fastext = crate::links::read_fastext(&mut fields)?;
            FromDecoder::Links { page, with_image, candidates, numbers, controls, fastext }
        }
        KIND_SUBPAGES => {
            let page = fields.channel_page()?;
            FromDecoder::SubPages { page, step: crate::subpages::read_step(&mut fields)? }
        }
        KIND_SUBPAGE_COUNT => {
            let page = fields.channel_page()?;
            FromDecoder::SubPageCount { page, count: crate::subpages::read_count(&mut fields)? }
        }
        KIND_SUBPAGE_ROWS => {
            let page = fields.channel_page()?;
            let rows = crate::subpages::read_rows(&mut fields, SUBPAGE_ROWS, |fields| raw_text(fields, MAX_PAGE_TITLE, "title too long"))?;
            FromDecoder::SubPageRows { page, rows }
        }
        // Only its length here: the trusted core checks the fields.
        #[cfg(feature = "interaction")]
        KIND_CURSOR => FromDecoder::Cursor { body: fields.array()? },
        #[cfg(feature = "interaction")]
        KIND_INPUT_CONTROL => {
            let length = fields.remaining();
            if length > neoteletext_input::control::MAX_BODY {
                return Err(ProtocolError::TooLong(length));
            }
            FromDecoder::InputControl { body: fields.take(length)?.to_vec() }
        }
        KIND_PICTURE_CONVERTED => {
            let (tti, t42, summary) = crate::picture::read_converted(&mut fields)?;
            FromDecoder::PictureConverted { tti, t42, summary }
        }
        KIND_PICTURE_REFUSED => FromDecoder::PictureRefused(raw_text(&mut fields, interface_core::picture::MAX_STATUS, "reason too long")?),
        KIND_PICTURE_OUTPUT => {
            let (language, output) = crate::picture::read_output(&mut fields)?;
            FromDecoder::PictureOutput { language, output }
        }
        KIND_CONFINED => FromDecoder::Confined,
        KIND_AUDIO_RECEIVER => {
            FromDecoder::AudioReceiver(AudioReceiver { store_bytes: fields.u32()?, objects: fields.u32()?, records_in_flight: fields.u32()? })
        }
        KIND_STORE_FULL => FromDecoder::StoreFull,
        KIND_MISSING => {
            let count = fields.count_u16(PAGES, "too many pages")?;
            FromDecoder::Missing((0..count).map(|_| fields.channel_page()).collect::<Result<_, _>>()?)
        }
        KIND_BOOKMARK_FOLDER | KIND_BOOKMARK_FOLDER_END | KIND_BOOKMARK_LINK => FromDecoder::Bookmark(bookmarks::read_entry(kind, &mut fields)?),
        KIND_BOOKMARKS_READ => FromDecoder::BookmarksRead { skipped: fields.u32()? },
        KIND_CHANNEL_LOG => {
            let level = crate::console::level_from_byte(fields.u8()?)?;
            let page = match fields.u16()? {
                NO_PAGE => None,
                page => Some(crate::fields::check_page(page).map(|()| page)?),
            };
            FromDecoder::ChannelLog { level, page, text: raw_text(&mut fields, MAX_CHANNEL_LOG, "log line too long")? }
        }
        kinds::KIND_RULES_OPEN | kinds::KIND_REPORT | kinds::KIND_PAGE_STATE | kinds::KIND_RULES_FAULT | kinds::KIND_RULE_COUNTERS => {
            read_rules_from_decoder(kind, &mut fields)?
        }
        other => return Err(ProtocolError::UnknownKind(other)),
    };
    fields.end()?;
    Ok(Some(message))
}
