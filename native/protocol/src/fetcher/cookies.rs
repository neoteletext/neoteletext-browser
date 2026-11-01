//! The cookie messages of the fetcher's pipe (`cookies.md` section 9): a
//! fetcher's questions about the cookies of a request and the cookies a
//! response set, and the trusted core's answers. The trusted core keeps
//! every cookie and applies every rule; a fetcher only asks and tells, and
//! reads each `Set-Cookie` header into typed, bounded fields, so that the
//! core reads no header a server wrote (DEC-1765).

use super::kinds::{KIND_CLEAR_SITE_DATA, KIND_COOKIE_ANSWER, KIND_COOKIE_QUESTION, KIND_COOKIE_SET, KIND_REACHED_ANSWER, KIND_REACHED_QUESTION};
use super::{FromFetcher, ToFetcher};
use crate::{Fields, MAX_ADDRESS, ProtocolError, push_raw_text, raw_text};

/// The most bytes of a cookie's name and value together: the engine's limit
/// (`neoteletext-web` `MAX_COOKIE_NAME_VALUE`, which the trusted core asserts
/// it equals).
pub const MAX_COOKIE_NAME_VALUE: usize = 4096;
/// The longest attribute value a cookie keeps: the engine's limit
/// (`MAX_COOKIE_ATTRIBUTE_VALUE`, asserted likewise).
pub const MAX_COOKIE_ATTRIBUTE: usize = 1024;
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

/// A cookie's `SameSite` attribute, as a fetcher read it (the engine's
/// `cookie::SameSite`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CookieSameSite {
    /// `Strict`.
    Strict,
    /// `Lax`.
    Lax,
    /// `None`.
    None,
    /// Absent or unrecognised.
    Default,
}

/// A `Set-Cookie` header as a fetcher read it, untrusted: the engine's jar in
/// the trusted core applies every rule to these fields (DEC-1765).
#[derive(Clone, Debug, PartialEq, Eq)]
#[allow(clippy::struct_excessive_bools, reason = "each is one attribute of the header, there or not")]
pub struct SetCookie {
    /// The cookie's name.
    pub name: Vec<u8>,
    /// The cookie's value, with the name at most [`MAX_COOKIE_NAME_VALUE`].
    pub value: Vec<u8>,
    /// `Max-Age`, in seconds.
    pub max_age: Option<i64>,
    /// `Expires`, in seconds since 1970.
    pub expires: Option<u64>,
    /// A `Domain` attribute was there.
    pub domain: bool,
    /// `Path`, at most [`MAX_COOKIE_ATTRIBUTE`].
    pub path: Option<Vec<u8>>,
    /// `Secure`.
    pub secure: bool,
    /// `HttpOnly`.
    pub http_only: bool,
    /// `Partitioned`.
    pub partitioned: bool,
    /// `SameSite`.
    pub same_site: CookieSameSite,
}

const SAME_SITE_DEFAULT: u8 = 0;
const SAME_SITE_STRICT: u8 = 1;
const SAME_SITE_LAX: u8 = 2;
const SAME_SITE_NONE: u8 = 3;

const COOKIE_SCOPE: u8 = 1 << 0;
const COOKIE_RESPONSE_SECURE: u8 = 1 << 1;
const COOKIE_DOMAIN: u8 = 1 << 2;
const COOKIE_SECURE: u8 = 1 << 3;
const COOKIE_HTTP_ONLY: u8 = 1 << 4;
const COOKIE_PARTITIONED: u8 = 1 << 5;
const COOKIE_MAX_AGE: u8 = 1 << 6;
const COOKIE_EXPIRES: u8 = 1 << 7;
/// In place of a `Path`'s length: the header had none.
const NO_PATH: u16 = u16::MAX;
const _: () = assert!(MAX_COOKIE_ATTRIBUTE < NO_PATH as usize && MAX_COOKIE_NAME_VALUE <= u16::MAX as usize);

/// A cookie's fields, within their bounds, after the message's flags.
fn push_set_cookie(payload: &mut Vec<u8>, cookie: &SetCookie) -> Result<(), ProtocolError> {
    if cookie.name.len().saturating_add(cookie.value.len()) > MAX_COOKIE_NAME_VALUE {
        return Err(ProtocolError::BadValue("cookie too long"));
    }
    if cookie.path.as_ref().is_some_and(|path| path.len() > MAX_COOKIE_ATTRIBUTE) {
        return Err(ProtocolError::BadValue("cookie path too long"));
    }
    if let Some(seconds) = cookie.max_age {
        payload.extend_from_slice(&seconds.to_be_bytes());
    }
    if let Some(seconds) = cookie.expires {
        payload.extend_from_slice(&seconds.to_be_bytes());
    }
    payload.push(match cookie.same_site {
        CookieSameSite::Default => SAME_SITE_DEFAULT,
        CookieSameSite::Strict => SAME_SITE_STRICT,
        CookieSameSite::Lax => SAME_SITE_LAX,
        CookieSameSite::None => SAME_SITE_NONE,
    });
    let length = |text: &[u8]| u16::try_from(text.len()).unwrap_or(u16::MAX).to_be_bytes();
    payload.extend_from_slice(&length(&cookie.name));
    payload.extend_from_slice(&length(&cookie.value));
    payload.extend_from_slice(&cookie.path.as_deref().map_or(NO_PATH.to_be_bytes(), length));
    payload.extend_from_slice(&cookie.name);
    payload.extend_from_slice(&cookie.value);
    payload.extend_from_slice(cookie.path.as_deref().unwrap_or_default());
    Ok(())
}

/// Reads what [`push_set_cookie`] wrote, under the message's `flags`.
fn read_set_cookie(flags: u8, fields: &mut Fields<'_>) -> Result<SetCookie, ProtocolError> {
    let max_age = (flags & COOKIE_MAX_AGE != 0).then(|| fields.array().map(i64::from_be_bytes)).transpose()?;
    let expires = (flags & COOKIE_EXPIRES != 0).then(|| fields.array().map(u64::from_be_bytes)).transpose()?;
    let same_site = match fields.u8()? {
        SAME_SITE_DEFAULT => CookieSameSite::Default,
        SAME_SITE_STRICT => CookieSameSite::Strict,
        SAME_SITE_LAX => CookieSameSite::Lax,
        SAME_SITE_NONE => CookieSameSite::None,
        _ => return Err(ProtocolError::BadValue("cookie SameSite")),
    };
    let (name, value, path) = (usize::from(fields.u16()?), usize::from(fields.u16()?), fields.u16()?);
    if name.saturating_add(value) > MAX_COOKIE_NAME_VALUE {
        return Err(ProtocolError::BadValue("cookie too long"));
    }
    let name = fields.take(name)?.to_vec();
    let value = fields.take(value)?.to_vec();
    let path = match path {
        NO_PATH => None,
        length if usize::from(length) > MAX_COOKIE_ATTRIBUTE => return Err(ProtocolError::BadValue("cookie path too long")),
        length => Some(fields.take(usize::from(length))?.to_vec()),
    };
    if fields.remaining() != 0 {
        return Err(ProtocolError::BadValue("cookie has more"));
    }
    Ok(SetCookie {
        name,
        value,
        max_age,
        expires,
        domain: flags & COOKIE_DOMAIN != 0,
        path,
        secure: flags & COOKIE_SECURE != 0,
        http_only: flags & COOKIE_HTTP_ONLY != 0,
        partitioned: flags & COOKIE_PARTITIONED != 0,
        same_site,
    })
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
        FromFetcher::CookieSet { host, path, scope, secure, cookie } => {
            push_raw_text(payload, host, MAX_ADDRESS);
            push_raw_text(payload, path, MAX_ADDRESS);
            let flag = |set: bool, bit: u8| if set { bit } else { 0 };
            payload.push(
                flag(scope.is_some(), COOKIE_SCOPE)
                    | flag(*secure, COOKIE_RESPONSE_SECURE)
                    | flag(cookie.domain, COOKIE_DOMAIN)
                    | flag(cookie.secure, COOKIE_SECURE)
                    | flag(cookie.http_only, COOKIE_HTTP_ONLY)
                    | flag(cookie.partitioned, COOKIE_PARTITIONED)
                    | flag(cookie.max_age.is_some(), COOKIE_MAX_AGE)
                    | flag(cookie.expires.is_some(), COOKIE_EXPIRES),
            );
            if let Some(scope) = scope {
                push_raw_text(payload, scope, MAX_ADDRESS);
            }
            push_set_cookie(payload, cookie)?;
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
            let scope = if flags & COOKIE_SCOPE != 0 { Some(raw_text(fields, MAX_ADDRESS, "text too long")?) } else { None };
            let cookie = read_set_cookie(flags, fields)?;
            FromFetcher::CookieSet { host, path, scope, secure: flags & COOKIE_RESPONSE_SECURE != 0, cookie }
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
