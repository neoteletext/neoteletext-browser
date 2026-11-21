//! Windows: GDI's `SetDIBitsToDevice` (Windows SDK, `wingdi.h`), one call
//! a changed box, from the interface's own buffer into the window's device
//! context, which the compositor shows at its next refresh. Each box is
//! given as a top-down device-independent bitmap of its own rows, starting
//! at the box's first row, so its source is the whole bitmap's height and
//! no question of which way up a bitmap's rows count arises. The window is
//! then marked painted (`ValidateRect`), so the system does not ask for it
//! again.
#![allow(unsafe_code, reason = "GDI is C functions taking raw pointers: each call has its reason")]

use raw_window_handle::Win32WindowHandle;
use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::Graphics::Gdi::{BI_RGB, BITMAPINFO, BITMAPINFOHEADER, DIB_RGB_COLORS, GetDC, ReleaseDC, SetDIBitsToDevice, ValidateRect};

use crate::{Error, Rect, pixel_count};

/// Bits in a pixel: `0x00RRGGBB` little-endian is GDI's 32-bit `BI_RGB`
/// layout, blue, green, red and an unused byte.
const BITS_PER_PIXEL: u16 = 32;

/// Presenting to one window.
pub struct Surface {
    window: HWND,
    pixels: Vec<u32>,
    width: u32,
    height: u32,
    presented: bool,
}

impl Surface {
    #[expect(clippy::unnecessary_wraps, reason = "one signature on every platform: on the others a display can refuse")]
    pub fn new(window: Win32WindowHandle) -> Result<Self, Error> {
        Ok(Self { window: window.hwnd.get(), pixels: Vec::new(), width: 0, height: 0, presented: false })
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        self.pixels = vec![0; pixel_count(width, height)];
        self.width = width;
        self.height = height;
        self.presented = false;
    }

    pub fn prepare(&self) -> u8 {
        u8::from(self.presented)
    }

    pub fn pixels(&self) -> &[u32] {
        &self.pixels
    }

    pub fn pixels_mut(&mut self) -> &mut [u32] {
        &mut self.pixels
    }

    pub fn present(&mut self, damage: &[Rect]) -> Result<(), Error> {
        let width = i32::try_from(self.width).map_err(|_| Error::new("the window"))?;
        let stride = usize::try_from(self.width).map_err(|_| Error::new("the window"))?;
        // SAFETY: the window is winit's, which outlives this surface; its
        // device context is released below.
        let context = unsafe { GetDC(self.window) };
        if context == 0 {
            return Err(Error::new("the window's device context"));
        }
        let mut result = Ok(());
        for rect in damage {
            let (Ok(x), Ok(y), Ok(rows)) = (i32::try_from(rect.x), i32::try_from(rect.y), i32::try_from(rect.height.get())) else {
                result = Err(Error::new("a box beyond GDI's coordinates"));
                break;
            };
            let start = usize::try_from(rect.y).unwrap_or(usize::MAX).saturating_mul(stride);
            let needed = usize::try_from(rect.height.get()).unwrap_or(usize::MAX).saturating_mul(stride);
            let Some(bits) = self.pixels.get(start..start.saturating_add(needed)) else {
                result = Err(Error::new("a box beyond the frame"));
                break;
            };
            let info = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: u32::try_from(size_of::<BITMAPINFOHEADER>()).unwrap_or(0),
                    biWidth: width,
                    // Negative: top-down, the first row first.
                    biHeight: -rows,
                    biPlanes: 1,
                    biBitCount: BITS_PER_PIXEL,
                    biCompression: BI_RGB,
                    biSizeImage: 0,
                    biXPelsPerMeter: 0,
                    biYPelsPerMeter: 0,
                    biClrUsed: 0,
                    biClrImportant: 0,
                },
                bmiColors: [windows_sys::Win32::Graphics::Gdi::RGBQUAD { rgbBlue: 0, rgbGreen: 0, rgbRed: 0, rgbReserved: 0 }],
            };
            // SAFETY: `bits` holds `rows` whole rows of `width` 32-bit
            // pixels, the bitmap `info` describes, and lives until the call
            // returns; the box lies within the window's frame
            // (`crate::within`).
            let copied = unsafe {
                SetDIBitsToDevice(
                    context,
                    x,
                    y,
                    rect.width.get(),
                    rect.height.get(),
                    x,
                    0,
                    0,
                    rect.height.get(),
                    bits.as_ptr().cast(),
                    &raw const info,
                    DIB_RGB_COLORS,
                )
            };
            if copied == 0 {
                result = Err(Error::new("GDI drew none of the frame"));
                break;
            }
        }
        // SAFETY: the window is winit's; a null rectangle is the whole
        // client area. The device context was got above for this window.
        unsafe {
            ValidateRect(self.window, std::ptr::null());
            ReleaseDC(self.window, context);
        }
        if result.is_ok() {
            self.presented = true;
        }
        result
    }
}
