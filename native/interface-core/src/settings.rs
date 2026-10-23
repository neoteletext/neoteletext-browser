//! The settings as the trusted core shows them and the changes the
//! settings panel asks for (`settings-home-console-proxy.md` section 1.3).
//! The interface acts on none of them (requirement 3.6): each change is
//! strip input the trusted core checks.

use alloc::string::String;
use alloc::vec::Vec;

/// A proxy, as the panel shows it and asks for it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProxyView {
    /// No proxy: connections go direct.
    None,
    /// An HTTP proxy.
    Http {
        /// Its host.
        host: String,
        /// Its port.
        port: u16,
    },
    /// A SOCKS5 proxy.
    Socks5 {
        /// Its host.
        host: String,
        /// Its port.
        port: u16,
    },
}

/// The settings as the panel shows them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SettingsView {
    /// The home address.
    pub home: String,
    /// Page sounds play.
    pub sound: bool,
    /// Their volume, 0 to 100.
    pub volume: u8,
    /// The proxy.
    pub proxy: ProxyView,
    /// The local addresses allowed, as text.
    pub local: Vec<String>,
    /// The last session is restored at start (requirement 20.24).
    pub restore_session: bool,
    /// What is kept of cookies (requirement 21.7).
    pub cookies: CookieMode,
    /// Why the last change was refused, if it was.
    pub refusal: Option<String>,
}

/// One change the panel asks for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SettingChange {
    /// The home address, as typed.
    Home(String),
    /// The open channel's address as the home.
    HomeIsThisPage,
    /// Page sounds on or off.
    Sound(bool),
    /// The volume.
    Volume(u8),
    /// The proxy, as typed.
    Proxy(ProxyView),
    /// A local address to allow, as typed.
    AddLocal(String),
    /// Stop allowing the local address at this place in the list.
    RemoveLocal(u8),
    /// Restore the last session at start, or open the home address
    /// (requirement 20.24).
    RestoreSession(bool),
    /// What is kept of cookies (requirement 21.7).
    Cookies(CookieMode),
    /// Check for updates, or not (`installation-and-updates.md` section 3.4).
    CheckUpdates(bool),
}

/// Updates, as the settings panel shows them (`installation-and-updates.md`
/// section 3.4), in the trusted core's words.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct UpdateView {
    /// The version running, or empty when the browser was not installed.
    pub version: String,
    /// What the trusted core says of updates.
    pub status: String,
    /// The version the user may install, when there is one.
    pub offer: Option<String>,
    /// The browser checks for updates.
    pub checking: bool,
}

/// What is kept of cookies (`cookies.md` section 4.2).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CookieMode {
    /// Persistent cookies between runs.
    #[default]
    Keep,
    /// None once the browser closes.
    Close,
    /// None stored or sent.
    Block,
}
