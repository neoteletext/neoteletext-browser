//! The history panel on the wire (`history-and-bookmarks.md` section 5):
//! the trusted core's view of the history, and the panel's requests.

use super::{
    ClosedTab, HistoryRequest, HistoryRow, HistoryView, MAX_CLOSED, MAX_LABEL, MAX_NAME, MAX_NOTICE, MAX_QUERY, MAX_SHOWN_ADDRESS, MAX_VIEW_ROWS, push_numbers,
    push_optional, push_page, push_range, read_numbers, read_optional, read_page, read_range,
};
use crate::{Fields, MAX_PAGE_TITLE, ProtocolError, push_count, push_flag, push_text};

const HISTORY_PANEL: u8 = 1;
const HISTORY_QUERY: u8 = 2;
const HISTORY_OPEN: u8 = 3;
pub(super) const HISTORY_REMOVE: u8 = 4;
// 5 was "Forget this channel", removed by decision 65; never reused.
pub(super) const HISTORY_CLEAR: u8 = 6;
const HISTORY_REOPEN: u8 = 7;
const HISTORY_CLEAR_COOKIES: u8 = 8;
const HISTORY_MORE_FROM_SITE: u8 = 9;
const HISTORY_REMOVE_BOOKMARK: u8 = 10;

pub(crate) fn push_history_view(payload: &mut Vec<u8>, view: &HistoryView) -> Result<(), ProtocolError> {
    push_text(payload, &view.query, MAX_QUERY)?;
    payload.extend_from_slice(&view.first.to_be_bytes());
    payload.extend_from_slice(&view.total.to_be_bytes());
    push_count(payload, view.rows.len(), MAX_VIEW_ROWS, "too many rows")?;
    for row in &view.rows {
        payload.extend_from_slice(&row.number.to_be_bytes());
        push_optional(payload, row.heading.as_deref(), MAX_LABEL)?;
        push_text(payload, &row.time, MAX_LABEL)?;
        push_text(payload, &row.channel, MAX_NAME)?;
        push_page(payload, row.page)?;
        push_text(payload, &row.title, MAX_PAGE_TITLE.max(MAX_NAME))?;
        push_text(payload, &row.address, MAX_SHOWN_ADDRESS)?;
        push_flag(payload, row.bookmarked);
    }
    push_count(payload, view.closed.len(), MAX_CLOSED, "too many rows")?;
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
    let count = fields.count_u16(MAX_VIEW_ROWS, "too many rows")?;
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
                bookmarked: fields.flag("flag")?,
            })
        })
        .collect::<Result<_, ProtocolError>>()?;
    let count = fields.count_u16(MAX_CLOSED, "too many rows")?;
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
        HistoryRequest::RemoveBookmark(number) => {
            payload.push(HISTORY_REMOVE_BOOKMARK);
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
        HISTORY_PANEL => HistoryRequest::Panel(fields.flag("flag")?),
        HISTORY_QUERY => HistoryRequest::Query { text: fields.text(MAX_QUERY)?, first: fields.u32()? },
        HISTORY_OPEN => HistoryRequest::Open { number: fields.u32()?, new_tab: fields.flag("flag")? },
        HISTORY_REMOVE => HistoryRequest::Remove(read_numbers(fields)?),
        HISTORY_MORE_FROM_SITE => HistoryRequest::MoreFromSite(fields.u32()?),
        HISTORY_REMOVE_BOOKMARK => HistoryRequest::RemoveBookmark(fields.u32()?),
        HISTORY_CLEAR => HistoryRequest::Clear(read_range(fields)?),
        HISTORY_CLEAR_COOKIES => HistoryRequest::ClearCookies(read_range(fields)?),
        HISTORY_REOPEN => HistoryRequest::Reopen(if fields.is_empty() { None } else { Some(fields.u32()?) }),
        _ => return Err(ProtocolError::BadValue("history request")),
    })
}
