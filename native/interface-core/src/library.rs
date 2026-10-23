//! History and bookmarks as the trusted core shows them, and what the
//! panels, the star, the bar, the address field and the Back and Forward
//! lists ask for (`history-and-bookmarks.md`). Every request is strip input
//! the trusted core checks; a row is named only by the number the trusted
//! core gave it in the view it sent, never by an address (requirement
//! 20.5), so the interface can open or change nothing it was not shown.

use alloc::string::String;
use alloc::vec::Vec;

/// The most rows of history, bookmarks or folders in one view: the rows on
/// screen at the tallest window, and a margin (section 2.4).
pub const MAX_VIEW_ROWS: usize = 100;
/// The most suggestions under the address field (section 2.10).
pub const MAX_SUGGESTIONS: usize = 8;
/// The most entries of a Back or Forward list (section 2.9): Chrome's
/// `BackForwardMenuModel::kMaxHistoryItems`.
pub const MAX_TAB_LIST: usize = 12;
/// The most chapter stops after a full list: Chrome's
/// `BackForwardMenuModel::kMaxChapterStops`.
pub const MAX_CHAPTER_STOPS: usize = 5;
/// The most entries a tab's back-and-forward list keeps, the oldest
/// dropped first: Chrome's `blink::kMaxSessionHistoryEntries`.
pub const MAX_TAB_ENTRIES: usize = 50;
/// The most steps a list's entry can be from the tab's place.
pub const MAX_STEPS: usize = MAX_TAB_ENTRIES - 1;
const _: () = assert!(MAX_STEPS <= u8::MAX as usize, "a step count is one byte");
/// The most recently closed tabs kept (section 2.8).
pub const MAX_CLOSED: usize = 25;
/// The most windows open at once (`multiple-windows.md`): more than anyone
/// arranges on a desktop, and few enough that every window's lines fit the
/// session file.
pub const MAX_WINDOWS: usize = 16;
/// The longest search text.
pub const MAX_QUERY: usize = 256;
/// The longest name of a bookmark or folder, and a channel's title.
pub const MAX_NAME: usize = 256;
/// The deepest folder (section 3.1).
pub const MAX_FOLDER_DEPTH: usize = 16;
/// The longest address as a view shows it: more than a panel's row can
/// hold, and short enough that a view of [`MAX_VIEW_ROWS`] rows always
/// fits one message. A row is opened by its number, never by this text.
pub const MAX_SHOWN_ADDRESS: usize = 512;
/// The longest heading, time or folder path a view shows.
pub const MAX_LABEL: usize = 256;
/// The longest notice a panel says.
pub const MAX_NOTICE: usize = 320;
/// The longest path typed to import bookmarks from.
pub const MAX_PATH: usize = crate::browser::MAX_ADDRESS;

/// One row of history: one address on one day.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HistoryRow {
    /// The number the panel asks for it by.
    pub number: u32,
    /// The day's heading, on the first row of each day shown.
    pub heading: Option<String>,
    /// The last visit's time of day, as `14:05`.
    pub time: String,
    /// The channel's title.
    pub channel: String,
    /// The page, when the address names one.
    pub page: Option<u16>,
    /// The page's title, or the channel's.
    pub title: String,
    /// The address as the address rules print it, cut to
    /// [`MAX_SHOWN_ADDRESS`].
    pub address: String,
    /// The address is bookmarked: its menu offers to remove the bookmark,
    /// as Chrome's does (decision 64).
    pub bookmarked: bool,
}

/// A recently closed tab, as the panel and menu list it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClosedTab {
    /// The number it is reopened by.
    pub number: u32,
    /// Its title.
    pub title: String,
}

/// The history panel's view (section 2.4).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HistoryView {
    /// The search the rows answer; empty for all.
    pub query: String,
    /// The place of the first row among all that answer.
    pub first: u32,
    /// How many rows answer.
    pub total: u32,
    /// At most [`MAX_VIEW_ROWS`], newest first.
    pub rows: Vec<HistoryRow>,
    /// Recently closed tabs, the most recent first.
    pub closed: Vec<ClosedTab>,
    /// What the panel says: the result of a removal, a refusal.
    pub notice: String,
}

/// The time a clearing removes (section 2.5).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClearRange {
    /// The last 15 minutes, as Chrome offers (`browsing_data::TimePeriod::LAST_15_MINUTES`).
    FifteenMinutes,
    /// The last hour.
    Hour,
    /// The last 24 hours.
    Day,
    /// The last 7 days.
    Week,
    /// The last 4 weeks.
    FourWeeks,
    /// Everything.
    All,
}

/// What the history panel asks for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HistoryRequest {
    /// The panel opened or closed: the trusted core sends views while open.
    Panel(bool),
    /// The rows that answer `text`, from the `first`.
    Query {
        /// The search; empty for all.
        text: String,
        /// The first row wanted.
        first: u32,
    },
    /// Open a row's address, in the active tab or a new one.
    Open {
        /// The row's number.
        number: u32,
        /// In a new tab.
        new_tab: bool,
    },
    /// Remove these rows.
    Remove(Vec<u32>),
    /// Search for this row's host, as Chrome's "More from this site".
    MoreFromSite(u32),
    /// Remove the bookmarks of this row's address, as Chrome's "Remove
    /// bookmark".
    RemoveBookmark(u32),
    /// Remove the rows of a time.
    Clear(ClearRange),
    /// Remove the cookies made in a time (requirement 21.8).
    ClearCookies(ClearRange),
    /// Reopen a closed tab by its number, or the most recent.
    Reopen(Option<u32>),
}

/// One suggestion under the address field.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Suggestion {
    /// The number it is chosen by.
    pub number: u32,
    /// The address as the address rules print it, cut to
    /// [`MAX_SHOWN_ADDRESS`].
    pub address: String,
    /// Its bookmark's name, or its page's or channel's title.
    pub title: String,
    /// It is a bookmark.
    pub bookmarked: bool,
}

/// The suggestions for what is typed (section 2.10).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Suggestions {
    /// The text they answer.
    pub text: String,
    /// An address the user typed before that starts with the text, to
    /// complete the field with; empty for none.
    pub inline: String,
    /// At most [`MAX_SUGGESTIONS`].
    pub rows: Vec<Suggestion>,
}

/// One entry of a Back or Forward list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TabEntry {
    /// How many steps away, from 1.
    pub steps: u8,
    /// The page, when the address names one.
    pub page: Option<u16>,
    /// Its title.
    pub title: String,
    /// Its address, cut to [`MAX_SHOWN_ADDRESS`].
    pub address: String,
}

/// The active tab's entries one way (section 2.9).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TabList {
    /// Forward, or back.
    pub forward: bool,
    /// At most [`MAX_TAB_LIST`], the nearest first.
    pub entries: Vec<TabEntry>,
    /// When the list is full, at most [`MAX_CHAPTER_STOPS`] entries beyond
    /// it, each where the host changes, as Chrome's chapter stops.
    pub chapters: Vec<TabEntry>,
}

/// A folder as a list or tree shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FolderRow {
    /// The number it is asked for by.
    pub number: u32,
    /// Its name.
    pub name: String,
    /// How deep it is: the two that are always there are 0.
    pub depth: u8,
}

/// A bookmark or folder in a folder's contents or a search's results.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ItemRow {
    /// The number it is asked for by.
    pub number: u32,
    /// A folder, not a bookmark.
    pub folder: bool,
    /// Its name.
    pub name: String,
    /// A bookmark's address, cut to [`MAX_SHOWN_ADDRESS`]; empty for a folder.
    pub address: String,
    /// In a search's results, the folder it is in; else empty.
    pub place: String,
}

/// The bookmarks manager's view (section 3.3).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BookmarksView {
    /// The open folder and those it is in, outermost first.
    pub path: Vec<FolderRow>,
    /// The folder tree, from `folders_first`.
    pub folders: Vec<FolderRow>,
    /// The place of the first folder shown.
    pub folders_first: u32,
    /// How many folders there are.
    pub folders_total: u32,
    /// The search the items answer; empty for the open folder's contents.
    pub query: String,
    /// The open folder's contents or the search's results, from `items_first`.
    pub items: Vec<ItemRow>,
    /// The place of the first item shown.
    pub items_first: u32,
    /// How many items there are.
    pub items_total: u32,
    /// What Undo would put back, as "Removed …"; empty for nothing.
    pub undo: String,
    /// What the manager says: an import's or export's result, a refusal.
    pub notice: String,
}

/// What the bookmarks manager asks for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BookmarkRequest {
    /// The manager opened or closed.
    Panel(bool),
    /// Open this folder.
    Folder(u32),
    /// Show the folders and items from these places.
    Scroll {
        /// The first folder wanted.
        folders_first: u32,
        /// The first item wanted.
        items_first: u32,
    },
    /// Search every bookmark's name and address; empty for the open folder.
    Query(String),
    /// Open a bookmark, in the active tab or a new one.
    Open {
        /// The bookmark's number.
        number: u32,
        /// In a new tab.
        new_tab: bool,
    },
    /// A new name, and for a bookmark a new address, as typed.
    Edit {
        /// The bookmark's or folder's number.
        number: u32,
        /// The name.
        name: String,
        /// The address; empty for a folder.
        address: String,
    },
    /// Move a bookmark or folder into a folder, before the item at `index`.
    Move {
        /// What moves.
        number: u32,
        /// The folder it moves into.
        folder: u32,
        /// Where in it.
        index: u32,
    },
    /// A new folder in the open folder, by this name.
    NewFolder(String),
    /// A new bookmark in the open folder, as typed.
    Add {
        /// Its name.
        name: String,
        /// Its address.
        address: String,
    },
    /// Remove a bookmark or folder.
    Remove(u32),
    /// Put back what was last removed.
    Undo,
    /// Sort the open folder by name.
    Sort,
    /// Read bookmarks from the file at this path: one this browser wrote,
    /// or another browser's Netscape bookmark file (decision 71).
    Import(String),
    /// Write the bookmarks to a file beside the browser's own.
    Export,
}

/// The star's popover (section 3.2).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StarView {
    /// Whether the popover is open.
    pub open: bool,
    /// The bookmark's name.
    pub name: String,
    /// The folder it is in.
    pub folder: u32,
    /// The folders it can be put in, at most [`MAX_VIEW_ROWS`].
    pub folders: Vec<FolderRow>,
    /// Its address names a page, so "The whole channel" is offered.
    pub page: bool,
}

/// What the star's popover asks for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StarRequest {
    /// A new name, and a folder, for the bookmark the popover shows.
    Edit {
        /// The name.
        name: String,
        /// The folder's number.
        folder: u32,
    },
    /// Keep the whole channel: drop the page from the address.
    WholeChannel,
    /// Remove the bookmark.
    Remove,
    /// Close the popover.
    Done,
}

/// The bookmarks bar (section 3.4).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BarView {
    /// Whether the bar is shown.
    pub shown: bool,
    /// The Bookmarks bar folder's contents, at most [`MAX_VIEW_ROWS`].
    pub items: Vec<ItemRow>,
    /// A folder of the bar opened as a list: its number and contents.
    pub open: Option<(u32, Vec<ItemRow>)>,
}

/// What the bar asks for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BarRequest {
    /// Show the bar, or hide it.
    Show(bool),
    /// Open a folder of the bar as a list, or close the list.
    Folder(Option<u32>),
    /// Open a bookmark, in the active tab or a new one.
    Open {
        /// The bookmark's number.
        number: u32,
        /// In a new tab.
        new_tab: bool,
    },
}

/// The furthest a window's corner is kept from the origin, in logical
/// points (`history-and-bookmarks.md` section 15.1).
pub const MAX_COORDINATE: i32 = 32_768;
/// The largest window kept, in logical points.
pub const MAX_WINDOW_SIZE: u32 = 32_768;

/// The window's place and size, in logical points, as the session keeps
/// it (requirement 20.21).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WindowGeometry {
    /// Its outer left edge.
    pub x: i32,
    /// Its outer top edge.
    pub y: i32,
    /// Its inner width.
    pub width: u32,
    /// Its inner height.
    pub height: u32,
    /// Maximised.
    pub maximised: bool,
}

impl WindowGeometry {
    /// Whether every field is within its bound.
    #[must_use]
    pub fn is_plausible(&self) -> bool {
        (-MAX_COORDINATE..=MAX_COORDINATE).contains(&self.x)
            && (-MAX_COORDINATE..=MAX_COORDINATE).contains(&self.y)
            && (1..=MAX_WINDOW_SIZE).contains(&self.width)
            && (1..=MAX_WINDOW_SIZE).contains(&self.height)
    }
}
