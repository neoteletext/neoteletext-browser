//! History and bookmarks between the trusted core and the interface
//! (`history-and-bookmarks.md` section 5): the views the trusted core
//! sends, each bounded so that one always fits a message, and the requests
//! the interface makes, which are strip input. A row is named only by the
//! number its view gave it (requirement 20.5).

mod bookmarks;
mod history;
mod lists;
#[cfg(test)]
mod tests;

use interface_core::channel::page_index;

use crate::{Fields, ProtocolError, push_count, push_flag, push_text};

pub(crate) use bookmarks::{
    push_bar_request, push_bar_view, push_bookmark_request, push_bookmarks_view, push_star_request, push_star_view, read_bar_request, read_bar_view,
    read_bookmark_request, read_bookmarks_view, read_star_request, read_star_view,
};
pub(crate) use history::{push_history_request, push_history_view, read_history_request, read_history_view};
pub(crate) use lists::{push_suggestions, push_tab_list, read_steps, read_suggestions, read_tab_list};

pub use interface_core::library::{
    BarRequest, BarView, BookmarkRequest, BookmarksView, ClearRange, ClosedTab, FolderRow, HistoryRequest, HistoryRow, HistoryView, ItemRow, MAX_CHAPTER_STOPS,
    MAX_CLOSED, MAX_FOLDER_DEPTH, MAX_LABEL, MAX_NAME, MAX_NOTICE, MAX_PATH, MAX_QUERY, MAX_SHOWN_ADDRESS, MAX_STEPS, MAX_SUGGESTIONS, MAX_TAB_LIST,
    MAX_VIEW_ROWS, StarRequest, StarView, Suggestion, Suggestions, TabEntry, TabList, WindowGeometry,
};

/// No page: a page number is never 0.
const NO_PAGE: u16 = 0;

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
    push_count(payload, numbers.len(), MAX_VIEW_ROWS, "too many rows")?;
    for number in numbers {
        payload.extend_from_slice(&number.to_be_bytes());
    }
    Ok(())
}

fn read_numbers(fields: &mut Fields<'_>) -> Result<Vec<u32>, ProtocolError> {
    let count = fields.count_u16(MAX_VIEW_ROWS, "too many rows")?;
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
    push_count(payload, folders.len(), max, "too many rows")?;
    folders.iter().try_for_each(|folder| push_folder(payload, folder))
}

fn read_folders(fields: &mut Fields<'_>, max: usize) -> Result<Vec<FolderRow>, ProtocolError> {
    let count = fields.count_u16(max, "too many rows")?;
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
        folder: fields.flag("flag")?,
        name: fields.text(MAX_NAME)?,
        address: fields.text(MAX_SHOWN_ADDRESS)?,
        place: fields.text(MAX_LABEL)?,
    })
}

fn push_items(payload: &mut Vec<u8>, items: &[ItemRow]) -> Result<(), ProtocolError> {
    push_count(payload, items.len(), MAX_VIEW_ROWS, "too many rows")?;
    items.iter().try_for_each(|item| push_item(payload, item))
}

fn read_items(fields: &mut Fields<'_>) -> Result<Vec<ItemRow>, ProtocolError> {
    let count = fields.count_u16(MAX_VIEW_ROWS, "too many rows")?;
    (0..count).map(|_| read_item(fields)).collect()
}

fn push_optional(payload: &mut Vec<u8>, text: Option<&str>, max: usize) -> Result<(), ProtocolError> {
    push_flag(payload, text.is_some());
    if let Some(text) = text { push_text(payload, text, max) } else { Ok(()) }
}

fn read_optional(fields: &mut Fields<'_>, max: usize) -> Result<Option<String>, ProtocolError> {
    if fields.flag("flag")? { Ok(Some(fields.text(max)?)) } else { Ok(None) }
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
    let window = WindowGeometry { x: signed(fields)?, y: signed(fields)?, width: fields.u32()?, height: fields.u32()?, maximised: fields.flag("flag")? };
    if window.is_plausible() { Ok(window) } else { Err(ProtocolError::BadValue("window")) }
}
