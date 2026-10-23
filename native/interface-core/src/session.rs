//! What the session keeps of each window (requirement 20.21,
//! `multiple-windows.md`): its place and size, bounded, and how many
//! windows there may be. `library.rs` re-exports it, where its importers
//! name it.

/// The most windows open at once (`multiple-windows.md`): more than anyone
/// arranges on a desktop, and few enough that every window's lines fit the
/// session file.
pub const MAX_WINDOWS: usize = 16;
/// The furthest a window's corner is kept from the origin, in logical
/// points (`history-and-bookmarks.md` section 15.1).
pub const MAX_COORDINATE: i32 = 32_768;
/// The largest window kept, in logical points.
pub const MAX_WINDOW_SIZE: u32 = 32_768;

/// The window's place and size, in logical points, as the session keeps
/// it (requirement 20.21).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WindowGeometry {
    /// Its outer left edge.
    pub x: i32,
    /// Its outer top edge.
    pub y: i32,
    /// Its inner width.
    pub width: u32,
    /// Its inner height.
    pub height: u32,
    /// Maximised.
    pub maximised: bool,
}

impl WindowGeometry {
    /// Whether every field is within its bound.
    #[must_use]
    pub fn is_plausible(&self) -> bool {
        (-MAX_COORDINATE..=MAX_COORDINATE).contains(&self.x)
            && (-MAX_COORDINATE..=MAX_COORDINATE).contains(&self.y)
            && (1..=MAX_WINDOW_SIZE).contains(&self.width)
            && (1..=MAX_WINDOW_SIZE).contains(&self.height)
    }
}
