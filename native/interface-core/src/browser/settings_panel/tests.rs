use super::*;
use alloc::vec::Vec;

use crate::browser::{Browser, Frame, Key, Modifiers, Output};
use crate::geometry::{Point, Rect};
use crate::settings::{CookieMode, ProxyView, SettingChange, SettingsView, UpdateView};
use crate::test_canvas::Recorder;

fn view() -> SettingsView {
    SettingsView {
        home: "neoteletext:home".into(),
        sound: true,
        volume: 80,
        proxy: ProxyView::None,
        local: alloc::vec!["192.168.1.20".into()],
        restore_session: true,
        cookies: CookieMode::Keep,
        refusal: None,
    }
}

fn open() -> Browser {
    let mut browser = Browser::new(Frame { left_inset: 0.0, window_buttons: false });
    browser.set_settings(view());
    browser.draw(&mut Recorder::new(), 1440.0, 900.0, 0);
    browser.toggle_settings();
    browser.draw(&mut Recorder::new(), 1440.0, 900.0, 0);
    browser.take_outputs();
    browser
}

fn press(browser: &mut Browser, rect: Rect) -> Vec<Output> {
    assert!(browser.settings_press(Point::new(f64::midpoint(rect.x0, rect.x1), f64::midpoint(rect.y0, rect.y1))));
    browser.draw(&mut Recorder::new(), 1440.0, 900.0, 0);
    browser.take_outputs()
}

fn typed(browser: &mut Browser, text: &str) {
    browser.key(Key::Text(text), Modifiers::default(), 0);
}

/// `installation-and-updates.md` section 3.4: the panel says the
/// version and the trusted core's word, offers Install only when the
/// trusted core offers a version, and asks for checking on or off.
#[test]
fn the_panel_offers_an_update_only_when_the_trusted_core_does() {
    let mut browser = open();
    assert_eq!(browser.settings.controls.updates_on, Rect::ZERO, "no updates row before the trusted core says");
    browser.set_update(UpdateView { version: "0.1.0".into(), status: "NeoTeletext is up to date".into(), offer: None, checking: true });
    browser.draw(&mut Recorder::new(), 1440.0, 900.0, 0);
    assert_eq!(browser.settings.controls.install, Rect::ZERO, "nothing to install");
    let off = browser.settings.controls.updates_off;
    assert_eq!(press(&mut browser, off), [Output::Setting(SettingChange::CheckUpdates(false))]);
    browser.set_update(UpdateView { version: "0.1.0".into(), status: "NeoTeletext 0.2.0 is available".into(), offer: Some("0.2.0".into()), checking: true });
    browser.draw(&mut Recorder::new(), 1440.0, 900.0, 0);
    let install = browser.settings.controls.install;
    assert_ne!(install, Rect::ZERO);
    assert_eq!(press(&mut browser, install), [Output::InstallUpdate]);
    let on = browser.settings.controls.updates_on;
    assert_eq!(press(&mut browser, on), [Output::Setting(SettingChange::CheckUpdates(true))]);
}

/// Requirement 19.3: each control asks the trusted core for its change.
#[test]
fn each_control_asks_for_its_change() {
    let mut browser = open();
    let controls = &browser.settings.controls;
    let (this_page, off, quieter, louder, add) = (controls.this_page, controls.sound_off, controls.quieter, controls.louder, controls.add);
    let remove = controls.removes[0].1;
    let socks = controls.kinds[2].1;
    let none = controls.kinds[0].1;
    let home = controls.fields[0].1;
    assert_eq!(press(&mut browser, this_page), [Output::Setting(SettingChange::HomeIsThisPage)]);
    assert_eq!(press(&mut browser, off), [Output::Setting(SettingChange::Sound(false))]);
    assert_eq!(press(&mut browser, quieter), [Output::Setting(SettingChange::Volume(70))]);
    assert_eq!(press(&mut browser, louder), [Output::Setting(SettingChange::Volume(90))]);
    assert_eq!(press(&mut browser, remove), [Output::Setting(SettingChange::RemoveLocal(0))]);
    // A proxy's kind waits for its host and port.
    assert_eq!(press(&mut browser, socks), []);
    let (host, port, apply) = (browser.settings.controls.fields[1].1, browser.settings.controls.fields[2].1, browser.settings.controls.apply);
    assert_eq!(press(&mut browser, host), [Output::TextInput(true)]);
    typed(&mut browser, "127.0.0.1");
    browser.key(Key::Tab, Modifiers::default(), 0);
    assert_eq!(browser.settings.focus, Some(Field::ProxyPort));
    typed(&mut browser, "1080");
    browser.take_outputs();
    assert_eq!(
        press(&mut browser, apply),
        [Output::TextInput(false), Output::Setting(SettingChange::Proxy(ProxyView::Socks5 { host: "127.0.0.1".into(), port: 1080 }))]
    );
    let _ = port;
    assert_eq!(press(&mut browser, none), [Output::Setting(SettingChange::Proxy(ProxyView::None))]);
    let (restore, home_at_start) = (browser.settings.controls.restore_on, browser.settings.controls.restore_off);
    assert_eq!(press(&mut browser, home_at_start), [Output::Setting(SettingChange::RestoreSession(false))]);
    assert_eq!(press(&mut browser, restore), [Output::Setting(SettingChange::RestoreSession(true))]);
    let on_close = browser.settings.controls.cookie_modes[1].1;
    assert_eq!(press(&mut browser, on_close), [Output::Setting(SettingChange::Cookies(CookieMode::Close))]);
    let manage = browser.settings.controls.manage_cookies;
    assert_eq!(
        press(&mut browser, manage),
        [Output::Console(true), Output::Cookies(crate::cookies::CookiesRequest::Panel(true))],
        "the settings panel gives way to the console's cookies"
    );
    assert!(browser.cookies_open() && browser.console_open() && !browser.settings_open());
    browser.toggle_settings();
    browser.draw(&mut Recorder::new(), 1440.0, 900.0, 0);
    browser.take_outputs();
    // A field's change goes on Enter.
    press(&mut browser, home);
    browser.key(Key::Text("a"), Modifiers { command: true, ..Modifiers::default() }, 0);
    typed(&mut browser, "news.example");
    browser.key(Key::Enter, Modifiers::default(), 0);
    assert_eq!(browser.take_outputs(), [Output::TextInput(false), Output::Setting(SettingChange::Home("news.example".into()))]);
    let local = browser.settings.controls.fields.iter().find(|(field, _)| *field == Field::Local).map(|(_, rect)| *rect).unwrap();
    press(&mut browser, local);
    typed(&mut browser, "10.0.0.7");
    browser.take_outputs();
    assert_eq!(press(&mut browser, add), [Output::TextInput(false), Output::Setting(SettingChange::AddLocal("10.0.0.7".into()))]);
}

/// Requirement 19.3: a refusal is the trusted core's, shown as sent;
/// the panel shows the settings it was sent.
#[test]
fn the_panel_shows_the_settings_and_the_refusal() {
    let mut browser = open();
    browser.set_settings(SettingsView {
        refusal: Some("Not an address on this computer or the local network".into()),
        proxy: ProxyView::Http { host: "proxy.example".into(), port: 3128 },
        ..view()
    });
    let mut canvas = Recorder::new();
    browser.draw(&mut canvas, 1440.0, 900.0, 0);
    let texts: Vec<&str> = canvas.texts.iter().map(|(_, text)| text.as_str()).collect();
    for expected in ["Settings", "neoteletext:home", "80", "192.168.1.20", "proxy.example", "3128", "Not an address on this computer or the local network"] {
        assert!(texts.contains(&expected), "{expected} in {texts:?}");
    }
    assert!(browser.settings_key(Key::Escape, false) == Some(true) && !browser.settings_open(), "Escape closes it");
}

#[test]
fn the_gear_and_its_shortcut_open_the_panel() {
    let mut browser = Browser::new(Frame { left_inset: 0.0, window_buttons: false });
    browser.draw(&mut Recorder::new(), 1440.0, 900.0, 0);
    let gear = browser.regions.gear;
    browser.pointer_moved(f64::midpoint(gear.x0, gear.x1), f64::midpoint(gear.y0, gear.y1), 0);
    browser.pointer_pressed(&mut Recorder::new(), false, 0);
    assert!(browser.settings_open());
    browser.key(Key::Text(","), Modifiers { command: true, ..Modifiers::default() }, 0);
    assert!(!browser.settings_open());
}
