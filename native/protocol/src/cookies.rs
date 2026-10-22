//! The cookies panel between the trusted core and the interface
//! (`cookies.md` section 9): the view, bounded so that one always fits a
//! message, carries no cookie's name or value (requirement 21.6), and the
//! requests act by the view's numbers alone.

use super::{Fields, ProtocolError, push_text};

pub use interface_core::cookies::{CookieRow, CookiesRequest, CookiesView, MAX_LABEL, MAX_NOTICE, MAX_QUERY, MAX_USED, MAX_VIEW_ROWS};

const PANEL: u8 = 1;
const QUERY: u8 = 2;
const DELETE: u8 = 3;
const BLOCK: u8 = 4;
const DELETE_ALL: u8 = 5;

fn flag(fields: &mut Fields<'_>) -> Result<bool, ProtocolError> {
    match fields.u8()? {
        0 => Ok(false),
        1 => Ok(true),
        _ => Err(ProtocolError::BadValue("flag")),
    }
}

pub(crate) fn push_view(payload: &mut Vec<u8>, view: &CookiesView) -> Result<(), ProtocolError> {
    if view.rows.len() > MAX_VIEW_ROWS {
        return Err(ProtocolError::BadValue("too many rows"));
    }
    push_text(payload, &view.query, MAX_QUERY)?;
    payload.extend_from_slice(&view.first.to_be_bytes());
    payload.extend_from_slice(&view.total.to_be_bytes());
    payload.extend_from_slice(&u16::try_from(view.rows.len()).unwrap_or(0).to_be_bytes());
    for row in &view.rows {
        payload.extend_from_slice(&row.number.to_be_bytes());
        payload.push(u8::from(row.site) | (u8::from(row.blocked) << 1));
        push_text(payload, &row.label, MAX_LABEL)?;
        payload.extend_from_slice(&row.count.to_be_bytes());
        payload.extend_from_slice(&row.bytes.to_be_bytes());
        push_text(payload, &row.used, MAX_USED)?;
    }
    payload.extend_from_slice(&view.count.to_be_bytes());
    payload.extend_from_slice(&view.bytes.to_be_bytes());
    push_text(payload, &view.notice, MAX_NOTICE)
}

pub(crate) fn read_view(fields: &mut Fields<'_>) -> Result<CookiesView, ProtocolError> {
    let query = fields.text(MAX_QUERY)?;
    let first = fields.u32()?;
    let total = fields.u32()?;
    let rows = usize::from(fields.u16()?);
    if rows > MAX_VIEW_ROWS {
        return Err(ProtocolError::BadValue("too many rows"));
    }
    let rows = (0..rows)
        .map(|_| {
            let number = fields.u32()?;
            let flags = fields.u8()?;
            if flags > 3 {
                return Err(ProtocolError::BadValue("cookie row flags"));
            }
            Ok(CookieRow {
                number,
                site: flags & 1 != 0,
                blocked: flags & 2 != 0,
                label: fields.text(MAX_LABEL)?,
                count: fields.u32()?,
                bytes: u64::from_be_bytes(fields.array()?),
                used: fields.text(MAX_USED)?,
            })
        })
        .collect::<Result<_, ProtocolError>>()?;
    let count = fields.u32()?;
    let bytes = u64::from_be_bytes(fields.array()?);
    Ok(CookiesView { query, first, total, rows, count, bytes, notice: fields.text(MAX_NOTICE)? })
}

pub(crate) fn push_request(payload: &mut Vec<u8>, request: &CookiesRequest) -> Result<(), ProtocolError> {
    match request {
        CookiesRequest::Panel(open) => payload.extend_from_slice(&[PANEL, u8::from(*open)]),
        CookiesRequest::Query { text, first } => {
            payload.push(QUERY);
            push_text(payload, text, MAX_QUERY)?;
            payload.extend_from_slice(&first.to_be_bytes());
        }
        CookiesRequest::Delete(number) => {
            payload.push(DELETE);
            payload.extend_from_slice(&number.to_be_bytes());
        }
        CookiesRequest::Block { number, blocked } => {
            payload.push(BLOCK);
            payload.extend_from_slice(&number.to_be_bytes());
            payload.push(u8::from(*blocked));
        }
        CookiesRequest::DeleteAll => payload.push(DELETE_ALL),
    }
    Ok(())
}

pub(crate) fn read_request(fields: &mut Fields<'_>) -> Result<CookiesRequest, ProtocolError> {
    Ok(match fields.u8()? {
        PANEL => CookiesRequest::Panel(flag(fields)?),
        QUERY => CookiesRequest::Query { text: fields.text(MAX_QUERY)?, first: fields.u32()? },
        DELETE => CookiesRequest::Delete(fields.u32()?),
        BLOCK => CookiesRequest::Block { number: fields.u32()?, blocked: flag(fields)? },
        DELETE_ALL => CookiesRequest::DeleteAll,
        _ => return Err(ProtocolError::BadValue("cookies request")),
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, reason = "a test")]
mod tests {
    use super::*;
    use crate::MAX_PAYLOAD;

    fn row(number: u32) -> CookieRow {
        CookieRow {
            number,
            site: number == 1,
            label: "https://news.example/teletext/".into(),
            count: 3,
            bytes: 120,
            used: "4 days ago".into(),
            blocked: number == 2,
        }
    }

    /// Requirement 21.6: the view and each request round trip; the largest
    /// view fits one message.
    #[test]
    fn views_and_requests_round_trip() {
        let view = CookiesView { query: "news".into(), first: 0, total: 2, rows: vec![row(1), row(2)], count: 3, bytes: 120, notice: "Deleted".into() };
        let mut payload = Vec::new();
        push_view(&mut payload, &view).unwrap();
        assert_eq!(read_view(&mut Fields { bytes: &payload }).unwrap(), view);
        for request in [
            CookiesRequest::Panel(true),
            CookiesRequest::Query { text: "x".into(), first: 100 },
            CookiesRequest::Delete(4),
            CookiesRequest::Block { number: 2, blocked: true },
            CookiesRequest::DeleteAll,
        ] {
            let mut payload = Vec::new();
            push_request(&mut payload, &request).unwrap();
            assert_eq!(read_request(&mut Fields { bytes: &payload }).unwrap(), request);
        }
        let label = "x".repeat(MAX_LABEL);
        let rows =
            (0..MAX_VIEW_ROWS).map(|number| CookieRow { label: label.clone(), used: "u".repeat(MAX_USED), ..row(u32::try_from(number).unwrap()) }).collect();
        let largest = CookiesView { query: "q".repeat(MAX_QUERY), rows, notice: "n".repeat(MAX_NOTICE), ..CookiesView::default() };
        let mut payload = Vec::new();
        push_view(&mut payload, &largest).unwrap();
        assert!(payload.len() < MAX_PAYLOAD, "{}", payload.len());
    }
}
