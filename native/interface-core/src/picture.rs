//! The Tools menu's picture converter (`docs/picture-tools.md`): the choices
//! the picture panel offers, and what the trusted core says of a
//! conversion. The interface converts nothing and saves nothing: each choice
//! goes to the trusted core, which checks it and has a decoder convert the
//! picture (requirement 3.2).

use alloc::string::String;

/// The presentation levels the panel offers. Level 1 is left out: it encodes
/// exactly as Level 1.5, which the engine's converter says of it.
pub const LEVELS: [&str; 3] = ["1.5", "2.5", "3.5"];
/// The fits, as the engine's converter names them.
pub const FITS: [&str; 3] = ["Crop", "Contain", "Stretch"];
/// The nine alignments, row by row from the top left, as the panel lays them
/// out.
pub const ALIGNS: [&str; 9] = ["Top left", "Top", "Top right", "Left", "Centre", "Right", "Bottom left", "Bottom", "Bottom right"];
/// The middle of [`ALIGNS`], the default.
pub const CENTRE: u8 = 4;
/// The eight Level 1 colours, by colour number (EN 300 706 Table 26), the
/// backgrounds offered.
pub const BACKGROUNDS: [&str; 8] = ["Black", "Red", "Green", "Yellow", "Blue", "Magenta", "Cyan", "White"];
/// The longest status the trusted core sends: a sentence.
pub const MAX_STATUS: usize = 200;

/// The choices of a conversion, each an index into its list above.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PictureChoice {
    /// Into [`LEVELS`].
    pub level: u8,
    /// Into [`FITS`].
    pub fit: u8,
    /// Into [`ALIGNS`].
    pub align: u8,
    /// Into [`BACKGROUNDS`].
    pub background: u8,
    /// Hold Mosaics, at Level 1.5 only.
    pub hold_mosaics: bool,
}

impl Default for PictureChoice {
    /// Level 3.5, cropped, centred, on black: the engine's converter's own
    /// defaults.
    fn default() -> Self {
        Self { level: 2, fit: 0, align: CENTRE, background: 0, hold_mosaics: false }
    }
}

impl PictureChoice {
    /// Whether every index is within its list, and hold mosaics only at
    /// Level 1.5.
    #[must_use]
    pub fn is_valid(&self) -> bool {
        usize::from(self.level) < LEVELS.len()
            && usize::from(self.fit) < FITS.len()
            && usize::from(self.align) < ALIGNS.len()
            && usize::from(self.background) < BACKGROUNDS.len()
            && (!self.hold_mosaics || self.level == 0)
    }
}

/// What the trusted core says of the picture converter, for the panel.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PictureStatus {
    /// A converted page is shown and may be saved.
    pub converted: bool,
    /// One sentence, at most [`MAX_STATUS`] bytes: what is happening, what
    /// was made, why it failed, or where it was saved.
    pub text: String,
}
