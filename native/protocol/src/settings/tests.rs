//! Settings' tests: the view, every change and the update view round
//! trip, and what breaks a bound is refused.

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
