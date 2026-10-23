//! The kind bytes of the trusted core's messages to the interface and the
//! interface's intents to the trusted core, and the flag bits of the
//! strip's and a tab's state.
//!
//! The kinds of an optional area live in that area's file, so they are
//! compiled out with it; they are listed here so that the numbering reads
//! in one place:
//!
//! | Kind | Name | File |
//! |---|---|---|
//! | 11 to 15 | `KIND_PAGE_LINKS` to `KIND_ALIASES` | `links.rs` |
//! | 24 | `KIND_POINT` | `links.rs` |
//! | 25 | `KIND_TYPED` | `links.rs` |
//! | 26 | `KIND_FOCUSED` | here: hyperlinks and typing both use it |
//! | 27 | `KIND_INPUT` | `interaction.rs` |
//! | 28 | `KIND_PHONE_PANEL` | `keyboard.rs` |
//! | 29 | `KIND_PHONE_FINISH` | `keyboard.rs` |
//! | 30 | `KIND_EDITING` | `interaction.rs` |
//! | 36 | `KIND_CONFIRM` | `links.rs` |
//! | 37 | `KIND_CANCEL` | `links.rs` |
//! | 40 | `KIND_TYPING` | `interaction.rs` |
//! | 41 | `KIND_PHONE` | `keyboard.rs` |
//! | 42 | `KIND_PHONE_KEY` | `keyboard.rs` |

// Trusted core to interface.
pub(crate) const KIND_STRIP: u8 = 2;
pub(crate) const KIND_FRAME: u8 = 3;
pub(crate) const KIND_CLOSE: u8 = 4;
pub(crate) const KIND_TABS: u8 = 5;
pub(crate) const KIND_CHANNEL: u8 = 6;
pub(crate) const KIND_PAGE: u8 = 8;
pub(crate) const KIND_UPDATED: u8 = 9;
pub(crate) const KIND_TAB_SOUNDING: u8 = 10;
pub(crate) const KIND_SETTINGS: u8 = 43;
pub(crate) const KIND_CONSOLE_UPDATE: u8 = 44;
// Windows (`multiple-windows.md`, decision 72).
pub(crate) const KIND_TARGET: u8 = 46;
pub(crate) const KIND_OPEN_WINDOW: u8 = 47;
pub(crate) const KIND_CLOSE_WINDOW_VIEW: u8 = 49;
// Interface to trusted core: intents.
pub(crate) const KIND_VISIBLE: u8 = 16;
pub(crate) const KIND_CELL: u8 = 17;
pub(crate) const KIND_SELECT_TAB: u8 = 18;
pub(crate) const KIND_CLOSE_TAB: u8 = 19;
pub(crate) const KIND_NEW_TAB: u8 = 20;
pub(crate) const KIND_OPENED: u8 = 21;
pub(crate) const KIND_MUTE_TAB: u8 = 22;
pub(crate) const KIND_MUTE_ALL: u8 = 23;
#[cfg(any(feature = "links", feature = "interaction"))]
pub(crate) const KIND_FOCUSED: u8 = 26;
// Interface to trusted core: strip input, kinds of their own
// (`architecture.md` section 6.4).
pub(crate) const KIND_NAVIGATE: u8 = 32;
pub(crate) const KIND_BACK: u8 = 33;
pub(crate) const KIND_FORWARD: u8 = 34;
pub(crate) const KIND_BOOKMARK: u8 = 35;
pub(crate) const KIND_CONSOLE: u8 = 38;
pub(crate) const KIND_HOME: u8 = 39;
pub(crate) const KIND_CHANGE_SETTING: u8 = 45;
// History and bookmarks (`history-and-bookmarks.md` section 5): the
// trusted core's views, then the interface's requests.
pub(crate) const KIND_HISTORY_VIEW: u8 = 50;
pub(crate) const KIND_SUGGESTIONS: u8 = 51;
pub(crate) const KIND_TAB_LIST_VIEW: u8 = 52;
pub(crate) const KIND_BOOKMARKS_VIEW: u8 = 53;
pub(crate) const KIND_STAR_VIEW: u8 = 54;
pub(crate) const KIND_BAR_VIEW: u8 = 55;
pub(crate) const KIND_WINDOW_VIEW: u8 = 56;
// The cookies panel (`cookies.md` section 9).
pub(crate) const KIND_COOKIES_VIEW: u8 = 57;
pub(crate) const KIND_PAGE_CONTROLS: u8 = 58;
pub(crate) const KIND_REMOTE: u8 = 59;
pub(crate) const KIND_HISTORY: u8 = 60;
pub(crate) const KIND_BOOKMARKS: u8 = 61;
pub(crate) const KIND_STAR: u8 = 62;
pub(crate) const KIND_BAR: u8 = 63;
pub(crate) const KIND_SUGGEST: u8 = 64;
pub(crate) const KIND_SUGGESTION: u8 = 65;
pub(crate) const KIND_TAB_LIST: u8 = 66;
pub(crate) const KIND_GO_TO: u8 = 67;
pub(crate) const KIND_NEW_PRIVATE_TAB: u8 = 68;
pub(crate) const KIND_WINDOW: u8 = 69;
pub(crate) const KIND_COOKIES: u8 = 70;
pub(crate) const KIND_RELOAD: u8 = 71;
pub(crate) const KIND_PAGE_CONTROL: u8 = 72;
pub(crate) const KIND_MOVE_TAB: u8 = 73;
pub(crate) const KIND_FROM: u8 = 74;
pub(crate) const KIND_NEW_WINDOW: u8 = 75;
pub(crate) const KIND_TAB_TO_WINDOW: u8 = 76;
pub(crate) const KIND_CLOSE_WINDOW: u8 = 77;
// Updates (`installation-and-updates.md` sections 3.2 and 3.4), far from
// the others, so kinds added beside these elsewhere do not meet them.
pub(crate) const KIND_UPDATE_VIEW: u8 = 120;
pub(crate) const KIND_DRAWN: u8 = 121;
pub(crate) const KIND_INSTALL_UPDATE: u8 = 122;

// The strip's flags.
pub(crate) const ENCRYPTED: u8 = 1;
pub(crate) const CAN_GO_BACK: u8 = 2;
pub(crate) const CAN_GO_FORWARD: u8 = 4;
pub(crate) const BOOKMARKED: u8 = 8;
pub(crate) const LOCAL: u8 = 16;
pub(crate) const BUILT_IN: u8 = 32;
pub(crate) const PRIVATE: u8 = 64;
pub(crate) const ALL_FLAGS: u8 = ENCRYPTED | CAN_GO_BACK | CAN_GO_FORWARD | BOOKMARKED | LOCAL | BUILT_IN | PRIVATE;
// A tab's flags.
pub(crate) const TAB_MUTED: u8 = 1;
pub(crate) const TAB_PRIVATE: u8 = 2;
pub(crate) const TAB_FLAGS: u8 = TAB_MUTED | TAB_PRIVATE;
