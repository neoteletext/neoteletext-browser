//! Presenting the interface's frames (DEC-7168, replacing `softbuffer`):
//! the pixels the interface drew on the processor, handed to its window by
//! each platform's own path, limited to the boxes that changed wherever the
//! platform takes them (DEC-1255):
//!
//! - X11: the window's own connection to the X server (Xlib's, as winit
//!   opened it), and an image the server shares with the interface through
//!   the shared-memory extension (`MIT-SHM`, `ShmPutImage` for each box),
//!   or sent in the request (`PutImage`, box by box) where the server
//!   cannot share memory;
//! - Wayland: two shared-memory buffers (`wl_shm`) attached in turn, each
//!   box marked damaged (`wl_surface.damage_buffer`);
//! - macOS: a Core Animation layer over the window's view, whose contents
//!   are set to an image of the frame (`CGImage`); Core Animation takes
//!   the whole image, so the boxes do not limit what it copies;
//! - Windows: GDI's `SetDIBitsToDevice`, box by box, into the window.
//!
//! A pixel is a `u32`: `0x00RRGGBB`, the top byte unused. The buffer a
//! frame is drawn into says its age: 1 when it holds the last frame
//! presented, so only the boxes that changed need drawing again; 0 (or 2,
//! on Wayland's other buffer) when it holds less, and every pixel must be
//! drawn.
//!
//! Each present returns before the frame is shown: the platform shows it
//! at its next composition, and a newer frame replaces one not yet shown
//! (requirement 18.14, REQ-RCH-004). Where the platform reads the buffer
//! after the present returns (X11's shared memory, Wayland's buffers), the
//! next frame waits for it to be done before it is drawn into.

use std::num::NonZeroU32;

use raw_window_handle::{HasDisplayHandle, HasWindowHandle, RawDisplayHandle, RawWindowHandle};

#[cfg(target_os = "macos")]
mod macos;
#[cfg(all(unix, not(target_vendor = "apple")))]
mod shared_memory;
#[cfg(all(unix, not(target_vendor = "apple")))]
mod wayland;
#[cfg(windows)]
mod windows;
#[cfg(all(unix, not(target_vendor = "apple")))]
mod x11;

/// The widest and tallest window presented, in pixels: X11's coordinates
/// are 16-bit, and no display is larger. A larger size is refused rather
/// than allocated.
pub const MAX_SIDE: u32 = 16_384;
/// Bytes in a pixel.
pub const BYTES_PER_PIXEL: usize = 4;

/// A box of the window that changed, in pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    /// Its left edge.
    pub x: u32,
    /// Its top edge.
    pub y: u32,
    /// Its width.
    pub width: NonZeroU32,
    /// Its height.
    pub height: NonZeroU32,
}

/// Why a window could not be presented to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Error(String);

impl Error {
    fn new(text: impl Into<String>) -> Self {
        Self(text.into())
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for Error {}

/// The platform's way of presenting to one window.
enum Platform {
    #[cfg(all(unix, not(target_vendor = "apple")))]
    X11(x11::Surface),
    #[cfg(all(unix, not(target_vendor = "apple")))]
    Wayland(wayland::Surface),
    #[cfg(target_os = "macos")]
    MacOs(macos::Surface),
    #[cfg(windows)]
    Windows(windows::Surface),
}

/// What presents to one window, which it keeps for as long as it lives.
pub struct Surface<W> {
    platform: Platform,
    width: u32,
    height: u32,
    /// Declared last, so the window outlives what presents to it.
    _window: W,
}

impl<W: HasWindowHandle + HasDisplayHandle> Surface<W> {
    /// Prepares to present to `window`.
    ///
    /// # Errors
    /// The window's platform is not one presented to, or its display
    /// refused what presenting needs.
    pub fn new(window: W) -> Result<Self, Error> {
        let display = window.display_handle().map_err(|error| Error::new(format!("the window's display: {error}")))?.as_raw();
        let handle = window.window_handle().map_err(|error| Error::new(format!("the window: {error}")))?.as_raw();
        let platform = Self::platform(display, handle)?;
        Ok(Self { platform, width: 0, height: 0, _window: window })
    }

    fn platform(display: RawDisplayHandle, handle: RawWindowHandle) -> Result<Platform, Error> {
        match (display, handle) {
            #[cfg(all(unix, not(target_vendor = "apple")))]
            (RawDisplayHandle::Xlib(_) | RawDisplayHandle::Xcb(_), RawWindowHandle::Xlib(_) | RawWindowHandle::Xcb(_)) => {
                Ok(Platform::X11(x11::Surface::new(display, handle)?))
            }
            #[cfg(all(unix, not(target_vendor = "apple")))]
            (RawDisplayHandle::Wayland(display), RawWindowHandle::Wayland(window)) => Ok(Platform::Wayland(wayland::Surface::new(display, window)?)),
            #[cfg(target_os = "macos")]
            (_, RawWindowHandle::AppKit(window)) => Ok(Platform::MacOs(macos::Surface::new(window)?)),
            #[cfg(windows)]
            (_, RawWindowHandle::Win32(window)) => Ok(Platform::Windows(windows::Surface::new(window)?)),
            _ => Err(Error::new("the window's platform is not one presented to")),
        }
    }
}

impl<W> Surface<W> {
    /// Sizes the frames that follow to `width` by `height` pixels.
    ///
    /// # Errors
    /// A side larger than [`MAX_SIDE`], or a display that refused the
    /// memory for it.
    pub fn resize(&mut self, width: NonZeroU32, height: NonZeroU32) -> Result<(), Error> {
        let (width, height) = (width.get(), height.get());
        if width > MAX_SIDE || height > MAX_SIDE {
            return Err(Error::new(format!("a window of {width} by {height} pixels")));
        }
        if (width, height) == (self.width, self.height) {
            return Ok(());
        }
        match &mut self.platform {
            #[cfg(all(unix, not(target_vendor = "apple")))]
            Platform::X11(surface) => surface.resize(width, height)?,
            #[cfg(all(unix, not(target_vendor = "apple")))]
            Platform::Wayland(surface) => surface.resize(width, height),
            #[cfg(target_os = "macos")]
            Platform::MacOs(surface) => surface.resize(width, height),
            #[cfg(windows)]
            Platform::Windows(surface) => surface.resize(width, height),
        }
        self.width = width;
        self.height = height;
        Ok(())
    }

    /// The buffer the next frame is drawn into, once the platform is done
    /// reading it.
    ///
    /// # Errors
    /// The surface was never sized, or the display failed.
    pub fn buffer_mut(&mut self) -> Result<Buffer<'_>, Error> {
        if self.width == 0 || self.height == 0 {
            return Err(Error::new("the surface has no size"));
        }
        let age = self.platform.prepare()?;
        let expected = pixel_count(self.width, self.height);
        let held = self.platform.pixels().len();
        if held != expected {
            return Err(Error::new(format!("the buffer holds {held} pixels, not {expected}")));
        }
        Ok(Buffer { platform: &mut self.platform, age, width: self.width, height: self.height })
    }
}

impl Platform {
    /// Waits until the next buffer may be drawn into, and returns its age.
    #[cfg_attr(
        any(target_os = "macos", windows),
        expect(clippy::unnecessary_wraps, reason = "one signature on every platform: on Linux the wait for the display server can fail")
    )]
    fn prepare(&mut self) -> Result<u8, Error> {
        match self {
            #[cfg(all(unix, not(target_vendor = "apple")))]
            Self::X11(surface) => surface.prepare(),
            #[cfg(all(unix, not(target_vendor = "apple")))]
            Self::Wayland(surface) => surface.prepare(),
            #[cfg(target_os = "macos")]
            Self::MacOs(surface) => Ok(surface.prepare()),
            #[cfg(windows)]
            Self::Windows(surface) => Ok(surface.prepare()),
        }
    }

    fn pixels(&self) -> &[u32] {
        match self {
            #[cfg(all(unix, not(target_vendor = "apple")))]
            Self::X11(surface) => surface.pixels(),
            #[cfg(all(unix, not(target_vendor = "apple")))]
            Self::Wayland(surface) => surface.pixels(),
            #[cfg(target_os = "macos")]
            Self::MacOs(surface) => surface.pixels(),
            #[cfg(windows)]
            Self::Windows(surface) => surface.pixels(),
        }
    }

    fn pixels_mut(&mut self) -> &mut [u32] {
        match self {
            #[cfg(all(unix, not(target_vendor = "apple")))]
            Self::X11(surface) => surface.pixels_mut(),
            #[cfg(all(unix, not(target_vendor = "apple")))]
            Self::Wayland(surface) => surface.pixels_mut(),
            #[cfg(target_os = "macos")]
            Self::MacOs(surface) => surface.pixels_mut(),
            #[cfg(windows)]
            Self::Windows(surface) => surface.pixels_mut(),
        }
    }

    fn present(&mut self, damage: &[Rect]) -> Result<(), Error> {
        match self {
            #[cfg(all(unix, not(target_vendor = "apple")))]
            Self::X11(surface) => surface.present(damage),
            #[cfg(all(unix, not(target_vendor = "apple")))]
            Self::Wayland(surface) => surface.present(damage),
            #[cfg(target_os = "macos")]
            Self::MacOs(surface) => surface.present(damage),
            #[cfg(windows)]
            Self::Windows(surface) => surface.present(damage),
        }
    }
}

/// The pixels of a window `width` by `height`.
fn pixel_count(width: u32, height: u32) -> usize {
    usize::try_from(u64::from(width).saturating_mul(u64::from(height))).unwrap_or(usize::MAX)
}

/// The boxes of `damage` that lie within a window `width` by `height`,
/// cut to it; a box wholly outside is left out.
fn within(damage: &[Rect], width: u32, height: u32) -> Vec<Rect> {
    damage
        .iter()
        .filter_map(|rect| {
            let right = rect.x.saturating_add(rect.width.get()).min(width);
            let bottom = rect.y.saturating_add(rect.height.get()).min(height);
            Some(Rect { x: rect.x, y: rect.y, width: NonZeroU32::new(right.checked_sub(rect.x)?)?, height: NonZeroU32::new(bottom.checked_sub(rect.y)?)? })
        })
        .collect()
}

/// The buffer one frame is drawn into, and presented from.
pub struct Buffer<'a> {
    platform: &'a mut Platform,
    age: u8,
    width: u32,
    height: u32,
}

impl Buffer<'_> {
    /// How many frames ago its pixels were presented: 1 when it holds the
    /// last frame, 0 when it holds nothing to keep.
    #[must_use]
    pub const fn age(&self) -> u8 {
        self.age
    }

    /// Presents the frame, only the boxes of `damage` where the platform
    /// takes them; boxes reaching outside the window are cut to it.
    ///
    /// # Errors
    /// The display failed.
    pub fn present_with_damage(self, damage: &[Rect]) -> Result<(), Error> {
        let damage = within(damage, self.width, self.height);
        self.platform.present(&damage)
    }
}

impl std::ops::Deref for Buffer<'_> {
    type Target = [u32];

    fn deref(&self) -> &[u32] {
        self.platform.pixels()
    }
}

impl std::ops::DerefMut for Buffer<'_> {
    fn deref_mut(&mut self) -> &mut [u32] {
        self.platform.pixels_mut()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: u32, y: u32, width: u32, height: u32) -> Rect {
        Rect { x, y, width: NonZeroU32::new(width).unwrap(), height: NonZeroU32::new(height).unwrap() }
    }

    /// Boxes are cut to the window, and one wholly outside it is left out,
    /// so no platform is asked to copy pixels it does not hold.
    #[test]
    fn damage_is_cut_to_the_window() {
        let damage = [rect(0, 0, 10, 10), rect(95, 45, 10, 10), rect(100, 0, 5, 5), rect(0, 50, 5, 5), rect(u32::MAX - 1, 0, 5, 5)];
        assert_eq!(within(&damage, 100, 50), [rect(0, 0, 10, 10), rect(95, 45, 5, 5)]);
    }
}
