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
    /// Where the remote control is (`remote-control.md`, decision 68).
    Remote(RemotePlace),
}

/// The furthest a floating remote's corner may be from the window's, in
/// logical points: beyond any display today (an 8K display is 7 680 wide
/// at its natural scale), and a bound on what the settings file holds.
pub const MAX_REMOTE_PLACE: u16 = 16_384;

/// Where the remote control is (`remote-control.md` section 2).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RemoteMode {
    /// Put away: only its handle shows.
    #[default]
    Hidden,
    /// Along the window's right edge.
    Docked,
    /// Over the page area, where the user put it.
    Floating,
}

/// The remote control's place: its mode, and a floating remote's top left
/// corner in logical points, at most [`MAX_REMOTE_PLACE`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RemotePlace {
    /// Hidden, docked or floating.
    pub mode: RemoteMode,
    /// The floating remote's left edge.
    pub x: u16,
    /// Its top edge.
    pub y: u16,
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
