//! The kind bytes of the messages between the trusted core and a
//! decoder.

// Trusted core to decoder.
pub(super) const KIND_DATA: u8 = 48;
pub(super) const KIND_END: u8 = 49;
pub(super) const KIND_SHOWING: u8 = 50;
// 51 was the trusted core's tick, removed on 21 October 2026 (decision
// 47): the decoder keeps its own clock. The number is not reused.
pub(super) const KIND_CLOSE: u8 = 52;
pub(super) const KIND_OPENED: u8 = 53;
pub(super) const KIND_WHOLE: u8 = 54;
pub(super) const KIND_VIEW: u8 = 55;
// The look under the pointer, far from the others: 56 to 63 were never
// used, and the next free kind of the whole protocol is taken.
#[cfg(feature = "links")]
pub(super) const KIND_LOOK: u8 = 99;
/// The channel's default character set region, in the next number free
/// across every pipe.
pub(super) const KIND_REGION: u8 = 139;
// Decoder to trusted core.
pub(super) const KIND_ARRIVED: u8 = 64;
pub(super) const KIND_FRAME: u8 = 65;
pub(super) const KIND_UPDATED: u8 = 66;
pub(super) const KIND_AUDIO: u8 = 67;
#[cfg(feature = "links")]
pub(super) const KIND_LINKS: u8 = 68;
#[cfg(feature = "interaction")]
pub(super) const KIND_CURSOR: u8 = 69;
pub(super) const KIND_STORE_FULL: u8 = 70;
#[cfg(feature = "interaction")]
pub(super) const KIND_INPUT_CONTROL: u8 = 71;
pub(super) const KIND_SUBPAGES: u8 = 72;
pub(super) const KIND_BOOKMARK_FOLDER: u8 = 73;
pub(super) const KIND_BOOKMARK_FOLDER_END: u8 = 74;
pub(super) const KIND_BOOKMARK_LINK: u8 = 75;
pub(super) const KIND_BOOKMARKS_READ: u8 = 76;
pub(super) const KIND_CHANNEL_LOG: u8 = 77;
pub(super) const KIND_CONFINED: u8 = 78;
// 79 was every sub-page's title, withdrawn on 28 October 2026 (decision
// 112): titles go by range. The number is not reused.
pub(super) const KIND_SUBPAGE_COUNT: u8 = 102;
pub(super) const KIND_SUBPAGE_ROWS: u8 = 103;
// Trusted core to decoder: the rows of a page's sub-pages asked for.
pub(super) const KIND_ASK_SUBPAGE_ROWS: u8 = 104;
// Decoder to trusted core: the pages on screen it cannot draw from its
// cache (REQ-1559), in the next number free across every pipe.
pub(super) const KIND_MISSING: u8 = 135;
// The Tools menu's picture converter (`docs/picture-tools.md`), in the next
// numbers free across every pipe: the choices, to a decoder started with
// `--picture`; the page it made; and why it made none.
pub(super) const KIND_PICTURE: u8 = 145;
pub(super) const KIND_PICTURE_CONVERTED: u8 = 146;
pub(super) const KIND_PICTURE_REFUSED: u8 = 147;
// Decoder to trusted core: its audio receiver's figures (the engine's
// REQ-AUD-017), in the next number free across every pipe.
pub(super) const KIND_AUDIO_RECEIVER: u8 = 152;
// Decoder to trusted core: a picture drawn by the ReGIS renderer and its
// pick record (DEC-6335), in the next number free across every pipe.
pub(super) const KIND_REGIS: u8 = 153;
// Trusted core to decoder: what the viewer's pointer is doing to a
// sub-page's ReGIS picture (REQ-6374), in the next number free across
// every pipe.
pub(super) const KIND_REGIS_VIEWER: u8 = 157;
// Decoder to trusted core: the numbered drawings of a sub-page's ReGIS
// picture that are links (REQ-6376), in the next number free across every
// pipe.
#[cfg(feature = "links")]
pub(super) const KIND_REGIS_LINKS: u8 = 158;
// A page's rule table between the trusted core and its decoder
// (neoteletext-common's page-rules.md RULE-63), in the next numbers free
// across every pipe on 18 November 2026: the viewer's input and the
// metered values to the decoder; what a table asks for, a report, a state
// answer and a fault from it; a state query to it; its counters.
pub(super) const KIND_RULE_INPUT: u8 = 178;
pub(super) const KIND_METERED: u8 = 179;
pub(super) const KIND_RULES_OPEN: u8 = 180;
pub(super) const KIND_REPORT: u8 = 181;
pub(super) const KIND_PAGE_STATE: u8 = 182;
pub(super) const KIND_RULES_FAULT: u8 = 183;
pub(super) const KIND_RULES_QUERY: u8 = 184;
pub(super) const KIND_RULE_COUNTERS: u8 = 185;
// Trusted core to decoder: the page of the channel that is the open page
// of the active tab, whose table alone is evaluated (RULE-58), or none.
pub(super) const KIND_RULES_PAGE: u8 = 187;
// Trusted core to decoder: the viewer's setting "Paint drawings
// gradually" (neoteletext-common's animation.md ANIM-15), in the next
// number free across every pipe on 18 November 2026.
pub(super) const KIND_PAINT_GRADUALLY: u8 = 188;
// Trusted core to decoder, for the second form of the rule table
// (page-rules.md RULE-63, RULE-98, RULE-111), in the next numbers free
// across every pipe on 18 November 2026: the view's connection ended, a
// cue was heard, and the interface's frame phase. 192 is the interface's
// own `FramePhase` to the trusted core.
pub(super) const KIND_CONNECTION_ENDED: u8 = 189;
pub(super) const KIND_CUE_HEARD: u8 = 190;
pub(super) const KIND_FRAME_PHASE: u8 = 191;
// Decoder to trusted core: the rectangle of a teletext page that changed
// (animation.md ANIM-5), in the next number free across every pipe after
// the interface's 192.
pub(super) const KIND_FRAME_AREA: u8 = 193;
// Decoder to trusted core: a picture's title that came after its page
// arrived (DEC-6638 item 3, TODO-6748), in a number clear of the fetcher's
// 200 and of what other work in flight on 19 November 2026 may take
// below it.
pub(super) const KIND_PICTURE_TITLE: u8 = 201;
// Trusted core to decoder: the platform asks for reduced motion (renderer
// parity PAR-33), in the next number free across every pipe on 19
// November 2026.
pub(super) const KIND_REDUCED_MOTION: u8 = 202;
