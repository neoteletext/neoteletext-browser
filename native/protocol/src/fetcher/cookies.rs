//! The cookie messages of the fetcher's pipe (`cookies.md` section 9): a
//! fetcher's questions about the cookies of a request and the cookies a
//! response set, and the trusted core's answers. The trusted core keeps
//! every cookie; a fetcher only asks and tells.

use super::kinds::{KIND_CLEAR_SITE_DATA, KIND_COOKIE_ANSWER, KIND_COOKIE_QUESTION, KIND_COOKIE_SET, KIND_REACHED_ANSWER, KIND_REACHED_QUESTION};
use super::{FromFetcher, ToFetcher};
use crate::{Fields, MAX_ADDRESS, ProtocolError, push_raw_text, raw_text};

/// The longest `Set-Cookie` value a fetcher passes on: the engine's limit
/// on one response's headers (`neoteletext-web` `DEFAULT_HEADER_LIMIT`,
/// which the trusted core asserts it equals), since one header cannot be
/// longer than all of them.
pub const MAX_SET_COOKIE: usize = 16 * 1024;
/// The longest `Cookie` header the trusted core answers with: the engine's
/// room for one whole request (`DEFAULT_REQUEST_LIMIT`, asserted likewise).
pub const MAX_COOKIE_HEADER: usize = 16 * 1024;

/// The kind of request a `Cookie` header is asked for (the engine's
/// `cookie::Site`, REQ-WEB-006).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CookieSite {
    /// A request within the channel.
    SameService,
    /// A top-level navigation into the channel from another.
    CrossServiceNavigation,
}

const SITE_SAME: u8 = 0;
const SITE_NAVIGATION: u8 = 1;
// 2 was a form sent to another origin, withdrawn with the engine's forms
// (its decision E.126); it is refused, and not reused.

/// The payload of a cookie message (`cookies.md` section 9), and its kind.
pub(super) fn push_cookie_message(payload: &mut Vec<u8>, message: &FromFetcher) -> Result<u8, ProtocolError> {
    Ok(match message {
        FromFetcher::CookieQuestion { id, host, path, secure, site } => {
            payload.extend_from_slice(&id.to_be_bytes());
            push_raw_text(payload, host, MAX_ADDRESS);
            push_raw_text(payload, path, MAX_ADDRESS);
            payload.push(u8::from(*secure));
            payload.push(match site {
                CookieSite::SameService => SITE_SAME,
                CookieSite::CrossServiceNavigation => SITE_NAVIGATION,
            });
            KIND_COOKIE_QUESTION
        }
        FromFetcher::ReachedQuestion { id, host } => {
            payload.extend_from_slice(&id.to_be_bytes());
            push_raw_text(payload, host, MAX_ADDRESS);
            KIND_REACHED_QUESTION
        }
        FromFetcher::CookieSet { host, path, scope, secure, line } => {
            if line.len() > MAX_SET_COOKIE {
                return Err(ProtocolError::BadValue("Set-Cookie too long"));
            }
            push_raw_text(payload, host, MAX_ADDRESS);
            push_raw_text(payload, path, MAX_ADDRESS);
            payload.push(u8::from(scope.is_some()) | (u8::from(*secure) << 1));
            if let Some(scope) = scope {
                push_raw_text(payload, scope, MAX_ADDRESS);
            }
            payload.extend_from_slice(line);
            KIND_COOKIE_SET
        }
        FromFetcher::ClearSiteData(host) => {
            push_raw_text(payload, host, MAX_ADDRESS);
            KIND_CLEAR_SITE_DATA
        }
        _ => return Err(ProtocolError::BadValue("not a cookie message")),
    })
}

/// Reads a cookie message of `kind`.
pub(super) fn read_cookie_message(kind: u8, fields: &mut Fields<'_>) -> Result<FromFetcher, ProtocolError> {
    Ok(match kind {
        KIND_COOKIE_QUESTION => {
            let id = fields.u32()?;
            let host = raw_text(fields, MAX_ADDRESS, "text too long")?;
            let path = raw_text(fields, MAX_ADDRESS, "text too long")?;
            let secure = fields.flag("secure")?;
            let site = match fields.u8()? {
                SITE_SAME => CookieSite::SameService,
                SITE_NAVIGATION => CookieSite::CrossServiceNavigation,
                _ => return Err(ProtocolError::BadValue("site")),
            };
            FromFetcher::CookieQuestion { id, host, path, secure, site }
        }
        KIND_REACHED_QUESTION => {
            let id = fields.u32()?;
            FromFetcher::ReachedQuestion { id, host: raw_text(fields, MAX_ADDRESS, "text too long")? }
        }
        KIND_COOKIE_SET => {
            let host = raw_text(fields, MAX_ADDRESS, "text too long")?;
            let path = raw_text(fields, MAX_ADDRESS, "text too long")?;
            let flags = fields.u8()?;
            if flags > 3 {
                return Err(ProtocolError::BadValue("cookie flags"));
            }
            let scope = if flags & 1 != 0 { Some(raw_text(fields, MAX_ADDRESS, "text too long")?) } else { None };
            let line = fields.take(fields.remaining())?;
            if line.len() > MAX_SET_COOKIE {
                return Err(ProtocolError::BadValue("Set-Cookie too long"));
            }
            FromFetcher::CookieSet { host, path, scope, secure: flags & 2 != 0, line: line.to_vec() }
        }
        KIND_CLEAR_SITE_DATA => FromFetcher::ClearSiteData(raw_text(fields, MAX_ADDRESS, "text too long")?),
        _ => return Err(ProtocolError::UnknownKind(kind)),
    })
}

/// The payload of an answer to a cookie question, and its kind.
pub(super) fn push_answer(payload: &mut Vec<u8>, answer: &ToFetcher) -> Result<u8, ProtocolError> {
    Ok(match answer {
        ToFetcher::CookieAnswer { id, header } => {
            if header.len() > MAX_COOKIE_HEADER {
                return Err(ProtocolError::BadValue("cookie header too long"));
            }
            payload.extend_from_slice(&id.to_be_bytes());
            payload.extend_from_slice(header);
            KIND_COOKIE_ANSWER
        }
        ToFetcher::ReachedAnswer { id, reached } => {
            payload.extend_from_slice(&id.to_be_bytes());
            payload.push(u8::from(*reached));
            KIND_REACHED_ANSWER
        }
        #[cfg(feature = "interaction")]
        ToFetcher::Send { .. } | ToFetcher::MeasureRoundTrip(_) => return Err(ProtocolError::BadValue("not a cookie answer")),
        ToFetcher::FetchSubPages { .. } | ToFetcher::FetchSubPage { .. } => return Err(ProtocolError::BadValue("not a cookie answer")),
    })
}

/// Reads an answer to a cookie question of `kind`.
pub(super) fn read_answer(kind: u8, fields: &mut Fields<'_>) -> Result<ToFetcher, ProtocolError> {
    Ok(match kind {
        KIND_COOKIE_ANSWER => {
            let id = fields.u32()?;
            let header = fields.take(fields.remaining())?;
            if header.len() > MAX_COOKIE_HEADER {
                return Err(ProtocolError::BadValue("cookie header too long"));
            }
            ToFetcher::CookieAnswer { id, header: header.to_vec() }
        }
        KIND_REACHED_ANSWER => {
            let id = fields.u32()?;
            let reached = fields.flag("reached")?;
            ToFetcher::ReachedAnswer { id, reached }
        }
        other => return Err(ProtocolError::UnknownKind(other)),
    })
}
