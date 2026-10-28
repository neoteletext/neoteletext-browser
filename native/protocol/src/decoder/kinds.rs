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
