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

pub use interface_core::settings::{ProxyView, SettingChange, SettingsView};

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
const CHANGE_REMEMBER_HISTORY: u8 = 8;
const CHANGE_RESTORE_SESSION: u8 = 9;

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
    push_flag(payload, view.remember_history);
    push_flag(payload, view.restore_session);
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
    let remember_history = read_flag(fields)?;
    let restore_session = read_flag(fields)?;
    let refusal = if read_flag(fields)? { Some(fields.text(MAX_REFUSAL)?) } else { None };
    Ok(SettingsView { home, sound, volume, proxy, local, remember_history, restore_session, refusal })
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
        SettingChange::RememberHistory(on) => {
            payload.push(CHANGE_REMEMBER_HISTORY);
            push_flag(payload, *on);
        }
        SettingChange::RestoreSession(on) => {
            payload.push(CHANGE_RESTORE_SESSION);
            push_flag(payload, *on);
        }
    }
    Ok(())
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
        CHANGE_REMEMBER_HISTORY => SettingChange::RememberHistory(read_flag(fields)?),
        CHANGE_RESTORE_SESSION => SettingChange::RestoreSession(read_flag(fields)?),
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
            remember_history: false,
            restore_session: true,
            refusal: Some("Not an address on this computer or the local network".into()),
        }
    }

    #[test]
    fn the_view_and_every_change_round_trip() {
        let mut payload = Vec::new();
        push_view(&mut payload, &view()).unwrap();
        assert_eq!(read_view(&mut Fields { bytes: &payload }).unwrap(), view());
        for change in [
            SettingChange::Home("news.example".into()),
            SettingChange::HomeIsThisPage,
            SettingChange::Sound(false),
            SettingChange::Volume(100),
            SettingChange::Proxy(ProxyView::Http { host: "proxy.example".into(), port: 3128 }),
            SettingChange::Proxy(ProxyView::None),
            SettingChange::AddLocal("10.0.0.1".into()),
            SettingChange::RemoveLocal(0),
            SettingChange::RememberHistory(false),
            SettingChange::RestoreSession(false),
        ] {
            let mut payload = Vec::new();
            push_change(&mut payload, &change).unwrap();
            assert_eq!(read_change(&mut Fields { bytes: &payload }).unwrap(), change);
        }
    }

    #[test]
    fn what_breaks_a_bound_is_refused() {
        assert!(push_view(&mut Vec::new(), &SettingsView { volume: 101, ..view() }).is_err());
        assert!(push_view(&mut Vec::new(), &SettingsView { local: vec!["10.0.0.1".into(); MAX_LOCAL + 1], ..view() }).is_err());
        assert!(push_view(&mut Vec::new(), &SettingsView { home: "a\u{202e}b".into(), ..view() }).is_err(), "hidden characters");
        assert!(push_change(&mut Vec::new(), &SettingChange::Volume(101)).is_err());
        assert!(read_change(&mut Fields { bytes: &[CHANGE_VOLUME, 101] }).is_err());
        assert!(read_change(&mut Fields { bytes: &[9] }).is_err());
        assert!(read_change(&mut Fields { bytes: &[CHANGE_SOUND, 2] }).is_err());
        assert!(read_change(&mut Fields { bytes: &[CHANGE_PROXY, 3] }).is_err());
    }
}
