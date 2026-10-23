//! The settings panel (`settings-home-console-proxy.md`, requirements 19.1
//! to 19.4 and 19.12): its intents, its view, each change checked by its
//! setting's rule, and the settings saved.

use neoteletext::settings::{self as stored, Settings};
use protocol::console::Level;
use protocol::settings::{ProxyView, SettingChange, SettingsView};
use protocol::sound::ToSound;
use protocol::{ToCore, ToInterface, clean};

use super::Core;

/// What the strip says, and the log keeps, when the user turns update
/// checks off (finding F-68): a security setting, said as the others are.
const UPDATES_OFF: &str = "Updates are off: this browser will not learn of security fixes";

impl Core {
    /// The strip's intents of the settings: Home, a change the panel asks
    /// for, and the console.
    pub(super) fn settings_intent(&mut self, intent: ToCore) -> Result<(), String> {
        match intent {
            ToCore::Home => {
                let home = self.settings.home.clone();
                if let Err(reason) = self.session.navigate(&home, false) {
                    self.log(Level::Warning, None, &format!("{home}: {reason}"));
                }
                self.send_all()
            }
            ToCore::ChangeSetting(change) => self.change_setting(change),
            ToCore::Console(open) => {
                self.console.set_open(open);
                self.send_console()
            }
            ToCore::Drawn | ToCore::InstallUpdate => self.update_intent(&intent),
            _ => Ok(()),
        }
    }

    /// The settings as the panel shows them, with the last refusal; the
    /// sound helper is told the sound and the volume (requirement 19.4).
    pub(super) fn send_settings(&mut self) -> Result<(), String> {
        let message = ToSound::Settings { sound: self.settings.sound, volume: self.settings.volume };
        self.tell_sound(&message);
        let proxy = match &self.settings.proxy {
            None => ProxyView::None,
            Some(stored::Proxy::Http { host, port }) => ProxyView::Http { host: clean(host, protocol::settings::MAX_PROXY_HOST), port: *port },
            Some(stored::Proxy::Socks5 { host, port }) => ProxyView::Socks5 { host: clean(host, protocol::settings::MAX_PROXY_HOST), port: *port },
        };
        let view = SettingsView {
            home: clean(&self.settings.home, protocol::MAX_ADDRESS),
            sound: self.settings.sound,
            volume: self.settings.volume,
            proxy,
            local: self.settings.local.iter().map(ToString::to_string).collect(),
            restore_session: self.settings.restore,
            cookies: super::cookies::mode_view(self.settings.cookies),
            refusal: self.refusal.as_deref().map(|refusal| clean(refusal, protocol::settings::MAX_REFUSAL)),
        };
        self.send(&ToInterface::Settings(view))
    }

    /// Writes the settings, if there is a place for them; a failure is
    /// logged and said in the panel.
    pub(super) fn save_settings(&mut self) {
        let Some(folder) = &self.settings_folder else { return };
        if let Err(error) = self.settings.save(folder) {
            self.log(Level::Error, None, &format!("the settings could not be saved in {}: {error}", folder.display()));
            self.refusal = Some("The settings could not be saved".into());
        }
    }

    /// A change the panel asked for, checked as strip input (requirement
    /// 19.3): kept and saved when the rules accept it, said in the panel
    /// when not; a change of a security setting is said in the strip and
    /// logged.
    fn change_setting(&mut self, change: SettingChange) -> Result<(), String> {
        let current = self.session.active_tab().and_then(|tab| tab.address()).map(|address| address.channel_text().to_owned());
        let cookies = if let SettingChange::Cookies(mode) = change { Some(mode) } else { None };
        let remote = matches!(change, SettingChange::Remote(_));
        let checking = if let SettingChange::CheckUpdates(on) = change { Some(on) } else { None };
        match checked_change(&mut self.settings, change, current.as_deref()) {
            Ok(security) => {
                self.refusal = None;
                self.save_settings();
                self.others_stale();
                if let Some(mode) = cookies {
                    self.cookie_mode_changed(mode);
                }
                match checking {
                    // "Off" stops the updater from starting at all.
                    Some(false) => self.updates.stop(),
                    Some(true) => self.start_updater(),
                    None => {}
                }
                if checking.is_some() {
                    self.send_update()?;
                }
                if let Some(text) = security {
                    self.say(text)?;
                }
                if remote {
                    self.send(&ToInterface::Remote(self.settings.remote))?;
                }
            }
            Err(reason) => self.refusal = Some(reason),
        }
        self.send_settings()?;
        self.send_history()
    }
}

/// Applies one change to `settings` if its setting's rule accepts it;
/// returns what the strip says of a change of a security setting, or why
/// the change was refused. `current` is the open channel's address.
fn checked_change(settings: &mut Settings, change: SettingChange, current: Option<&str>) -> Result<Option<String>, String> {
    const NOT_LOCAL: &str = "Not an address on this computer or the local network";
    match change {
        SettingChange::Home(typed) => {
            let address = neoteletext::address::parse(&typed)?;
            address.channel_text().clone_into(&mut settings.home);
            Ok(None)
        }
        SettingChange::HomeIsThisPage => {
            current.ok_or("No channel is open")?.clone_into(&mut settings.home);
            Ok(None)
        }
        SettingChange::Sound(on) => {
            settings.sound = on;
            Ok(None)
        }
        SettingChange::Volume(volume) if volume <= stored::MAX_VOLUME => {
            settings.volume = volume;
            Ok(None)
        }
        SettingChange::Volume(_) => Err("The volume is from 0 to 100".into()),
        SettingChange::Proxy(ProxyView::None) => {
            let was = settings.proxy.take();
            Ok(was.map(|_| "No proxy: channels opened from now connect directly".to_owned()))
        }
        SettingChange::Proxy(view) => {
            let (kind, host, port) = match &view {
                ProxyView::Http { host, port } => ("http", host, *port),
                ProxyView::Socks5 { host, port } => ("socks5", host, *port),
                ProxyView::None => return Ok(None),
            };
            let proxy = stored::Proxy::from_parts(kind, host.trim(), port).ok_or("Not a proxy: a host and a port from 1 to 65535")?;
            let text = format!("Proxy {}: channels opened from now go through it", proxy.url());
            settings.proxy = Some(proxy);
            Ok(Some(text))
        }
        SettingChange::AddLocal(typed) => {
            let address: std::net::IpAddr = typed.trim().parse().map_err(|_| NOT_LOCAL)?;
            if !stored::is_local(address) {
                return Err(NOT_LOCAL.into());
            }
            if settings.local.contains(&address) {
                return Err(format!("{address} is allowed already"));
            }
            if settings.local.len() >= stored::MAX_LOCAL {
                return Err(format!("At most {} local addresses can be allowed", stored::MAX_LOCAL));
            }
            settings.local.push(address);
            Ok(Some(format!("{address} on the local network allowed for channels opened from now")))
        }
        SettingChange::RemoveLocal(index) => {
            let index = usize::from(index);
            if index >= settings.local.len() {
                return Err("No such address".into());
            }
            let address = settings.local.remove(index);
            Ok(Some(format!("{address} on the local network no longer allowed")))
        }
        // Where the remote control is: the window's arrangement, said
        // nowhere (decision 68).
        SettingChange::Remote(place) => {
            settings.remote = place;
            Ok(None)
        }
        // Restoring the last session is the user's privacy, not a security
        // setting the strip says (requirement 20.24).
        SettingChange::RestoreSession(on) => {
            settings.restore = on;
            Ok(None)
        }
        // And what is kept of cookies (requirement 21.7).
        SettingChange::Cookies(mode) => {
            settings.cookies = match mode {
                protocol::settings::CookieMode::Keep => neoteletext::cookies::Mode::Keep,
                protocol::settings::CookieMode::Close => neoteletext::cookies::Mode::Close,
                protocol::settings::CookieMode::Block => neoteletext::cookies::Mode::Block,
            };
            Ok(None)
        }
        // And whether to check for updates (`installation-and-updates.md`
        // section 3.4).
        SettingChange::CheckUpdates(on) => {
            settings.check_updates = on;
            Ok((!on).then(|| UPDATES_OFF.to_owned()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Requirement 19.3: each change is checked by its setting's rule; a
    /// refused one changes nothing and says why; a security setting's
    /// change is said.
    #[test]
    fn each_setting_change_is_checked() {
        let mut settings = Settings::default();
        let mut change = |change: SettingChange| checked_change(&mut settings, change, Some("https://open.example/"));
        assert_eq!(change(SettingChange::Home("news.example/teletext/#302".into())), Ok(None));
        assert!(change(SettingChange::Home("javascript:alert(1)".into())).is_err());
        assert!(change(SettingChange::Home("neoteletext:settings".into())).is_err());
        assert_eq!(change(SettingChange::Volume(101)), Err("The volume is from 0 to 100".into()));
        assert_eq!(change(SettingChange::Volume(30)), Ok(None));
        assert_eq!(change(SettingChange::Sound(false)), Ok(None));
        assert_eq!(change(SettingChange::Proxy(ProxyView::None)), Ok(None), "no proxy before: nothing to say");
        assert!(change(SettingChange::Proxy(ProxyView::Http { host: "bad host".into(), port: 8080 })).is_err());
        assert!(change(SettingChange::Proxy(ProxyView::Socks5 { host: "p.example".into(), port: 0 })).is_err());
        let said = change(SettingChange::Proxy(ProxyView::Socks5 { host: " 127.0.0.1 ".into(), port: 1080 })).unwrap().unwrap();
        assert!(said.contains("socks5://127.0.0.1:1080"), "{said}");
        assert!(change(SettingChange::Proxy(ProxyView::None)).unwrap().is_some(), "removing it is said");
        for refused in ["93.184.216.34", "not an address", "8.8.8.8", "2001:4860::8888"] {
            assert_eq!(change(SettingChange::AddLocal(refused.into())), Err("Not an address on this computer or the local network".into()), "{refused}");
        }
        assert!(change(SettingChange::AddLocal("192.168.1.20".into())).unwrap().is_some());
        assert!(change(SettingChange::AddLocal("192.168.1.20".into())).is_err(), "once");
        for n in 0..20 {
            let _ = change(SettingChange::AddLocal(format!("10.0.0.{n}")));
        }
        assert!(change(SettingChange::RemoveLocal(16)).is_err());
        assert!(change(SettingChange::RemoveLocal(0)).unwrap().is_some());
        assert_eq!(change(SettingChange::HomeIsThisPage), Ok(None));
        // A privacy setting, not a security one: nothing said in the strip.
        assert_eq!(change(SettingChange::RestoreSession(false)), Ok(None));
        assert!(!settings.restore);
        assert_eq!(settings.home, "https://open.example/");
        assert_eq!((settings.volume, settings.sound, settings.proxy.is_none()), (30, false, true));
        assert_eq!(settings.local.len(), stored::MAX_LOCAL - 1);
        assert!(!settings.local.contains(&"192.168.1.20".parse().unwrap()));
        assert_eq!(checked_change(&mut settings, SettingChange::HomeIsThisPage, None), Err("No channel is open".into()));
    }

    /// Finding F-68: turning update checks off is a security setting's
    /// change, said in the strip and logged; turning them on is not.
    #[test]
    fn turning_update_checks_off_is_said() {
        let mut settings = Settings::default();
        let said = checked_change(&mut settings, SettingChange::CheckUpdates(false), None).unwrap();
        assert_eq!(said.as_deref(), Some(UPDATES_OFF));
        assert!(!settings.check_updates);
        assert_eq!(checked_change(&mut settings, SettingChange::CheckUpdates(true), None), Ok(None));
    }
}
