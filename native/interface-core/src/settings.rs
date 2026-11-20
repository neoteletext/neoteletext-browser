//! The settings as the trusted core shows them and the changes the
//! settings panel and the menus ask for (`settings-home-console-proxy.md`
//! section 1.3).
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
#[allow(clippy::struct_excessive_bools, reason = "independent settings the trusted core keeps, each a switch of its own")]
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
    /// Channels may ask to know the viewer: "Allow", or "Never", which
    /// declines every request to make a key (VS-112, open question Q16).
    pub channels_may_know: bool,
    /// Pages are drawn with the 4:3 aspect correction (REQ-737).
    pub aspect: bool,
    /// A mouse wheel's notches are taken the other way (REQ-6351).
    pub wheel_inverted: bool,
    /// A ReGIS picture is contained in the page area, its page laid out in
    /// the picture's shape; else contained to the teletext page (REQ-6363).
    pub regis_contain: bool,
    /// Drawings with a paint rate appear gradually and a NAPLPS WAIT
    /// delays what follows it; off, everything appears at once
    /// (animation.md ANIM-15, DEC-6525 item 27).
    pub paint_gradually: bool,
    /// A channel's tabs share one connection to its server; off, each tab
    /// has its own (REQ-6982).
    pub share_connections: bool,
    /// Tabs not looked at this run show snapshots (REQ-6886 item 9).
    pub tab_pictures: bool,
    /// The file name of the font the viewer chose for alphabet 0 of every
    /// ReGIS picture, or empty for the one compiled in (renderer parity
    /// PAR-35).
    pub regis_font: String,
    /// The file name of the stroke font the viewer chose for every NAPLPS
    /// picture, or empty for the one compiled in.
    pub naplps_font: String,
    /// Why the last change was refused, if it was.
    pub refusal: Option<String>,
    /// On Windows, the command an administrator runs once so that channels
    /// on this computer can be reached (decision 78, TODO-1251); none
    /// elsewhere.
    pub loopback_exemption: Option<String>,
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
    /// Channels may ask to know the viewer, or never (VS-112).
    ChannelsMayKnow(bool),
    /// Where the remote control is (`remote-control.md`, decision 68).
    Remote(RemotePlace),
    /// Check for updates, or not (`installation-and-updates.md` section 3.4).
    CheckUpdates(bool),
    /// The 4:3 aspect correction on or off (REQ-737).
    Aspect(bool),
    /// A mouse wheel's direction inverted, or as it comes (REQ-6351).
    WheelInverted(bool),
    /// A ReGIS picture contained in the page area (true) or to the
    /// teletext page (REQ-6363).
    RegisContain(bool),
    /// Drawings painted gradually (true) or at once (animation.md
    /// ANIM-15).
    PaintGradually(bool),
    /// A channel's tabs share one connection (true) or each has its own
    /// (REQ-6982).
    ShareConnections(bool),
    /// Tab pictures on or off (REQ-6886 item 9).
    TabPictures(bool),
    /// Pictures of this language drawn in the font compiled in again, not
    /// one the viewer chose (renderer parity PAR-35). A font is chosen
    /// through the file dialog, not as a change.
    BuiltInFont(crate::canvas::PictureLanguage),
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

/// Updates, as the settings panel shows them (`installation-and-updates.md`
/// section 3.4), in the trusted core's words.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct UpdateView {
    /// The version running: the installed one, or the one built.
    pub version: String,
    /// What the trusted core says of updates.
    pub status: String,
    /// The version the user may install, when there is one.
    pub offer: Option<String>,
    /// The browser checks for updates.
    pub checking: bool,
}
