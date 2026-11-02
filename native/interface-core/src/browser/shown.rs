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
    /// The tab's address, as the trusted core wrote it and cut to
    /// [`crate::library::MAX_SHOWN_ADDRESS`], which its tip shows (decision
    /// 103).
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
    /// A redirect is taking the tab out of its channel: the complete address
    /// it leads to, which the page area shows in place of the pages for a
    /// second (REQ-1686); empty otherwise.
    pub redirecting: String,
    /// The channel refused what the tab selected: the page area says so, in
    /// the browser's own words (DOC-1961, REQ-1481).
    pub denied: Option<Denied>,
    /// The tab failed: the page area says so, in the browser's own words
    /// (REQ-683).
    pub failed: Option<Failed>,
}

/// Why a tab shows nothing of its channel: said by the trusted core, in the
/// browser's own words, since a helper or a server can only name which.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Failed {
    /// The tab's decoder broke its protocol and was closed (REQ-683).
    Decoder,
    /// The tab's fetcher broke its protocol and was closed (REQ-683).
    Fetcher,
    /// The address's `channel.ttx` was refused or could not be fetched:
    /// the real HTTP status, or none when nothing answered (REQ-708).
    Channel(Option<u16>),
    /// The page open was refused or could not be fetched: the real HTTP
    /// status, or none when nothing answered. A status that a DENIED record
    /// carries reads as that record does (REQ-708).
    Page(Option<u16>),
}

/// Why a channel refused a selection: the real HTTP status a DENIED record
/// carries (DOC-1961).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Denied {
    /// 403: the channel does not allow the page.
    Forbidden,
    /// 404: the channel has no such page.
    NotFound,
    /// 429: a limit of the channel was reached.
    TooMany,
    /// 503: the channel's program is not answering.
    Unavailable,
}

impl Denied {
    /// The refusal a status means; none for a status a DENIED record does
    /// not carry.
    #[must_use]
    pub const fn of_status(status: u16) -> Option<Self> {
        match status {
            403 => Some(Self::Forbidden),
            404 => Some(Self::NotFound),
            429 => Some(Self::TooMany),
            503 => Some(Self::Unavailable),
            _ => None,
        }
    }

    /// Its status number.
    #[must_use]
    pub const fn status(self) -> u16 {
        match self {
            Self::Forbidden => 403,
            Self::NotFound => 404,
            Self::TooMany => 429,
            Self::Unavailable => 503,
        }
    }
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
