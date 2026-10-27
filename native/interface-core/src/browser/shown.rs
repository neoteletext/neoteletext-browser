//! What the trusted core shows the window: the window's frame, the tabs,
//! the trusted strip's state and a refused certificate's reason. The
//! browser only draws them.

use alloc::string::String;

/// How the window's frame is drawn, as Chrome does on each platform: on
/// macOS the system's window buttons sit in the tab strip, so the tabs
/// start after them; elsewhere there is no system frame, and the browser
/// draws its own minimise, maximise and close buttons.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Frame {
    /// Where the tabs start: the width the system's window buttons take on macOS.
    pub left_inset: f64,
    /// Whether the browser draws its own minimise, maximise and close buttons.
    pub window_buttons: bool,
}

/// One tab as the trusted core describes it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TabInfo {
    /// The channel's title.
    pub title: String,
    /// An index into [`crate::theme::TAB_COLOURS`].
    pub colour: u8,
    /// The tab's channel, whose sound the interface plays.
    pub channel: u32,
    /// The user muted the tab.
    pub muted: bool,
    /// A private tab, which records no history (requirement 20.8).
    pub private: bool,
    /// The tab's address, as the trusted core wrote it, which its tip
    /// shows (decision 103).
    pub address: String,
}

/// What the trusted strip shows. The trusted core decides all of it; the
/// interface only draws it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[allow(clippy::struct_excessive_bools, reason = "independent facts the trusted core reports, each drawn on its own")]
pub struct StripState {
    /// The channel's address, as the address field shows it.
    pub address: String,
    /// The channel's host name, highlighted within the address.
    pub identity: String,
    /// The channel was fetched over an encrypted connection.
    pub encrypted: bool,
    /// The address is a file on this computer, not a site.
    pub local: bool,
    /// The tab has history to go back to.
    pub back: bool,
    /// The tab has history to go forward to.
    pub forward: bool,
    /// The channel is bookmarked.
    pub bookmarked: bool,
    /// The address is the browser's own start page, `neoteletext:home`.
    pub built_in: bool,
    /// The tab is private: it records no history (requirement 20.8).
    pub private: bool,
    /// What the strip says for a while, when not empty: a change of a
    /// security setting, a proxy that refused (requirements 19.3, 19.10).
    pub notice: String,
    /// The channel's certificate was refused, and why: the page area shows
    /// the certificate-error screen (`certificate-errors.md`, decision 83).
    pub refused: Option<CertificateProblem>,
}

/// Why a channel's certificate was refused (`certificate-errors.md`
/// section 3), as the fetcher's TLS library said it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CertificateProblem {
    /// It has expired.
    Expired,
    /// It is not valid yet.
    NotYetValid,
    /// No authority this browser trusts issued it.
    UnknownIssuer,
    /// It is for another host.
    WrongHost,
    /// Its issuer withdrew it.
    Revoked,
    /// It is not valid for another reason.
    Invalid,
    /// The secure handshake failed before the certificate was judged.
    Handshake,
}

impl CertificateProblem {
    /// Every kind, in the order of their codes on the wire.
    pub const ALL: [Self; 7] = [Self::Expired, Self::NotYetValid, Self::UnknownIssuer, Self::WrongHost, Self::Revoked, Self::Invalid, Self::Handshake];

    /// What the screen says failed, in the trusted core's words.
    #[must_use]
    pub const fn sentence(self) -> &'static str {
        match self {
            Self::Expired => "Its certificate has expired.",
            Self::NotYetValid => "Its certificate is not valid yet.",
            Self::UnknownIssuer => {
                "Its certificate was not issued by an authority this browser trusts; a certificate the server made for itself is refused so."
            }
            Self::WrongHost => "Its certificate is for another site.",
            Self::Revoked => "Its certificate has been withdrawn by the authority that issued it.",
            Self::Invalid => "Its certificate is not valid.",
            Self::Handshake => "The secure connection to it could not be made.",
        }
    }

    /// What may help, for the failures a person can mend.
    #[must_use]
    pub const fn hint(self) -> Option<&'static str> {
        match self {
            Self::Expired | Self::NotYetValid => Some("If this computer's date or time is wrong, correct it and try again."),
            _ => None,
        }
    }
}
