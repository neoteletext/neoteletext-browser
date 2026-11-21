//! X11: the frame put into the window on winit's own connection to the X
//! server, box by box. With the shared-memory extension (`MIT-SHM` 1.2,
//! `ShmAttachFd` and `ShmPutImage`) the server reads the pixels from memory
//! the interface shares with it, and the next frame waits for the server
//! to have read them (a `GetInputFocus` round trip after the puts, which
//! the server answers in order); without it each box's rows go in
//! `PutImage` requests (the X Window System Protocol, section 9), split to
//! the server's largest request.
//!
//! The window's visual must take a pixel as the interface draws it: 32
//! bits in the server's byte order, red in bits 16 to 23, green 8 to 15,
//! blue 0 to 7, at depth 24 or 32 (`TrueColor` or `DirectColor`), as every
//! desktop X server offers by default.
#![allow(unsafe_code, reason = "Xlib's display gives its XCB connection only through a C function and a raw pointer: two blocks, each with its reason")]

use std::os::fd::AsFd;

use raw_window_handle::{RawDisplayHandle, RawWindowHandle};
use x11rb::connection::{Connection, RequestConnection, SequenceNumber};
use x11rb::cookie::Cookie;
use x11rb::protocol::shm::{self, ConnectionExt as _};
use x11rb::protocol::xproto::{self, ConnectionExt as _, ImageFormat, ImageOrder, VisualClass};
use x11rb::xcb_ffi::XCBConnection;

use crate::shared_memory::{Kind, Mapping};
use crate::{BYTES_PER_PIXEL, Error, Rect, pixel_count};

/// The pixel the interface draws, as the visual's masks give it.
const RED_MASK: u32 = 0x00FF_0000;
const GREEN_MASK: u32 = 0x0000_FF00;
const BLUE_MASK: u32 = 0x0000_00FF;
/// The bits a pixel takes in the server's image format.
const BITS_PER_PIXEL: u8 = 32;
/// The depths such a visual may have: 24 bits of colour, or 32 with the
/// top byte, which the interface leaves 0.
const DEPTHS: [u8; 2] = [24, 32];
/// The least version of `MIT-SHM` that takes a descriptor (`ShmAttachFd`).
const SHM_FD_VERSION: (u16, u16) = (1, 2);
/// The bytes of a `PutImage` request before its data (the protocol's
/// encoding of `PutImage`: six 4-byte words).
const PUT_IMAGE_HEADER_BYTES: usize = 24;

fn failed(what: &str, error: impl std::fmt::Display) -> Error {
    Error::new(format!("{what}: {error}"))
}

/// Where the frame is drawn.
enum Pixels {
    /// Shared with the server: the memory, its segment's number, and the
    /// round trip that says the server is done with it.
    Shared { mapping: Option<(Mapping, shm::Seg)>, done: Option<SequenceNumber> },
    /// The interface's own, sent in requests.
    Wire(Vec<u32>),
}

/// Presenting to one X11 window.
pub struct Surface {
    connection: XCBConnection,
    window: xproto::Window,
    gc: xproto::Gcontext,
    depth: u8,
    pixels: Pixels,
    width: u32,
    height: u32,
    /// The buffer holds the last frame presented.
    presented: bool,
}

impl Surface {
    pub fn new(display: RawDisplayHandle, window: RawWindowHandle) -> Result<Self, Error> {
        let raw = match display {
            RawDisplayHandle::Xlib(xlib) => {
                let display = xlib.display.ok_or_else(|| Error::new("the X display has no Xlib connection"))?;
                let library = x11_dl::xlib_xcb::Xlib_xcb::open().map_err(|error| failed("libX11-xcb", error))?;
                // SAFETY: the display is winit's open Xlib display, which
                // outlives its windows and so this surface; the function
                // only reads the display's XCB connection.
                unsafe { (library.XGetXCBConnection)(display.as_ptr().cast()) }
            }
            RawDisplayHandle::Xcb(xcb) => xcb.connection.ok_or_else(|| Error::new("the X display has no connection"))?.as_ptr(),
            _ => return Err(Error::new("not an X11 display")),
        };
        if raw.is_null() {
            return Err(Error::new("the X display has no XCB connection"));
        }
        // SAFETY: `raw` is the live XCB connection of winit's display,
        // which outlives this surface; it is not closed when the surface
        // drops (`should_drop` false), as winit owns it.
        let connection = unsafe { XCBConnection::from_raw_xcb_connection(raw, false) }.map_err(|error| failed("the X connection", error))?;
        let (window, visual) = match window {
            RawWindowHandle::Xlib(xlib) => {
                (u32::try_from(xlib.window).map_err(|_| Error::new("the X window"))?, u32::try_from(xlib.visual_id).ok().filter(|&visual| visual != 0))
            }
            RawWindowHandle::Xcb(xcb) => (xcb.window.get(), xcb.visual_id.map(std::num::NonZeroU32::get)),
            _ => return Err(Error::new("not an X11 window")),
        };
        let geometry = connection
            .get_geometry(window)
            .map_err(|error| failed("the window's geometry", error))?
            .reply()
            .map_err(|error| failed("the window's geometry", error))?;
        let visual = match visual {
            Some(visual) => visual,
            None => {
                connection
                    .get_window_attributes(window)
                    .map_err(|error| failed("the window's visual", error))?
                    .reply()
                    .map_err(|error| failed("the window's visual", error))?
                    .visual
            }
        };
        if !takes_our_pixels(connection.setup(), visual, geometry.depth) {
            return Err(Error::new(format!("the window's visual 0x{visual:x} at depth {} does not take 32-bit RGB pixels", geometry.depth)));
        }
        let gc = connection.generate_id().map_err(|error| failed("a graphics context", error))?;
        connection
            .create_gc(gc, window, &xproto::CreateGCAux::new().graphics_exposures(0))
            .map_err(|error| failed("a graphics context", error))?
            .check()
            .map_err(|error| failed("a graphics context", error))?;
        let pixels = if shares_memory(&connection) { Pixels::Shared { mapping: None, done: None } } else { Pixels::Wire(Vec::new()) };
        Ok(Self { connection, window, gc, depth: geometry.depth, pixels, width: 0, height: 0, presented: false })
    }

    pub fn resize(&mut self, width: u32, height: u32) -> Result<(), Error> {
        let count = pixel_count(width, height);
        self.wait()?;
        if let Pixels::Shared { mapping, .. } = &mut self.pixels {
            let fits = mapping.as_ref().is_some_and(|(memory, _)| memory.bytes() >= count.saturating_mul(BYTES_PER_PIXEL));
            if !fits {
                let shared = share(&self.connection, count);
                // A server that names the extension but cannot map the
                // memory, or memory the sandbox will not make: the pixels
                // go in requests instead, as they would with no extension.
                let replaced = if let Ok(shared) = shared { mapping.replace(shared) } else { mapping.take() };
                if let Some((_, old)) = replaced {
                    self.connection.shm_detach(old).map_err(|error| failed("the old shared segment", error))?.ignore_error();
                }
                if mapping.is_none() {
                    self.pixels = Pixels::Wire(Vec::new());
                }
            }
        }
        if let Pixels::Wire(pixels) = &mut self.pixels {
            pixels.resize(count, 0);
        }
        self.width = width;
        self.height = height;
        self.presented = false;
        Ok(())
    }

    /// Waits for the server to have read the last frame.
    fn wait(&mut self) -> Result<(), Error> {
        if let Pixels::Shared { done, .. } = &mut self.pixels
            && let Some(sequence) = done.take()
        {
            Cookie::<_, xproto::GetInputFocusReply>::new(&self.connection, sequence).reply().map_err(|error| failed("waiting for the X server", error))?;
        }
        Ok(())
    }

    pub fn prepare(&mut self) -> Result<u8, Error> {
        self.wait()?;
        Ok(u8::from(self.presented))
    }

    pub fn pixels(&self) -> &[u32] {
        let count = pixel_count(self.width, self.height);
        match &self.pixels {
            Pixels::Shared { mapping: Some((memory, _)), .. } => memory.pixels(count),
            Pixels::Shared { mapping: None, .. } => &[],
            Pixels::Wire(pixels) => pixels,
        }
    }

    pub fn pixels_mut(&mut self) -> &mut [u32] {
        let count = pixel_count(self.width, self.height);
        match &mut self.pixels {
            Pixels::Shared { mapping: Some((memory, _)), .. } => memory.pixels_mut(count),
            Pixels::Shared { mapping: None, .. } => &mut [],
            Pixels::Wire(pixels) => pixels,
        }
    }

    pub fn present(&mut self, damage: &[Rect]) -> Result<(), Error> {
        let (Ok(total_width), Ok(total_height)) = (u16::try_from(self.width), u16::try_from(self.height)) else {
            return Err(Error::new("the window is larger than X11's coordinates"));
        };
        let format = u8::from(ImageFormat::Z_PIXMAP);
        match &mut self.pixels {
            Pixels::Shared { mapping: Some((_, segment)), done } => {
                for rect in damage {
                    let (x, y, width, height) = sides(rect)?;
                    let (dst_x, dst_y) = (i16::try_from(x).map_err(|_| Error::new("a box"))?, i16::try_from(y).map_err(|_| Error::new("a box"))?);
                    self.connection
                        .shm_put_image(
                            self.window,
                            self.gc,
                            total_width,
                            total_height,
                            x,
                            y,
                            width,
                            height,
                            dst_x,
                            dst_y,
                            self.depth,
                            format,
                            false,
                            *segment,
                            0,
                        )
                        .map_err(|error| failed("putting the frame", error))?
                        .ignore_error();
                }
                *done = Some(self.connection.get_input_focus().map_err(|error| failed("the X server", error))?.sequence_number());
            }
            Pixels::Shared { mapping: None, .. } => return Err(Error::new("the surface has no size")),
            Pixels::Wire(pixels) => {
                let most = self.connection.maximum_request_bytes();
                for rect in damage {
                    put_rows(&self.connection, self.window, self.gc, self.depth, pixels, self.width, rect, most)?;
                }
            }
        }
        self.connection.flush().map_err(|error| failed("the X server", error))?;
        self.presented = true;
        Ok(())
    }
}

/// Memory for `count` pixels, and room for the window to grow by half
/// before the next, shared with the server by descriptor.
fn share(connection: &XCBConnection, count: usize) -> Result<(Mapping, shm::Seg), Error> {
    let memory = Mapping::new(Kind::Posix, count.saturating_add(count / 2))?;
    let segment = connection.generate_id().map_err(|error| failed("a shared segment", error))?;
    let descriptor = memory.as_fd().try_clone_to_owned().map_err(|error| failed("the shared memory's descriptor", error))?;
    connection
        .shm_attach_fd(segment, descriptor, true)
        .map_err(|error| failed("sharing memory with the X server", error))?
        .check()
        .map_err(|error| failed("sharing memory with the X server", error))?;
    Ok((memory, segment))
}

/// A box's place and size in X11's 16-bit coordinates.
fn sides(rect: &Rect) -> Result<(u16, u16, u16, u16), Error> {
    let side = |value: u32| u16::try_from(value).map_err(|_| Error::new("a box beyond X11's coordinates"));
    Ok((side(rect.x)?, side(rect.y)?, side(rect.width.get())?, side(rect.height.get())?))
}

/// Puts one box's rows into the window in `PutImage` requests, as many rows
/// a request as fit in the server's largest.
#[expect(clippy::too_many_arguments, reason = "one request's every field, from the surface's parts borrowed apart")]
fn put_rows(
    connection: &XCBConnection,
    window: xproto::Window,
    gc: xproto::Gcontext,
    depth: u8,
    pixels: &[u32],
    stride: u32,
    rect: &Rect,
    most: usize,
) -> Result<(), Error> {
    let (x, y, width, height) = sides(rect)?;
    let row_bytes = usize::from(width).saturating_mul(BYTES_PER_PIXEL);
    let rows_a_request = most.saturating_sub(PUT_IMAGE_HEADER_BYTES).checked_div(row_bytes).unwrap_or(0).clamp(1, usize::from(height));
    let stride = usize::try_from(stride).map_err(|_| Error::new("the window"))?;
    let mut data = Vec::with_capacity(rows_a_request.saturating_mul(row_bytes));
    let mut row = 0;
    while row < height {
        let rows = u16::try_from(rows_a_request).unwrap_or(u16::MAX).min(height - row);
        data.clear();
        for line in row..row + rows {
            let start = (usize::from(y) + usize::from(line)).saturating_mul(stride).saturating_add(usize::from(x));
            let line_pixels = pixels.get(start..start.saturating_add(usize::from(width))).ok_or_else(|| Error::new("a box beyond the frame"))?;
            data.extend(line_pixels.iter().flat_map(|pixel| pixel.to_ne_bytes()));
        }
        let dst_y = i16::try_from(u32::from(y) + u32::from(row)).map_err(|_| Error::new("a box"))?;
        let dst_x = i16::try_from(x).map_err(|_| Error::new("a box"))?;
        connection
            .put_image(ImageFormat::Z_PIXMAP, window, gc, width, rows, dst_x, dst_y, 0, depth, &data)
            .map_err(|error| failed("putting the frame", error))?
            .ignore_error();
        row += rows;
    }
    Ok(())
}

/// Whether the server takes a 32-bit pixel at `depth`, and `visual` holds
/// red, green and blue where the interface puts them, in the server's
/// byte order as this machine's.
fn takes_our_pixels(setup: &xproto::Setup, visual: xproto::Visualid, depth: u8) -> bool {
    #[cfg(target_endian = "little")]
    let ours = ImageOrder::LSB_FIRST;
    #[cfg(target_endian = "big")]
    let ours = ImageOrder::MSB_FIRST;
    let format = setup.pixmap_formats.iter().any(|format| format.depth == depth && format.bits_per_pixel == BITS_PER_PIXEL);
    let order = setup.image_byte_order == ours;
    let masks = setup
        .roots
        .iter()
        .flat_map(|screen| &screen.allowed_depths)
        .filter(|allowed| allowed.depth == depth && DEPTHS.contains(&allowed.depth))
        .flat_map(|allowed| &allowed.visuals)
        .any(|candidate| {
            candidate.visual_id == visual
                && (candidate.class == VisualClass::TRUE_COLOR || candidate.class == VisualClass::DIRECT_COLOR)
                && (candidate.red_mask, candidate.green_mask, candidate.blue_mask) == (RED_MASK, GREEN_MASK, BLUE_MASK)
        });
    format && order && masks
}

/// Whether the server shares memory by descriptor (`MIT-SHM` 1.2).
fn shares_memory(connection: &XCBConnection) -> bool {
    if connection.extension_information(shm::X11_EXTENSION_NAME).ok().flatten().is_none() {
        return false;
    }
    connection
        .shm_query_version()
        .ok()
        .and_then(|cookie| cookie.reply().ok())
        .is_some_and(|version| (version.major_version, version.minor_version) >= SHM_FD_VERSION)
}

impl Drop for Surface {
    fn drop(&mut self) {
        let _ = self.wait();
        if let Pixels::Shared { mapping: Some((_, segment)), .. } = &self.pixels
            && let Ok(cookie) = self.connection.shm_detach(*segment)
        {
            cookie.ignore_error();
        }
        if let Ok(cookie) = self.connection.free_gc(self.gc) {
            cookie.ignore_error();
        }
        let _ = self.connection.flush();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::num::NonZeroU32;
    use std::ptr::NonNull;

    use raw_window_handle::{XcbDisplayHandle, XcbWindowHandle};

    const WIDTH: u16 = 64;
    const HEIGHT: u16 = 48;
    const BACKGROUND: u32 = 0x0011_2233;
    const CHANGED: u32 = 0x00AA_BBCC;
    const NOT_PRESENTED: u32 = 0x00FF_FFFF;
    /// The colour bits of a pixel read back at depth 24.
    const COLOUR: u32 = 0x00FF_FFFF;

    /// A mapped window of its own on the display `DISPLAY` names, and the
    /// handles of it and its connection.
    fn window() -> (XCBConnection, RawDisplayHandle, RawWindowHandle) {
        let (connection, screen) = XCBConnection::connect(None).expect("an X server: DISPLAY naming a private Xvfb");
        let root = &connection.setup().roots[screen];
        let window = connection.generate_id().unwrap();
        connection
            .create_window(
                root.root_depth,
                window,
                root.root,
                0,
                0,
                WIDTH,
                HEIGHT,
                0,
                xproto::WindowClass::INPUT_OUTPUT,
                root.root_visual,
                &xproto::CreateWindowAux::new(),
            )
            .unwrap()
            .check()
            .unwrap();
        connection.map_window(window).unwrap().check().unwrap();
        let display = RawDisplayHandle::Xcb(XcbDisplayHandle::new(NonNull::new(connection.get_raw_xcb_connection()), i32::try_from(screen).unwrap()));
        let mut handle = XcbWindowHandle::new(NonZeroU32::new(window).unwrap());
        handle.visual_id = NonZeroU32::new(root.root_visual);
        (connection, display, RawWindowHandle::Xcb(handle))
    }

    fn read_back(connection: &XCBConnection, window: RawWindowHandle) -> Vec<u32> {
        let RawWindowHandle::Xcb(handle) = window else { unreachable!() };
        let image = connection.get_image(ImageFormat::Z_PIXMAP, handle.window.get(), 0, 0, WIDTH, HEIGHT, u32::MAX).unwrap().reply().unwrap();
        image.data.as_chunks::<4>().0.iter().map(|&bytes| u32::from_ne_bytes(bytes) & COLOUR).collect()
    }

    fn rect(x: u32, y: u32, width: u32, height: u32) -> Rect {
        Rect { x, y, width: NonZeroU32::new(width).unwrap(), height: NonZeroU32::new(height).unwrap() }
    }

    /// A frame reaches the window whole, then only its changed box does: a
    /// pixel changed outside the box is not presented, which shows the
    /// present is limited to the damage. Both with the server's shared
    /// memory and with the pixels sent in requests.
    #[test]
    #[ignore = "needs an X server: run with DISPLAY naming a private Xvfb, never the desktop's"]
    fn only_the_changed_box_reaches_the_window() {
        for shared in [true, false] {
            let (connection, display, window) = window();
            let mut surface = Surface::new(display, window).unwrap();
            if shared {
                assert!(matches!(surface.pixels, Pixels::Shared { .. }), "Xvfb shares memory by descriptor");
            } else {
                surface.pixels = Pixels::Wire(Vec::new());
            }
            surface.resize(u32::from(WIDTH), u32::from(HEIGHT)).unwrap();
            assert_eq!(surface.prepare().unwrap(), 0, "a new buffer holds no frame");
            surface.pixels_mut().fill(BACKGROUND);
            surface.present(&[rect(0, 0, u32::from(WIDTH), u32::from(HEIGHT))]).unwrap();
            assert_eq!(surface.prepare().unwrap(), 1, "the buffer holds the frame presented");
            assert!(read_back(&connection, window).iter().all(|&pixel| pixel == BACKGROUND), "shared {shared}: the whole frame");
            let stride = usize::from(WIDTH);
            for y in 8..16 {
                surface.pixels_mut()[y * stride + 8..y * stride + 24].fill(CHANGED);
            }
            surface.pixels_mut()[40 * stride + 40] = NOT_PRESENTED;
            surface.present(&[rect(8, 8, 16, 8)]).unwrap();
            surface.prepare().unwrap();
            let shown = read_back(&connection, window);
            for y in 0..usize::from(HEIGHT) {
                for x in 0..stride {
                    let expected = if (8..24).contains(&x) && (8..16).contains(&y) { CHANGED } else { BACKGROUND };
                    assert_eq!(shown[y * stride + x], expected, "shared {shared}: the pixel at {x}, {y}");
                }
            }
            surface.resize(u32::from(WIDTH) / 2, u32::from(HEIGHT)).unwrap();
            assert_eq!(surface.prepare().unwrap(), 0, "resized, the buffer holds no frame");
        }
    }
}
