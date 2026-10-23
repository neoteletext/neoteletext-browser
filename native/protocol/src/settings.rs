//! Settings (`settings-home-console-proxy.md` section 1, requirements 19.1
//! to 19.4): the trusted core's settings as the panel of the trusted strip
//! shows them, and the changes the panel asks for, which are strip input
//! the trusted core checks. The interface receives a security setting only
//! as this text, and acts on none (requirement 3.6).

use super::{Fields, MAX_ADDRESS, ProtocolError, push_text};

/// The most local addresses allowed (`settings-home-console-proxy.md`
/// section 1.1).
pub const MAX_LOCAL: usize = 16;
/// The longest local address as text: an IPv6 address written in full.
pub const MAX_LOCAL_TEXT: usize = 45;
/// The longest proxy host: SOCKS5 carries a name of at most 255 bytes.
pub const MAX_PROXY_HOST: usize = 255;
/// The loudest volume.
pub const MAX_VOLUME: u8 = 100;
/// The longest refusal the panel says.
pub const MAX_REFUSAL: usize = 160;

pub use interface_core::settings::{CookieMode, MAX_REMOTE_PLACE, ProxyView, RemoteMode, RemotePlace, SettingChange, SettingsView, UpdateView};

const PROXY_NONE: u8 = 0;
const PROXY_HTTP: u8 = 1;
const PROXY_SOCKS5: u8 = 2;
const CHANGE_HOME: u8 = 1;
const CHANGE_HOME_IS_THIS_PAGE: u8 = 2;
const CHANGE_SOUND: u8 = 3;
const CHANGE_VOLUME: u8 = 4;
const CHANGE_PROXY: u8 = 5;
const CHANGE_ADD_LOCAL: u8 = 6;
const CHANGE_REMOVE_LOCAL: u8 = 7;
// 8 was "Remember history", removed by decision 65; never reused.
const CHANGE_RESTORE_SESSION: u8 = 9;
const CHANGE_COOKIES: u8 = 10;
const CHANGE_REMOTE: u8 = 11;
const REMOTE_HIDDEN: u8 = 0;
const REMOTE_DOCKED: u8 = 1;
const REMOTE_FLOATING: u8 = 2;

/// The remote control's place (decision 68).
pub(crate) fn push_remote(payload: &mut Vec<u8>, place: RemotePlace) -> Result<(), ProtocolError> {
    if place.x > MAX_REMOTE_PLACE || place.y > MAX_REMOTE_PLACE {
        return Err(ProtocolError::BadValue("remote place"));
    }
    payload.push(match place.mode {
        RemoteMode::Hidden => REMOTE_HIDDEN,
        RemoteMode::Docked => REMOTE_DOCKED,
        RemoteMode::Floating => REMOTE_FLOATING,
    });
    payload.extend_from_slice(&place.x.to_be_bytes());
    payload.extend_from_slice(&place.y.to_be_bytes());
    Ok(())
}

pub(crate) fn read_remote(fields: &mut Fields<'_>) -> Result<RemotePlace, ProtocolError> {
    let mode = match fields.u8()? {
        REMOTE_HIDDEN => RemoteMode::Hidden,
        REMOTE_DOCKED => RemoteMode::Docked,
        REMOTE_FLOATING => RemoteMode::Floating,
        _ => return Err(ProtocolError::BadValue("remote mode")),
    };
    let (x, y) = (fields.u16()?, fields.u16()?);
    if x > MAX_REMOTE_PLACE || y > MAX_REMOTE_PLACE {
        return Err(ProtocolError::BadValue("remote place"));
    }
    Ok(RemotePlace { mode, x, y })
}
// Far from the others, so changes added beside this one elsewhere do not
// meet it (installation-and-updates.md section 3.4).
const CHANGE_CHECK_UPDATES: u8 = 40;

fn push_mode(mode: CookieMode) -> u8 {
    match mode {
        CookieMode::Keep => 0,
        CookieMode::Close => 1,
        CookieMode::Block => 2,
    }
}

fn read_mode(fields: &mut Fields<'_>) -> Result<CookieMode, ProtocolError> {
    Ok(match fields.u8()? {
        0 => CookieMode::Keep,
        1 => CookieMode::Close,
        2 => CookieMode::Block,
        _ => return Err(ProtocolError::BadValue("cookie mode")),
    })
}

fn push_proxy(payload: &mut Vec<u8>, proxy: &ProxyView) -> Result<(), ProtocolError> {
    match proxy {
        ProxyView::None => payload.push(PROXY_NONE),
        ProxyView::Http { host, port } | ProxyView::Socks5 { host, port } => {
            payload.push(if matches!(proxy, ProxyView::Http { .. }) { PROXY_HTTP } else { PROXY_SOCKS5 });
            push_text(payload, host, MAX_PROXY_HOST)?;
            payload.extend_from_slice(&port.to_be_bytes());
        }
    }
    Ok(())
}

fn read_proxy(fields: &mut Fields<'_>) -> Result<ProxyView, ProtocolError> {
    Ok(match fields.u8()? {
        PROXY_NONE => ProxyView::None,
        PROXY_HTTP => ProxyView::Http { host: fields.text(MAX_PROXY_HOST)?, port: fields.u16()? },
        PROXY_SOCKS5 => ProxyView::Socks5 { host: fields.text(MAX_PROXY_HOST)?, port: fields.u16()? },
        _ => return Err(ProtocolError::BadValue("proxy kind")),
    })
}

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

fn read_volume(fields: &mut Fields<'_>) -> Result<u8, ProtocolError> {
    let volume = fields.u8()?;
    if volume > MAX_VOLUME { Err(ProtocolError::BadValue("volume")) } else { Ok(volume) }
}

pub(crate) fn push_view(payload: &mut Vec<u8>, view: &SettingsView) -> Result<(), ProtocolError> {
    if view.volume > MAX_VOLUME || view.local.len() > MAX_LOCAL {
        return Err(ProtocolError::BadValue("settings"));
    }
    push_text(payload, &view.home, MAX_ADDRESS)?;
    push_flag(payload, view.sound);
    payload.push(view.volume);
    push_proxy(payload, &view.proxy)?;
    payload.push(u8::try_from(view.local.len()).map_err(|_| ProtocolError::BadValue("settings"))?);
    for address in &view.local {
        push_text(payload, address, MAX_LOCAL_TEXT)?;
    }
    push_flag(payload, view.restore_session);
    payload.push(push_mode(view.cookies));
    match &view.refusal {
        Some(refusal) => {
            push_flag(payload, true);
            push_text(payload, refusal, MAX_REFUSAL)?;
        }
        None => push_flag(payload, false),
    }
    Ok(())
}

pub(crate) fn read_view(fields: &mut Fields<'_>) -> Result<SettingsView, ProtocolError> {
    let home = fields.text(MAX_ADDRESS)?;
    let sound = read_flag(fields)?;
    let volume = read_volume(fields)?;
    let proxy = read_proxy(fields)?;
    let count = usize::from(fields.u8()?);
    if count > MAX_LOCAL {
        return Err(ProtocolError::BadValue("too many local addresses"));
    }
    let local = (0..count).map(|_| fields.text(MAX_LOCAL_TEXT)).collect::<Result<Vec<_>, _>>()?;
    let restore_session = read_flag(fields)?;
    let cookies = read_mode(fields)?;
    let refusal = if read_flag(fields)? { Some(fields.text(MAX_REFUSAL)?) } else { None };
    Ok(SettingsView { home, sound, volume, proxy, local, restore_session, cookies, refusal })
}

pub(crate) fn push_change(payload: &mut Vec<u8>, change: &SettingChange) -> Result<(), ProtocolError> {
    match change {
        SettingChange::Home(home) => {
            payload.push(CHANGE_HOME);
            push_text(payload, home, MAX_ADDRESS)?;
        }
        SettingChange::HomeIsThisPage => payload.push(CHANGE_HOME_IS_THIS_PAGE),
        SettingChange::Sound(sound) => {
            payload.push(CHANGE_SOUND);
            push_flag(payload, *sound);
        }
        SettingChange::Volume(volume) => {
            if *volume > MAX_VOLUME {
                return Err(ProtocolError::BadValue("volume"));
            }
            payload.extend_from_slice(&[CHANGE_VOLUME, *volume]);
        }
        SettingChange::Proxy(proxy) => {
            payload.push(CHANGE_PROXY);
            push_proxy(payload, proxy)?;
        }
        SettingChange::AddLocal(address) => {
            payload.push(CHANGE_ADD_LOCAL);
            push_text(payload, address, MAX_LOCAL_TEXT)?;
        }
        SettingChange::RemoveLocal(index) => payload.extend_from_slice(&[CHANGE_REMOVE_LOCAL, *index]),
        SettingChange::RestoreSession(on) => {
            payload.push(CHANGE_RESTORE_SESSION);
            push_flag(payload, *on);
        }
        SettingChange::Cookies(mode) => payload.extend_from_slice(&[CHANGE_COOKIES, push_mode(*mode)]),
        SettingChange::Remote(place) => {
            payload.push(CHANGE_REMOTE);
            push_remote(payload, *place)?;
        }
        SettingChange::CheckUpdates(on) => {
            payload.push(CHANGE_CHECK_UPDATES);
            push_flag(payload, *on);
        }
    }
    Ok(())
}

/// The longest status line of updates the panel says.
pub const MAX_UPDATE_STATUS: usize = 200;
/// The longest version as text.
pub const MAX_VERSION: usize = 17;

pub(crate) fn push_update(payload: &mut Vec<u8>, view: &UpdateView) -> Result<(), ProtocolError> {
    push_text(payload, &view.version, MAX_VERSION)?;
    push_text(payload, &view.status, MAX_UPDATE_STATUS)?;
    match &view.offer {
        Some(offer) => {
            push_flag(payload, true);
            push_text(payload, offer, MAX_VERSION)?;
        }
        None => push_flag(payload, false),
    }
    push_flag(payload, view.checking);
    Ok(())
}

pub(crate) fn read_update(fields: &mut Fields<'_>) -> Result<UpdateView, ProtocolError> {
    let version = fields.text(MAX_VERSION)?;
    let status = fields.text(MAX_UPDATE_STATUS)?;
    let offer = if read_flag(fields)? { Some(fields.text(MAX_VERSION)?) } else { None };
    let checking = read_flag(fields)?;
    Ok(UpdateView { version, status, offer, checking })
}

pub(crate) fn read_change(fields: &mut Fields<'_>) -> Result<SettingChange, ProtocolError> {
    Ok(match fields.u8()? {
        CHANGE_HOME => SettingChange::Home(fields.text(MAX_ADDRESS)?),
        CHANGE_HOME_IS_THIS_PAGE => SettingChange::HomeIsThisPage,
        CHANGE_SOUND => SettingChange::Sound(read_flag(fields)?),
        CHANGE_VOLUME => SettingChange::Volume(read_volume(fields)?),
        CHANGE_PROXY => SettingChange::Proxy(read_proxy(fields)?),
        CHANGE_ADD_LOCAL => SettingChange::AddLocal(fields.text(MAX_LOCAL_TEXT)?),
        CHANGE_REMOVE_LOCAL => SettingChange::RemoveLocal(fields.u8()?),
        CHANGE_RESTORE_SESSION => SettingChange::RestoreSession(read_flag(fields)?),
        CHANGE_COOKIES => SettingChange::Cookies(read_mode(fields)?),
        CHANGE_REMOTE => SettingChange::Remote(read_remote(fields)?),
        CHANGE_CHECK_UPDATES => SettingChange::CheckUpdates(read_flag(fields)?),
        _ => return Err(ProtocolError::BadValue("setting change")),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view() -> SettingsView {
        SettingsView {
            home: "neoteletext:home".into(),
            sound: true,
            volume: 70,
            proxy: ProxyView::Socks5 { host: "127.0.0.1".into(), port: 1080 },
            local: vec!["192.168.1.20".into()],
            restore_session: true,
            cookies: CookieMode::Close,
            refusal: Some("Not an address on this computer or the local network".into()),
        }
    }

    #[test]
    fn the_view_and_every_change_round_trip() {
        let mut payload = Vec::new();
        push_view(&mut payload, &view()).unwrap();
        assert_eq!(read_view(&mut Fields::new(&payload)).unwrap(), view());
        for change in [
            SettingChange::Home("news.example".into()),
            SettingChange::HomeIsThisPage,
            SettingChange::Sound(false),
            SettingChange::Volume(100),
            SettingChange::Proxy(ProxyView::Http { host: "proxy.example".into(), port: 3128 }),
            SettingChange::Proxy(ProxyView::None),
            SettingChange::AddLocal("10.0.0.1".into()),
            SettingChange::RemoveLocal(0),
            SettingChange::RestoreSession(false),
            SettingChange::Cookies(CookieMode::Block),
            SettingChange::Remote(RemotePlace { mode: RemoteMode::Floating, x: 900, y: 140 }),
            SettingChange::Remote(RemotePlace { mode: RemoteMode::Docked, x: 0, y: 0 }),
            SettingChange::CheckUpdates(false),
        ] {
            let mut payload = Vec::new();
            push_change(&mut payload, &change).unwrap();
            assert_eq!(read_change(&mut Fields::new(&payload)).unwrap(), change);
        }
    }

    #[test]
    fn what_breaks_a_bound_is_refused() {
        assert!(push_view(&mut Vec::new(), &SettingsView { volume: 101, ..view() }).is_err());
        assert!(push_view(&mut Vec::new(), &SettingsView { local: vec!["10.0.0.1".into(); MAX_LOCAL + 1], ..view() }).is_err());
        assert!(push_view(&mut Vec::new(), &SettingsView { home: "a\u{202e}b".into(), ..view() }).is_err(), "hidden characters");
        assert!(push_change(&mut Vec::new(), &SettingChange::Volume(101)).is_err());
        assert!(read_change(&mut Fields::new(&[CHANGE_VOLUME, 101])).is_err());
        assert!(read_change(&mut Fields::new(&[9])).is_err());
        assert!(read_change(&mut Fields::new(&[CHANGE_SOUND, 2])).is_err());
        assert!(read_change(&mut Fields::new(&[CHANGE_PROXY, 3])).is_err());
        assert!(read_change(&mut Fields::new(&[CHANGE_CHECK_UPDATES, 2])).is_err());
    }

    #[test]
    fn the_update_view_round_trips_within_its_bounds() {
        let view = UpdateView { version: "0.1.0".into(), status: "NeoTeletext 0.2.0 is available".into(), offer: Some("0.2.0".into()), checking: true };
        let mut payload = Vec::new();
        push_update(&mut payload, &view).unwrap();
        assert_eq!(read_update(&mut Fields::new(&payload)).unwrap(), view);
        assert!(push_update(&mut Vec::new(), &UpdateView { status: "x".repeat(MAX_UPDATE_STATUS + 1), ..view.clone() }).is_err());
        assert!(push_update(&mut Vec::new(), &UpdateView { status: "a\u{202e}b".into(), ..view }).is_err(), "hidden characters");
    }
}
