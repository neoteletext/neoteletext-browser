//! The list "Channels that know you" between the trusted core and the
//! interface (neoteletext-common's `docs/viewer-state.md`, VS-112): the
//! view, bounded so that one always fits a message, carries no key and no
//! part of one (VS-116), and the requests act by the view's numbers alone.

use super::{Fields, ProtocolError, push_count, push_text};

pub use interface_core::viewer_state::{Known, KnownRequest, KnownRow, KnownView, MAX_LABEL, MAX_NOTICE, MAX_QUERY, MAX_VIEW_ROWS, MAX_WHEN};

const PANEL: u8 = 1;
const QUERY: u8 = 2;
const FORGET: u8 = 3;
const REMEMBER: u8 = 4;
const BLOCK: u8 = 5;
const FORGET_ALL: u8 = 6;
const REMEMBER_ACTIVE: u8 = 7;
const DISMISS_OFFER: u8 = 8;

/// A row's flags.
const HAS_KEY: u8 = 1;
const REMEMBERED: u8 = 2;
const BLOCKED: u8 = 4;
const ROW_FLAGS: u8 = HAS_KEY | REMEMBERED | BLOCKED;

/// The strip's mark of what the active tab's channel knows.
const KNOWN_NOT: u8 = 0;
const KNOWN_VISIT: u8 = 1;
const KNOWN_REMEMBERED: u8 = 2;

/// The byte of what a channel knows, on the strip's wire.
#[must_use]
pub const fn known_code(known: Known) -> u8 {
    match known {
        Known::Not => KNOWN_NOT,
        Known::Visit => KNOWN_VISIT,
        Known::Remembered => KNOWN_REMEMBERED,
    }
}

/// What a channel knows, from its byte.
///
/// # Errors
/// On any byte that names nothing.
pub const fn known_of(code: u8) -> Result<Known, ProtocolError> {
    match code {
        KNOWN_NOT => Ok(Known::Not),
        KNOWN_VISIT => Ok(Known::Visit),
        KNOWN_REMEMBERED => Ok(Known::Remembered),
        _ => Err(ProtocolError::BadValue("known")),
    }
}

pub(crate) fn push_view(payload: &mut Vec<u8>, view: &KnownView) -> Result<(), ProtocolError> {
    // Checked before any field, so that too many rows is the error
    // whatever else is wrong.
    if view.rows.len() > MAX_VIEW_ROWS {
        return Err(ProtocolError::BadValue("too many rows"));
    }
    push_text(payload, &view.query, MAX_QUERY)?;
    payload.extend_from_slice(&view.first.to_be_bytes());
    payload.extend_from_slice(&view.total.to_be_bytes());
    push_count(payload, view.rows.len(), MAX_VIEW_ROWS, "too many rows")?;
    let flag = |set: bool, bit: u8| if set { bit } else { 0 };
    for row in &view.rows {
        payload.extend_from_slice(&row.number.to_be_bytes());
        payload.push(flag(row.has_key, HAS_KEY) | flag(row.remembered, REMEMBERED) | flag(row.blocked, BLOCKED));
        push_text(payload, &row.label, MAX_LABEL)?;
        push_text(payload, &row.created, MAX_WHEN)?;
        push_text(payload, &row.used, MAX_WHEN)?;
    }
    payload.push(u8::from(view.can_remember));
    push_text(payload, &view.notice, MAX_NOTICE)
}

pub(crate) fn read_view(fields: &mut Fields<'_>) -> Result<KnownView, ProtocolError> {
    let query = fields.text(MAX_QUERY)?;
    let first = fields.u32()?;
    let total = fields.u32()?;
    let rows = fields.count_u16(MAX_VIEW_ROWS, "too many rows")?;
    let rows = (0..rows)
        .map(|_| {
            let number = fields.u32()?;
            let flags = fields.u8()?;
            if flags & !ROW_FLAGS != 0 {
                return Err(ProtocolError::BadValue("known row flags"));
            }
            Ok(KnownRow {
                number,
                has_key: flags & HAS_KEY != 0,
                remembered: flags & REMEMBERED != 0,
                blocked: flags & BLOCKED != 0,
                label: fields.text(MAX_LABEL)?,
                created: fields.text(MAX_WHEN)?,
                used: fields.text(MAX_WHEN)?,
            })
        })
        .collect::<Result<_, ProtocolError>>()?;
    let can_remember = fields.flag("flag")?;
    Ok(KnownView { query, first, total, rows, can_remember, notice: fields.text(MAX_NOTICE)? })
}

pub(crate) fn push_request(payload: &mut Vec<u8>, request: &KnownRequest) -> Result<(), ProtocolError> {
    match request {
        KnownRequest::Panel(open) => payload.extend_from_slice(&[PANEL, u8::from(*open)]),
        KnownRequest::Query { text, first } => {
            payload.push(QUERY);
            push_text(payload, text, MAX_QUERY)?;
            payload.extend_from_slice(&first.to_be_bytes());
        }
        KnownRequest::Forget(number) => {
            payload.push(FORGET);
            payload.extend_from_slice(&number.to_be_bytes());
        }
        KnownRequest::Remember { number, remembered } => {
            payload.push(REMEMBER);
            payload.extend_from_slice(&number.to_be_bytes());
            payload.push(u8::from(*remembered));
        }
        KnownRequest::Block { number, blocked } => {
            payload.push(BLOCK);
            payload.extend_from_slice(&number.to_be_bytes());
            payload.push(u8::from(*blocked));
        }
        KnownRequest::ForgetAll => payload.push(FORGET_ALL),
        KnownRequest::RememberActive(remembered) => payload.extend_from_slice(&[REMEMBER_ACTIVE, u8::from(*remembered)]),
        KnownRequest::DismissOffer => payload.push(DISMISS_OFFER),
    }
    Ok(())
}

pub(crate) fn read_request(fields: &mut Fields<'_>) -> Result<KnownRequest, ProtocolError> {
    Ok(match fields.u8()? {
        PANEL => KnownRequest::Panel(fields.flag("flag")?),
        QUERY => KnownRequest::Query { text: fields.text(MAX_QUERY)?, first: fields.u32()? },
        FORGET => KnownRequest::Forget(fields.u32()?),
        REMEMBER => KnownRequest::Remember { number: fields.u32()?, remembered: fields.flag("flag")? },
        BLOCK => KnownRequest::Block { number: fields.u32()?, blocked: fields.flag("flag")? },
        FORGET_ALL => KnownRequest::ForgetAll,
        REMEMBER_ACTIVE => KnownRequest::RememberActive(fields.flag("flag")?),
        DISMISS_OFFER => KnownRequest::DismissOffer,
        _ => return Err(ProtocolError::BadValue("known request")),
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, reason = "a test")]
mod tests {
    use super::*;
    use crate::MAX_PAYLOAD;

    fn row(number: u32) -> KnownRow {
        KnownRow {
            number,
            label: "https://news.example/teletext/".into(),
            has_key: number != 2,
            remembered: number == 1,
            blocked: number == 2,
            created: "4 days ago".into(),
            used: "today".into(),
        }
    }

    /// VS-112: the view and each request round trip; the largest view fits
    /// one message; a row's flags beyond those defined are refused.
    #[test]
    fn views_and_requests_round_trip() {
        let view = KnownView { query: "news".into(), first: 0, total: 2, rows: vec![row(1), row(2)], can_remember: true, notice: "Forgotten".into() };
        let mut payload = Vec::new();
        push_view(&mut payload, &view).unwrap();
        assert_eq!(read_view(&mut Fields::new(&payload)).unwrap(), view);
        for request in [
            KnownRequest::Panel(true),
            KnownRequest::Query { text: "x".into(), first: 100 },
            KnownRequest::Forget(4),
            KnownRequest::Remember { number: 1, remembered: false },
            KnownRequest::Block { number: 2, blocked: true },
            KnownRequest::ForgetAll,
            KnownRequest::RememberActive(true),
            KnownRequest::DismissOffer,
        ] {
            let mut payload = Vec::new();
            push_request(&mut payload, &request).unwrap();
            assert_eq!(read_request(&mut Fields::new(&payload)).unwrap(), request);
        }
        let label = "x".repeat(MAX_LABEL);
        let rows = (0..MAX_VIEW_ROWS)
            .map(|number| KnownRow { label: label.clone(), created: "c".repeat(MAX_WHEN), used: "u".repeat(MAX_WHEN), ..row(u32::try_from(number).unwrap()) })
            .collect();
        let largest = KnownView { query: "q".repeat(MAX_QUERY), rows, notice: "n".repeat(MAX_NOTICE), ..KnownView::default() };
        let mut payload = Vec::new();
        push_view(&mut payload, &largest).unwrap();
        assert!(payload.len() < MAX_PAYLOAD, "{}", payload.len());
        let mut bad = Vec::new();
        push_view(&mut bad, &KnownView { rows: vec![row(1)], ..KnownView::default() }).unwrap();
        // The row's flags follow the query's empty text, two counts, the
        // row count and the row's number.
        let flags_at = 2 + 4 + 4 + 2 + 4;
        bad[flags_at] = ROW_FLAGS + 1;
        assert!(read_view(&mut Fields::new(&bad)).is_err());
        assert!(known_of(3).is_err());
        for known in [Known::Not, Known::Visit, Known::Remembered] {
            assert_eq!(known_of(known_code(known)).unwrap(), known);
        }
    }
}
