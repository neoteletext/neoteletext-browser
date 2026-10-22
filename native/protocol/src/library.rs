//! History and bookmarks between the trusted core and the interface
//! (`history-and-bookmarks.md` section 5): the views the trusted core
//! sends, each bounded so that one always fits a message, and the requests
//! the interface makes, which are strip input. A row is named only by the
//! number its view gave it (requirement 20.5).

use super::{Fields, MAX_ADDRESS, MAX_PAGE_TITLE, ProtocolError, push_text};
use interface_core::channel::page_index;

pub use interface_core::library::{
    BarRequest, BarView, BookmarkRequest, BookmarksView, ClearRange, ClosedTab, FolderRow, HistoryRequest, HistoryRow, HistoryView, ItemRow, MAX_CHAPTER_STOPS,
    MAX_CLOSED, MAX_FOLDER_DEPTH, MAX_LABEL, MAX_NAME, MAX_NOTICE, MAX_PATH, MAX_QUERY, MAX_SHOWN_ADDRESS, MAX_STEPS, MAX_SUGGESTIONS, MAX_TAB_LIST,
    MAX_VIEW_ROWS, StarRequest, StarView, Suggestion, Suggestions, TabEntry, TabList, WindowGeometry,
};

const HISTORY_PANEL: u8 = 1;
const HISTORY_QUERY: u8 = 2;
const HISTORY_OPEN: u8 = 3;
const HISTORY_REMOVE: u8 = 4;
// 5 was "Forget this channel", removed by decision 65; never reused.
const HISTORY_CLEAR: u8 = 6;
const HISTORY_REOPEN: u8 = 7;
const HISTORY_CLEAR_COOKIES: u8 = 8;
const HISTORY_MORE_FROM_SITE: u8 = 9;

const BOOKMARKS_PANEL: u8 = 1;
const BOOKMARKS_FOLDER: u8 = 2;
const BOOKMARKS_SCROLL: u8 = 3;
const BOOKMARKS_QUERY: u8 = 4;
const BOOKMARKS_OPEN: u8 = 5;
const BOOKMARKS_EDIT: u8 = 6;
const BOOKMARKS_MOVE: u8 = 7;
const BOOKMARKS_NEW_FOLDER: u8 = 8;
const BOOKMARKS_ADD: u8 = 9;
const BOOKMARKS_REMOVE: u8 = 10;
const BOOKMARKS_UNDO: u8 = 11;
const BOOKMARKS_SORT: u8 = 12;
const BOOKMARKS_IMPORT: u8 = 13;
const BOOKMARKS_EXPORT: u8 = 14;

const STAR_EDIT: u8 = 1;
const STAR_WHOLE_CHANNEL: u8 = 2;
const STAR_REMOVE: u8 = 3;
const STAR_DONE: u8 = 4;

const BAR_SHOW: u8 = 1;
const BAR_FOLDER: u8 = 2;
const BAR_OPEN: u8 = 3;

/// No page: a page number is never 0.
const NO_PAGE: u16 = 0;

fn push_flag(payload: &mut Vec<u8>, flag: bool) {
    payload.push(u8::from(flag));
}

fn read_flag(fields: &mut Fields<'_>) -> Result<bool, ProtocolError> {
    match fields.u8()? {
        0 => Ok(false),
        1 => Ok(true),
        _ => Err(ProtocolError::BadValue("flag")),
    }
}

fn push_count(payload: &mut Vec<u8>, count: usize, max: usize) -> Result<(), ProtocolError> {
    if count > max {
        return Err(ProtocolError::BadValue("too many rows"));
    }
    payload.extend_from_slice(&u16::try_from(count).map_err(|_| ProtocolError::BadValue("too many rows"))?.to_be_bytes());
    Ok(())
}

fn read_count(fields: &mut Fields<'_>, max: usize) -> Result<usize, ProtocolError> {
    let count = usize::from(fields.u16()?);
    if count > max { Err(ProtocolError::BadValue("too many rows")) } else { Ok(count) }
}

fn push_page(payload: &mut Vec<u8>, page: Option<u16>) -> Result<(), ProtocolError> {
    match page {
        Some(page) if page_index(page).is_some() => payload.extend_from_slice(&page.to_be_bytes()),
        Some(_) => return Err(ProtocolError::BadValue("page number")),
        None => payload.extend_from_slice(&NO_PAGE.to_be_bytes()),
    }
    Ok(())
}

fn read_page(fields: &mut Fields<'_>) -> Result<Option<u16>, ProtocolError> {
    match fields.u16()? {
        NO_PAGE => Ok(None),
        page if page_index(page).is_some() => Ok(Some(page)),
        _ => Err(ProtocolError::BadValue("page number")),
    }
}

fn push_numbers(payload: &mut Vec<u8>, numbers: &[u32]) -> Result<(), ProtocolError> {
    push_count(payload, numbers.len(), MAX_VIEW_ROWS)?;
    for number in numbers {
        payload.extend_from_slice(&number.to_be_bytes());
    }
    Ok(())
}

fn read_numbers(fields: &mut Fields<'_>) -> Result<Vec<u32>, ProtocolError> {
    let count = read_count(fields, MAX_VIEW_ROWS)?;
    (0..count).map(|_| fields.u32()).collect()
}

fn push_folder(payload: &mut Vec<u8>, folder: &FolderRow) -> Result<(), ProtocolError> {
    if usize::from(folder.depth) > MAX_FOLDER_DEPTH {
        return Err(ProtocolError::BadValue("folder depth"));
    }
    payload.extend_from_slice(&folder.number.to_be_bytes());
    payload.push(folder.depth);
    push_text(payload, &folder.name, MAX_NAME)
}

fn read_folder(fields: &mut Fields<'_>) -> Result<FolderRow, ProtocolError> {
    let number = fields.u32()?;
    let depth = fields.u8()?;
    if usize::from(depth) > MAX_FOLDER_DEPTH {
        return Err(ProtocolError::BadValue("folder depth"));
    }
    Ok(FolderRow { number, depth, name: fields.text(MAX_NAME)? })
}

fn push_folders(payload: &mut Vec<u8>, folders: &[FolderRow], max: usize) -> Result<(), ProtocolError> {
    push_count(payload, folders.len(), max)?;
    folders.iter().try_for_each(|folder| push_folder(payload, folder))
}

fn read_folders(fields: &mut Fields<'_>, max: usize) -> Result<Vec<FolderRow>, ProtocolError> {
    let count = read_count(fields, max)?;
    (0..count).map(|_| read_folder(fields)).collect()
}

fn push_item(payload: &mut Vec<u8>, item: &ItemRow) -> Result<(), ProtocolError> {
    payload.extend_from_slice(&item.number.to_be_bytes());
    push_flag(payload, item.folder);
    push_text(payload, &item.name, MAX_NAME)?;
    push_text(payload, &item.address, MAX_SHOWN_ADDRESS)?;
    push_text(payload, &item.place, MAX_LABEL)
}

fn read_item(fields: &mut Fields<'_>) -> Result<ItemRow, ProtocolError> {
    Ok(ItemRow {
        number: fields.u32()?,
        folder: read_flag(fields)?,
        name: fields.text(MAX_NAME)?,
        address: fields.text(MAX_SHOWN_ADDRESS)?,
        place: fields.text(MAX_LABEL)?,
    })
}

fn push_items(payload: &mut Vec<u8>, items: &[ItemRow]) -> Result<(), ProtocolError> {
    push_count(payload, items.len(), MAX_VIEW_ROWS)?;
    items.iter().try_for_each(|item| push_item(payload, item))
}

fn read_items(fields: &mut Fields<'_>) -> Result<Vec<ItemRow>, ProtocolError> {
    let count = read_count(fields, MAX_VIEW_ROWS)?;
    (0..count).map(|_| read_item(fields)).collect()
}

fn push_optional(payload: &mut Vec<u8>, text: Option<&str>, max: usize) -> Result<(), ProtocolError> {
    push_flag(payload, text.is_some());
    if let Some(text) = text { push_text(payload, text, max) } else { Ok(()) }
}

fn read_optional(fields: &mut Fields<'_>, max: usize) -> Result<Option<String>, ProtocolError> {
    if read_flag(fields)? { Ok(Some(fields.text(max)?)) } else { Ok(None) }
}

fn push_range(range: ClearRange) -> u8 {
    match range {
        ClearRange::Hour => 1,
        ClearRange::Day => 2,
        ClearRange::Week => 3,
        ClearRange::FourWeeks => 4,
        ClearRange::All => 5,
        ClearRange::FifteenMinutes => 6,
    }
}

fn read_range(fields: &mut Fields<'_>) -> Result<ClearRange, ProtocolError> {
    Ok(match fields.u8()? {
        1 => ClearRange::Hour,
        2 => ClearRange::Day,
        3 => ClearRange::Week,
        4 => ClearRange::FourWeeks,
        5 => ClearRange::All,
        6 => ClearRange::FifteenMinutes,
        _ => return Err(ProtocolError::BadValue("clear range")),
    })
}

pub(crate) fn push_history_view(payload: &mut Vec<u8>, view: &HistoryView) -> Result<(), ProtocolError> {
    push_text(payload, &view.query, MAX_QUERY)?;
    payload.extend_from_slice(&view.first.to_be_bytes());
    payload.extend_from_slice(&view.total.to_be_bytes());
    push_count(payload, view.rows.len(), MAX_VIEW_ROWS)?;
    for row in &view.rows {
        payload.extend_from_slice(&row.number.to_be_bytes());
        push_optional(payload, row.heading.as_deref(), MAX_LABEL)?;
        push_text(payload, &row.time, MAX_LABEL)?;
        push_text(payload, &row.channel, MAX_NAME)?;
        push_page(payload, row.page)?;
        push_text(payload, &row.title, MAX_PAGE_TITLE.max(MAX_NAME))?;
        push_text(payload, &row.address, MAX_SHOWN_ADDRESS)?;
    }
    push_count(payload, view.closed.len(), MAX_CLOSED)?;
    for closed in &view.closed {
        payload.extend_from_slice(&closed.number.to_be_bytes());
        push_text(payload, &closed.title, MAX_NAME)?;
    }
    push_text(payload, &view.notice, MAX_NOTICE)
}

pub(crate) fn read_history_view(fields: &mut Fields<'_>) -> Result<HistoryView, ProtocolError> {
    let query = fields.text(MAX_QUERY)?;
    let first = fields.u32()?;
    let total = fields.u32()?;
    let count = read_count(fields, MAX_VIEW_ROWS)?;
    let rows = (0..count)
        .map(|_| {
            Ok(HistoryRow {
                number: fields.u32()?,
                heading: read_optional(fields, MAX_LABEL)?,
                time: fields.text(MAX_LABEL)?,
                channel: fields.text(MAX_NAME)?,
                page: read_page(fields)?,
                title: fields.text(MAX_PAGE_TITLE.max(MAX_NAME))?,
                address: fields.text(MAX_SHOWN_ADDRESS)?,
            })
        })
        .collect::<Result<_, ProtocolError>>()?;
    let count = read_count(fields, MAX_CLOSED)?;
    let closed = (0..count).map(|_| Ok(ClosedTab { number: fields.u32()?, title: fields.text(MAX_NAME)? })).collect::<Result<_, ProtocolError>>()?;
    Ok(HistoryView { query, first, total, rows, closed, notice: fields.text(MAX_NOTICE)? })
}

pub(crate) fn push_history_request(payload: &mut Vec<u8>, request: &HistoryRequest) -> Result<(), ProtocolError> {
    match request {
        HistoryRequest::Panel(open) => {
            payload.push(HISTORY_PANEL);
            push_flag(payload, *open);
        }
        HistoryRequest::Query { text, first } => {
            payload.push(HISTORY_QUERY);
            push_text(payload, text, MAX_QUERY)?;
            payload.extend_from_slice(&first.to_be_bytes());
        }
        HistoryRequest::Open { number, new_tab } => {
            payload.push(HISTORY_OPEN);
            payload.extend_from_slice(&number.to_be_bytes());
            push_flag(payload, *new_tab);
        }
        HistoryRequest::Remove(numbers) => {
            payload.push(HISTORY_REMOVE);
            push_numbers(payload, numbers)?;
        }
        HistoryRequest::MoreFromSite(number) => {
            payload.push(HISTORY_MORE_FROM_SITE);
            payload.extend_from_slice(&number.to_be_bytes());
        }
        HistoryRequest::Clear(range) => payload.extend_from_slice(&[HISTORY_CLEAR, push_range(*range)]),
        HistoryRequest::ClearCookies(range) => payload.extend_from_slice(&[HISTORY_CLEAR_COOKIES, push_range(*range)]),
        HistoryRequest::Reopen(number) => {
            payload.push(HISTORY_REOPEN);
            if let Some(number) = number {
                payload.extend_from_slice(&number.to_be_bytes());
            }
        }
    }
    Ok(())
}

pub(crate) fn read_history_request(fields: &mut Fields<'_>) -> Result<HistoryRequest, ProtocolError> {
    Ok(match fields.u8()? {
        HISTORY_PANEL => HistoryRequest::Panel(read_flag(fields)?),
        HISTORY_QUERY => HistoryRequest::Query { text: fields.text(MAX_QUERY)?, first: fields.u32()? },
        HISTORY_OPEN => HistoryRequest::Open { number: fields.u32()?, new_tab: read_flag(fields)? },
        HISTORY_REMOVE => HistoryRequest::Remove(read_numbers(fields)?),
        HISTORY_MORE_FROM_SITE => HistoryRequest::MoreFromSite(fields.u32()?),
        HISTORY_CLEAR => HistoryRequest::Clear(read_range(fields)?),
        HISTORY_CLEAR_COOKIES => HistoryRequest::ClearCookies(read_range(fields)?),
        HISTORY_REOPEN => HistoryRequest::Reopen(if fields.is_empty() { None } else { Some(fields.u32()?) }),
        _ => return Err(ProtocolError::BadValue("history request")),
    })
}

pub(crate) fn push_suggestions(payload: &mut Vec<u8>, suggestions: &Suggestions) -> Result<(), ProtocolError> {
    push_text(payload, &suggestions.text, MAX_ADDRESS)?;
    push_text(payload, &suggestions.inline, MAX_ADDRESS)?;
    push_count(payload, suggestions.rows.len(), MAX_SUGGESTIONS)?;
    for row in &suggestions.rows {
        payload.extend_from_slice(&row.number.to_be_bytes());
        push_text(payload, &row.address, MAX_SHOWN_ADDRESS)?;
        push_text(payload, &row.title, MAX_NAME)?;
        push_flag(payload, row.bookmarked);
    }
    Ok(())
}

pub(crate) fn read_suggestions(fields: &mut Fields<'_>) -> Result<Suggestions, ProtocolError> {
    let text = fields.text(MAX_ADDRESS)?;
    let inline = fields.text(MAX_ADDRESS)?;
    let count = read_count(fields, MAX_SUGGESTIONS)?;
    let rows = (0..count)
        .map(|_| {
            Ok(Suggestion { number: fields.u32()?, address: fields.text(MAX_SHOWN_ADDRESS)?, title: fields.text(MAX_NAME)?, bookmarked: read_flag(fields)? })
        })
        .collect::<Result<_, ProtocolError>>()?;
    Ok(Suggestions { text, inline, rows })
}

pub(crate) fn push_tab_list(payload: &mut Vec<u8>, list: &TabList) -> Result<(), ProtocolError> {
    push_flag(payload, list.forward);
    for (entries, most) in [(&list.entries, MAX_TAB_LIST), (&list.chapters, MAX_CHAPTER_STOPS)] {
        push_count(payload, entries.len(), most)?;
        for entry in entries {
            if entry.steps == 0 || usize::from(entry.steps) > MAX_STEPS {
                return Err(ProtocolError::BadValue("steps"));
            }
            payload.push(entry.steps);
            push_page(payload, entry.page)?;
            push_text(payload, &entry.title, MAX_NAME)?;
            push_text(payload, &entry.address, MAX_SHOWN_ADDRESS)?;
        }
    }
    Ok(())
}

pub(crate) fn read_tab_list(fields: &mut Fields<'_>) -> Result<TabList, ProtocolError> {
    let forward = read_flag(fields)?;
    let mut read = |most: usize| -> Result<Vec<TabEntry>, ProtocolError> {
        let count = read_count(fields, most)?;
        (0..count)
            .map(|_| {
                let steps = read_steps(fields)?;
                Ok(TabEntry { steps, page: read_page(fields)?, title: fields.text(MAX_NAME)?, address: fields.text(MAX_SHOWN_ADDRESS)? })
            })
            .collect()
    };
    let entries = read(MAX_TAB_LIST)?;
    let chapters = read(MAX_CHAPTER_STOPS)?;
    Ok(TabList { forward, entries, chapters })
}

/// How many steps a Back or Forward list's entry is, 1 to [`MAX_STEPS`].
pub(crate) fn read_steps(fields: &mut Fields<'_>) -> Result<u8, ProtocolError> {
    let steps = fields.u8()?;
    if steps == 0 || usize::from(steps) > MAX_STEPS { Err(ProtocolError::BadValue("steps")) } else { Ok(steps) }
}

pub(crate) fn push_bookmarks_view(payload: &mut Vec<u8>, view: &BookmarksView) -> Result<(), ProtocolError> {
    push_folders(payload, &view.path, MAX_FOLDER_DEPTH.saturating_add(1))?;
    push_folders(payload, &view.folders, MAX_VIEW_ROWS)?;
    payload.extend_from_slice(&view.folders_first.to_be_bytes());
    payload.extend_from_slice(&view.folders_total.to_be_bytes());
    push_text(payload, &view.query, MAX_QUERY)?;
    push_items(payload, &view.items)?;
    payload.extend_from_slice(&view.items_first.to_be_bytes());
    payload.extend_from_slice(&view.items_total.to_be_bytes());
    push_text(payload, &view.undo, MAX_NOTICE)?;
    push_text(payload, &view.notice, MAX_NOTICE)
}

pub(crate) fn read_bookmarks_view(fields: &mut Fields<'_>) -> Result<BookmarksView, ProtocolError> {
    Ok(BookmarksView {
        path: read_folders(fields, MAX_FOLDER_DEPTH.saturating_add(1))?,
        folders: read_folders(fields, MAX_VIEW_ROWS)?,
        folders_first: fields.u32()?,
        folders_total: fields.u32()?,
        query: fields.text(MAX_QUERY)?,
        items: read_items(fields)?,
        items_first: fields.u32()?,
        items_total: fields.u32()?,
        undo: fields.text(MAX_NOTICE)?,
        notice: fields.text(MAX_NOTICE)?,
    })
}

pub(crate) fn push_bookmark_request(payload: &mut Vec<u8>, request: &BookmarkRequest) -> Result<(), ProtocolError> {
    match request {
        BookmarkRequest::Panel(open) => {
            payload.push(BOOKMARKS_PANEL);
            push_flag(payload, *open);
        }
        BookmarkRequest::Folder(number) => {
            payload.push(BOOKMARKS_FOLDER);
            payload.extend_from_slice(&number.to_be_bytes());
        }
        BookmarkRequest::Scroll { folders_first, items_first } => {
            payload.push(BOOKMARKS_SCROLL);
            payload.extend_from_slice(&folders_first.to_be_bytes());
            payload.extend_from_slice(&items_first.to_be_bytes());
        }
        BookmarkRequest::Query(text) => {
            payload.push(BOOKMARKS_QUERY);
            push_text(payload, text, MAX_QUERY)?;
        }
        BookmarkRequest::Open { number, new_tab } => {
            payload.push(BOOKMARKS_OPEN);
            payload.extend_from_slice(&number.to_be_bytes());
            push_flag(payload, *new_tab);
        }
        BookmarkRequest::Edit { number, name, address } => {
            payload.push(BOOKMARKS_EDIT);
            payload.extend_from_slice(&number.to_be_bytes());
            push_text(payload, name, MAX_NAME)?;
            push_text(payload, address, MAX_ADDRESS)?;
        }
        BookmarkRequest::Move { number, folder, index } => {
            payload.push(BOOKMARKS_MOVE);
            payload.extend_from_slice(&number.to_be_bytes());
            payload.extend_from_slice(&folder.to_be_bytes());
            payload.extend_from_slice(&index.to_be_bytes());
        }
        BookmarkRequest::NewFolder(name) => {
            payload.push(BOOKMARKS_NEW_FOLDER);
            push_text(payload, name, MAX_NAME)?;
        }
        BookmarkRequest::Add { name, address } => {
            payload.push(BOOKMARKS_ADD);
            push_text(payload, name, MAX_NAME)?;
            push_text(payload, address, MAX_ADDRESS)?;
        }
        BookmarkRequest::Remove(number) => {
            payload.push(BOOKMARKS_REMOVE);
            payload.extend_from_slice(&number.to_be_bytes());
        }
        BookmarkRequest::Undo => payload.push(BOOKMARKS_UNDO),
        BookmarkRequest::Sort => payload.push(BOOKMARKS_SORT),
        BookmarkRequest::Import(path) => {
            payload.push(BOOKMARKS_IMPORT);
            push_text(payload, path, MAX_PATH)?;
        }
        BookmarkRequest::Export => payload.push(BOOKMARKS_EXPORT),
    }
    Ok(())
}

pub(crate) fn read_bookmark_request(fields: &mut Fields<'_>) -> Result<BookmarkRequest, ProtocolError> {
    Ok(match fields.u8()? {
        BOOKMARKS_PANEL => BookmarkRequest::Panel(read_flag(fields)?),
        BOOKMARKS_FOLDER => BookmarkRequest::Folder(fields.u32()?),
        BOOKMARKS_SCROLL => BookmarkRequest::Scroll { folders_first: fields.u32()?, items_first: fields.u32()? },
        BOOKMARKS_QUERY => BookmarkRequest::Query(fields.text(MAX_QUERY)?),
        BOOKMARKS_OPEN => BookmarkRequest::Open { number: fields.u32()?, new_tab: read_flag(fields)? },
        BOOKMARKS_EDIT => BookmarkRequest::Edit { number: fields.u32()?, name: fields.text(MAX_NAME)?, address: fields.text(MAX_ADDRESS)? },
        BOOKMARKS_MOVE => BookmarkRequest::Move { number: fields.u32()?, folder: fields.u32()?, index: fields.u32()? },
        BOOKMARKS_NEW_FOLDER => BookmarkRequest::NewFolder(fields.text(MAX_NAME)?),
        BOOKMARKS_ADD => BookmarkRequest::Add { name: fields.text(MAX_NAME)?, address: fields.text(MAX_ADDRESS)? },
        BOOKMARKS_REMOVE => BookmarkRequest::Remove(fields.u32()?),
        BOOKMARKS_UNDO => BookmarkRequest::Undo,
        BOOKMARKS_SORT => BookmarkRequest::Sort,
        BOOKMARKS_IMPORT => BookmarkRequest::Import(fields.text(MAX_PATH)?),
        BOOKMARKS_EXPORT => BookmarkRequest::Export,
        _ => return Err(ProtocolError::BadValue("bookmark request")),
    })
}

pub(crate) fn push_star_view(payload: &mut Vec<u8>, view: &StarView) -> Result<(), ProtocolError> {
    push_flag(payload, view.open);
    push_text(payload, &view.name, MAX_NAME)?;
    payload.extend_from_slice(&view.folder.to_be_bytes());
    push_folders(payload, &view.folders, MAX_VIEW_ROWS)?;
    push_flag(payload, view.page);
    Ok(())
}

pub(crate) fn read_star_view(fields: &mut Fields<'_>) -> Result<StarView, ProtocolError> {
    Ok(StarView {
        open: read_flag(fields)?,
        name: fields.text(MAX_NAME)?,
        folder: fields.u32()?,
        folders: read_folders(fields, MAX_VIEW_ROWS)?,
        page: read_flag(fields)?,
    })
}

pub(crate) fn push_star_request(payload: &mut Vec<u8>, request: &StarRequest) -> Result<(), ProtocolError> {
    match request {
        StarRequest::Edit { name, folder } => {
            payload.push(STAR_EDIT);
            push_text(payload, name, MAX_NAME)?;
            payload.extend_from_slice(&folder.to_be_bytes());
        }
        StarRequest::WholeChannel => payload.push(STAR_WHOLE_CHANNEL),
        StarRequest::Remove => payload.push(STAR_REMOVE),
        StarRequest::Done => payload.push(STAR_DONE),
    }
    Ok(())
}

pub(crate) fn read_star_request(fields: &mut Fields<'_>) -> Result<StarRequest, ProtocolError> {
    Ok(match fields.u8()? {
        STAR_EDIT => StarRequest::Edit { name: fields.text(MAX_NAME)?, folder: fields.u32()? },
        STAR_WHOLE_CHANNEL => StarRequest::WholeChannel,
        STAR_REMOVE => StarRequest::Remove,
        STAR_DONE => StarRequest::Done,
        _ => return Err(ProtocolError::BadValue("star request")),
    })
}

pub(crate) fn push_bar_view(payload: &mut Vec<u8>, view: &BarView) -> Result<(), ProtocolError> {
    push_flag(payload, view.shown);
    push_items(payload, &view.items)?;
    push_flag(payload, view.open.is_some());
    if let Some((number, items)) = &view.open {
        payload.extend_from_slice(&number.to_be_bytes());
        push_items(payload, items)?;
    }
    Ok(())
}

pub(crate) fn read_bar_view(fields: &mut Fields<'_>) -> Result<BarView, ProtocolError> {
    let shown = read_flag(fields)?;
    let items = read_items(fields)?;
    let open = if read_flag(fields)? { Some((fields.u32()?, read_items(fields)?)) } else { None };
    Ok(BarView { shown, items, open })
}

pub(crate) fn push_bar_request(payload: &mut Vec<u8>, request: &BarRequest) {
    match request {
        BarRequest::Show(shown) => {
            payload.push(BAR_SHOW);
            push_flag(payload, *shown);
        }
        BarRequest::Folder(number) => {
            payload.push(BAR_FOLDER);
            if let Some(number) = number {
                payload.extend_from_slice(&number.to_be_bytes());
            }
        }
        BarRequest::Open { number, new_tab } => {
            payload.push(BAR_OPEN);
            payload.extend_from_slice(&number.to_be_bytes());
            push_flag(payload, *new_tab);
        }
    }
}

pub(crate) fn read_bar_request(fields: &mut Fields<'_>) -> Result<BarRequest, ProtocolError> {
    Ok(match fields.u8()? {
        BAR_SHOW => BarRequest::Show(read_flag(fields)?),
        BAR_FOLDER => BarRequest::Folder(if fields.is_empty() { None } else { Some(fields.u32()?) }),
        BAR_OPEN => BarRequest::Open { number: fields.u32()?, new_tab: read_flag(fields)? },
        _ => return Err(ProtocolError::BadValue("bar request")),
    })
}

pub(crate) fn push_window(payload: &mut Vec<u8>, window: &WindowGeometry) -> Result<(), ProtocolError> {
    if !window.is_plausible() {
        return Err(ProtocolError::BadValue("window"));
    }
    payload.extend_from_slice(&window.x.to_be_bytes());
    payload.extend_from_slice(&window.y.to_be_bytes());
    payload.extend_from_slice(&window.width.to_be_bytes());
    payload.extend_from_slice(&window.height.to_be_bytes());
    push_flag(payload, window.maximised);
    Ok(())
}

pub(crate) fn read_window(fields: &mut Fields<'_>) -> Result<WindowGeometry, ProtocolError> {
    let signed = |fields: &mut Fields<'_>| fields.array::<4>().map(i32::from_be_bytes);
    let window = WindowGeometry { x: signed(fields)?, y: signed(fields)?, width: fields.u32()?, height: fields.u32()?, maximised: read_flag(fields)? };
    if window.is_plausible() { Ok(window) } else { Err(ProtocolError::BadValue("window")) }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, reason = "a test")]
mod tests {
    use super::*;
    use crate::MAX_PAYLOAD;

    fn item(number: u32) -> ItemRow {
        ItemRow {
            number,
            folder: number.is_multiple_of(2),
            name: "News Example".into(),
            address: "https://news.example/teletext/".into(),
            place: "Bookmarks bar".into(),
        }
    }

    fn folder(number: u32, depth: u8) -> FolderRow {
        FolderRow { number, name: "Weather".into(), depth }
    }

    fn history() -> HistoryView {
        HistoryView {
            query: "news".into(),
            first: 0,
            total: 1,
            rows: vec![HistoryRow {
                number: 7,
                heading: Some("Today: Wednesday 22 October 2026".into()),
                time: "14:05".into(),
                channel: "News Example".into(),
                page: Some(0x302),
                title: "WEATHER".into(),
                address: "https://news.example/teletext/#302".into(),
            }],
            closed: vec![ClosedTab { number: 3, title: "Sport".into() }],
            notice: String::new(),
        }
    }

    fn round_trip<T: PartialEq + std::fmt::Debug>(
        value: &T,
        push: fn(&mut Vec<u8>, &T) -> Result<(), ProtocolError>,
        read: fn(&mut Fields<'_>) -> Result<T, ProtocolError>,
    ) {
        let mut payload = Vec::new();
        push(&mut payload, value).unwrap();
        let mut fields = Fields { bytes: &payload };
        assert_eq!(&read(&mut fields).unwrap(), value);
        fields.end().unwrap();
    }

    /// Every view and request survives the pipe.
    #[test]
    fn every_view_and_request_round_trips() {
        round_trip(&history(), push_history_view, read_history_view);
        round_trip(
            &Suggestions {
                text: "news".into(),
                inline: "news.example/teletext/".into(),
                rows: vec![Suggestion { number: 1, address: "https://news.example/".into(), title: "News".into(), bookmarked: true }],
            },
            push_suggestions,
            read_suggestions,
        );
        round_trip(
            &TabList {
                forward: false,
                entries: vec![TabEntry { steps: 2, page: None, title: "Sport".into(), address: "https://sport.example/".into() }],
                chapters: vec![TabEntry { steps: 30, page: Some(0x100), title: "News".into(), address: "https://news.example/".into() }],
            },
            push_tab_list,
            read_tab_list,
        );
        round_trip(
            &BookmarksView {
                path: vec![folder(1, 0), folder(4, 1)],
                folders: vec![folder(1, 0), folder(2, 0), folder(4, 1)],
                folders_first: 0,
                folders_total: 3,
                query: String::new(),
                items: vec![item(5), item(6)],
                items_first: 0,
                items_total: 2,
                undo: "Removed News".into(),
                notice: "Exported".into(),
            },
            push_bookmarks_view,
            read_bookmarks_view,
        );
        round_trip(
            &StarView { open: true, name: "News".into(), folder: 2, folders: vec![folder(1, 0), folder(2, 0)], page: true },
            push_star_view,
            read_star_view,
        );
        round_trip(&BarView { shown: true, items: vec![item(1)], open: Some((2, vec![item(3)])) }, push_bar_view, read_bar_view);
        for request in [
            HistoryRequest::Panel(true),
            HistoryRequest::Query { text: "x".into(), first: 100 },
            HistoryRequest::Open { number: 4, new_tab: true },
            HistoryRequest::Remove(vec![1, 2, 3]),
            HistoryRequest::MoreFromSite(9),
            HistoryRequest::Clear(ClearRange::FourWeeks),
            HistoryRequest::ClearCookies(ClearRange::Hour),
            HistoryRequest::Clear(ClearRange::FifteenMinutes),
            HistoryRequest::Reopen(None),
            HistoryRequest::Reopen(Some(3)),
        ] {
            round_trip(&request, push_history_request, read_history_request);
        }
        for request in [
            BookmarkRequest::Panel(false),
            BookmarkRequest::Folder(4),
            BookmarkRequest::Scroll { folders_first: 100, items_first: 200 },
            BookmarkRequest::Query("news".into()),
            BookmarkRequest::Open { number: 5, new_tab: false },
            BookmarkRequest::Edit { number: 5, name: "News".into(), address: "news.example".into() },
            BookmarkRequest::Move { number: 5, folder: 1, index: 0 },
            BookmarkRequest::NewFolder("Weather".into()),
            BookmarkRequest::Add { name: "Sport".into(), address: "sport.example".into() },
            BookmarkRequest::Remove(5),
            BookmarkRequest::Undo,
            BookmarkRequest::Sort,
            BookmarkRequest::Import("/home/user/bookmarks.txt".into()),
            BookmarkRequest::Export,
        ] {
            round_trip(&request, push_bookmark_request, read_bookmark_request);
        }
        for request in [StarRequest::Edit { name: "News".into(), folder: 2 }, StarRequest::WholeChannel, StarRequest::Remove, StarRequest::Done] {
            round_trip(&request, push_star_request, read_star_request);
        }
        round_trip(&WindowGeometry { x: -120, y: 40, width: 1440, height: 900, maximised: true }, push_window, read_window);
        assert!(push_window(&mut Vec::new(), &WindowGeometry { x: 0, y: 0, width: 0, height: 900, maximised: false }).is_err());
        assert!(push_window(&mut Vec::new(), &WindowGeometry { x: 40_000, y: 0, width: 10, height: 900, maximised: false }).is_err());
        for request in [BarRequest::Show(true), BarRequest::Folder(None), BarRequest::Folder(Some(4)), BarRequest::Open { number: 1, new_tab: true }] {
            let mut payload = Vec::new();
            push_bar_request(&mut payload, &request);
            assert_eq!(read_bar_request(&mut Fields { bytes: &payload }).unwrap(), request);
        }
    }

    /// The bounds hold both ways: too many rows, too deep a folder, a page
    /// that is no page, text that was not cleaned, an unknown request.
    #[test]
    fn what_breaks_a_bound_is_refused() {
        let mut view = history();
        view.rows = vec![view.rows[0].clone(); MAX_VIEW_ROWS + 1];
        assert!(push_history_view(&mut Vec::new(), &view).is_err());
        let mut view = history();
        view.rows[0].title = "a\u{202E}b".into();
        assert!(push_history_view(&mut Vec::new(), &view).is_err(), "hidden characters");
        let mut view = history();
        view.rows[0].page = Some(0x0FF);
        assert!(push_history_view(&mut Vec::new(), &view).is_err());
        assert!(push_folder(&mut Vec::new(), &folder(1, u8::try_from(MAX_FOLDER_DEPTH + 1).unwrap())).is_err());
        assert!(read_history_request(&mut Fields { bytes: &[HISTORY_CLEAR, 7] }).is_err(), "no seventh range");
        assert!(read_history_request(&mut Fields { bytes: &[99] }).is_err());
        assert!(read_bookmark_request(&mut Fields { bytes: &[99] }).is_err());
        assert!(read_star_request(&mut Fields { bytes: &[99] }).is_err());
        assert!(read_bar_request(&mut Fields { bytes: &[BAR_SHOW, 2] }).is_err());
        let too_many = u16::try_from(MAX_VIEW_ROWS + 1).unwrap().to_be_bytes();
        assert!(read_history_request(&mut Fields { bytes: &[HISTORY_REMOVE, too_many[0], too_many[1]] }).is_err());
        assert!(read_tab_list(&mut Fields { bytes: &[0, 0, 1, 0, 0, 0] }).is_err(), "an entry no steps away");
    }

    /// Section 2.4: a whole view of the longest rows fits one message.
    #[test]
    fn the_largest_views_fit_one_message() {
        let long = |bytes: usize| "x".repeat(bytes);
        let row = HistoryRow {
            number: u32::MAX,
            heading: Some(long(MAX_LABEL)),
            time: long(MAX_LABEL),
            channel: long(MAX_NAME),
            page: Some(0x899),
            title: long(MAX_NAME),
            address: long(MAX_SHOWN_ADDRESS),
        };
        let view = HistoryView {
            query: long(MAX_QUERY),
            first: 0,
            total: u32::MAX,
            rows: vec![row; MAX_VIEW_ROWS],
            closed: vec![ClosedTab { number: 1, title: long(MAX_NAME) }; MAX_CLOSED],
            notice: long(MAX_NOTICE),
        };
        let mut payload = Vec::new();
        push_history_view(&mut payload, &view).unwrap();
        assert!(payload.len() <= MAX_PAYLOAD, "{}", payload.len());
        let item = ItemRow { number: 1, folder: false, name: long(MAX_NAME), address: long(MAX_SHOWN_ADDRESS), place: long(MAX_LABEL) };
        let folders = vec![FolderRow { number: 1, name: long(MAX_NAME), depth: 1 }; MAX_VIEW_ROWS];
        let view = BookmarksView {
            path: vec![FolderRow { number: 1, name: long(MAX_NAME), depth: 1 }; MAX_FOLDER_DEPTH + 1],
            folders,
            folders_first: 0,
            folders_total: 0,
            query: long(MAX_QUERY),
            items: vec![item.clone(); MAX_VIEW_ROWS],
            items_first: 0,
            items_total: 0,
            undo: long(MAX_NOTICE),
            notice: long(MAX_NOTICE),
        };
        let mut payload = Vec::new();
        push_bookmarks_view(&mut payload, &view).unwrap();
        assert!(payload.len() <= MAX_PAYLOAD, "{}", payload.len());
        let bar = BarView { shown: true, items: vec![item.clone(); MAX_VIEW_ROWS], open: Some((1, vec![item; MAX_VIEW_ROWS])) };
        let mut payload = Vec::new();
        push_bar_view(&mut payload, &bar).unwrap();
        assert!(payload.len() <= MAX_PAYLOAD, "{}", payload.len());
    }
}
