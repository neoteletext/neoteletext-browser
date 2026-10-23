//! The open page's controls (`page-controls.md`, decision 67): Fastext's
//! coloured keys and Index, reveal, hold, mix and sub-pages, as a
//! teletext set's remote offers them. The trusted core says which of them
//! the open page has and which are on; the interface asks for each as
//! strip input.

/// The highest sub-code a page can carry: EN 300 706 clause 9.3.1.3's
/// S4, S3, S2 and S1 at their most, 3F7F, which a link uses to mean any
/// sub-page.
pub const ANY_SUBCODE: u16 = 0x3F7F;

/// The most sub-pages listed for a page: the engine's
/// `neoteletext_core::viewer::MAX_LISTED_SUBPAGES`, which this crate does
/// not depend on; the decoder asserts the two agree.
pub const MAX_SUBPAGES: usize = 64;

/// Fastext's keys: the four coloured keys and Index (the engine's
/// REQ-NAV-003).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FastextKey {
    /// The red key, the page's first link.
    Red,
    /// The green key, its second.
    Green,
    /// The yellow key, its third.
    Yellow,
    /// The cyan key, its fourth.
    Cyan,
    /// Index, the page's sixth link (X/27/0's link 5).
    Index,
}

impl FastextKey {
    /// Every key, in the order of a page's links.
    pub const ALL: [Self; 5] = [Self::Red, Self::Green, Self::Yellow, Self::Cyan, Self::Index];

    /// Its place in [`Self::ALL`] and in a page's links.
    #[must_use]
    pub const fn index(self) -> usize {
        match self {
            Self::Red => 0,
            Self::Green => 1,
            Self::Yellow => 2,
            Self::Cyan => 3,
            Self::Index => 4,
        }
    }

    /// The key at `index` of [`Self::ALL`].
    #[must_use]
    pub fn at(index: usize) -> Option<Self> {
        Self::ALL.get(index).copied()
    }

    /// Its name, for the tips and the remote control.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Red => "Red",
            Self::Green => "Green",
            Self::Yellow => "Yellow",
            Self::Cyan => "Cyan",
            Self::Index => "Index",
        }
    }
}

/// What the user asks of the open page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PageControl {
    /// Follow a coloured key or Index.
    Fastext(FastextKey),
    /// Show concealed text, or hide it again.
    Reveal,
    /// Stop the page changing, or let it change again.
    Hold,
    /// Draw the cells' backgrounds transparent, or not.
    Mix,
    /// The next sub-page (true) or the one before.
    SubPage(bool),
    /// Back to the latest sub-page as it arrives.
    Latest,
}

/// The open page's controls, as the trusted core sends them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PageControls {
    /// The open page, or none.
    pub page: Option<u16>,
    /// Which of the coloured keys and Index lead somewhere now, with row 24
    /// shown (the engine's REQ-SEC-024), in [`FastextKey::ALL`]'s order.
    pub fastext: [bool; 5],
    /// Concealed text is shown.
    pub reveal: bool,
    /// The page is held.
    pub hold: bool,
    /// Mix is on.
    pub mix: bool,
    /// How many sub-pages the page has: 0 or 1 when it does not rotate.
    pub subpages: u8,
    /// The place of the sub-page shown among them, from 0, when one was
    /// chosen; none while the latest is shown as it arrives.
    pub chosen: Option<u8>,
    /// The sub-code of the sub-page shown, for the address and the tips.
    pub subcode: u16,
}
