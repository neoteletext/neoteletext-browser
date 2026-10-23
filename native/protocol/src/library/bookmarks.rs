//! The bookmarks manager, the star's popover and the bookmarks bar on the
//! wire (`history-and-bookmarks.md` section 5): the trusted core's views
//! and the interface's requests.

use super::{
    BarRequest, BarView, BookmarkRequest, BookmarksView, MAX_FOLDER_DEPTH, MAX_NAME, MAX_NOTICE, MAX_PATH, MAX_QUERY, MAX_VIEW_ROWS, StarRequest, StarView,
    push_folders, push_items, read_folders, read_items,
};
use crate::{Fields, MAX_ADDRESS, ProtocolError, push_flag, push_text};

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

pub(super) const BAR_SHOW: u8 = 1;
const BAR_FOLDER: u8 = 2;
const BAR_OPEN: u8 = 3;

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
        BOOKMARKS_PANEL => BookmarkRequest::Panel(fields.flag("flag")?),
        BOOKMARKS_FOLDER => BookmarkRequest::Folder(fields.u32()?),
        BOOKMARKS_SCROLL => BookmarkRequest::Scroll { folders_first: fields.u32()?, items_first: fields.u32()? },
        BOOKMARKS_QUERY => BookmarkRequest::Query(fields.text(MAX_QUERY)?),
        BOOKMARKS_OPEN => BookmarkRequest::Open { number: fields.u32()?, new_tab: fields.flag("flag")? },
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
        open: fields.flag("flag")?,
        name: fields.text(MAX_NAME)?,
        folder: fields.u32()?,
        folders: read_folders(fields, MAX_VIEW_ROWS)?,
        page: fields.flag("flag")?,
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
    let shown = fields.flag("flag")?;
    let items = read_items(fields)?;
    let open = if fields.flag("flag")? { Some((fields.u32()?, read_items(fields)?)) } else { None };
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
        BAR_SHOW => BarRequest::Show(fields.flag("flag")?),
        BAR_FOLDER => BarRequest::Folder(if fields.is_empty() { None } else { Some(fields.u32()?) }),
        BAR_OPEN => BarRequest::Open { number: fields.u32()?, new_tab: fields.flag("flag")? },
        _ => return Err(ProtocolError::BadValue("bar request")),
    })
}
