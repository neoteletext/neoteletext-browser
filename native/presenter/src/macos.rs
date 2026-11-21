//! macOS: a Core Animation layer (`CALayer`, Apple's Core Animation
//! Programming Guide) added over the window's view, whose contents are
//! set, in a transaction with its animations off (`CATransaction`), to an
//! image of each frame (`CGImageCreate` over a `CGDataProvider` that owns
//! a copy of the pixels). Core Animation shows it at the next refresh, a
//! newer frame replacing one not yet shown. A layer's contents are a whole
//! image, so the changed boxes do not limit what is copied here; the
//! interface's own buffer keeps the last frame (its age is 1), so only the
//! changed boxes are drawn into it, where softbuffer gave a fresh buffer
//! each frame and the whole window was drawn into it again (DEC-1255).
#![allow(unsafe_code, reason = "the window's view is given as a raw pointer, and Core Graphics' image takes raw memory: each block has its reason")]

use std::ffi::c_void;
use std::ptr::NonNull;

use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{MainThreadMarker, msg_send};
use objc2_core_foundation::{CFRetained, CGPoint};
use objc2_core_graphics::{
    CGBitmapInfo, CGColorRenderingIntent, CGColorSpace, CGDataProvider, CGImage, CGImageAlphaInfo, CGImageByteOrderInfo, CGImageComponentInfo,
    CGImagePixelFormatInfo,
};
use objc2_quartz_core::{CALayer, CATransaction, kCAGravityTopLeft};
use raw_window_handle::AppKitWindowHandle;

use crate::{BYTES_PER_PIXEL, Error, pixel_count};

/// Bits in a colour component and in a pixel: `0x00RRGGBB` read as a
/// 32-bit little-endian word with its first byte unused
/// (`kCGImageAlphaNoneSkipFirst`, `kCGImageByteOrder32Little`).
const BITS_PER_COMPONENT: usize = 8;
const BITS_PER_PIXEL: usize = 32;

/// Presenting to one window's view.
pub struct Surface {
    layer: Retained<CALayer>,
    root: Retained<CALayer>,
    colour_space: CFRetained<CGColorSpace>,
    pixels: Vec<u32>,
    width: u32,
    height: u32,
    presented: bool,
}

impl Surface {
    pub fn new(window: AppKitWindowHandle) -> Result<Self, Error> {
        // Core Animation's layers belong to the main thread, as the
        // window does.
        let _main = MainThreadMarker::new().ok_or_else(|| Error::new("presenting from a thread other than the main"))?;
        // SAFETY: the view is winit's live `NSView` of this window, which
        // outlives this surface; it is read on the main thread.
        let view: &AnyObject = unsafe { window.ns_view.cast::<AnyObject>().as_ref() };
        // SAFETY: `setWantsLayer:` and `layer` are `NSView` methods taking
        // and returning what is written (AppKit's `NSView`); a view that
        // wants a layer has one.
        let root: Option<Retained<CALayer>> = unsafe {
            let () = msg_send![view, setWantsLayer: true];
            msg_send![view, layer]
        };
        let root = root.ok_or_else(|| Error::new("the window's view has no layer"))?;
        let layer = CALayer::new();
        root.addSublayer(&layer);
        layer.setAnchorPoint(CGPoint::new(0.0, 0.0));
        // The frame's first row at the top, as AppKit's views are drawn;
        // a frame smaller than the layer, while the window is resized, is
        // shown at its top left, never stretched.
        layer.setGeometryFlipped(true);
        // SAFETY: a constant string Core Animation defines.
        layer.setContentsGravity(unsafe { kCAGravityTopLeft });
        let colour_space = CGColorSpace::new_device_rgb().ok_or_else(|| Error::new("the device's colour space"))?;
        Ok(Self { layer, root, colour_space, pixels: Vec::new(), width: 0, height: 0, presented: false })
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        self.pixels = vec![0; pixel_count(width, height)];
        self.width = width;
        self.height = height;
        self.presented = false;
    }

    pub const fn prepare(&self) -> u8 {
        if self.presented { 1 } else { 0 }
    }

    pub fn pixels(&self) -> &[u32] {
        &self.pixels
    }

    pub fn pixels_mut(&mut self) -> &mut [u32] {
        &mut self.pixels
    }

    /// Presents the whole frame: a layer's contents are one image, which
    /// `_damage` cannot limit.
    pub fn present(&mut self, _damage: &[crate::Rect]) -> Result<(), Error> {
        /// Frees the copy of the pixels the image held, when Core Graphics
        /// is done with it.
        unsafe extern "C-unwind" fn release(_: *mut c_void, data: NonNull<c_void>, size: usize) {
            let pixels = std::ptr::slice_from_raw_parts_mut(data.cast::<u32>().as_ptr(), size / BYTES_PER_PIXEL);
            // SAFETY: `data` and `size` are the boxed slice handed to the
            // provider below, given back once.
            drop(unsafe { Box::from_raw(pixels) });
        }
        let (width, height) =
            (usize::try_from(self.width).map_err(|_| Error::new("the window"))?, usize::try_from(self.height).map_err(|_| Error::new("the window"))?);
        let copy: Box<[u32]> = self.pixels.clone().into_boxed_slice();
        let bytes = copy.len().saturating_mul(BYTES_PER_PIXEL);
        let data = Box::into_raw(copy);
        // SAFETY: the provider owns the boxed copy from here and frees it
        // through `release` once, with the length given.
        let provider = unsafe { CGDataProvider::with_data(std::ptr::null_mut(), data.cast::<c_void>().cast_const(), bytes, Some(release)) };
        let Some(provider) = provider else {
            // SAFETY: the provider was not made, so the copy is still ours.
            drop(unsafe { Box::from_raw(data) });
            return Err(Error::new("an image of the frame"));
        };
        let info = CGBitmapInfo(
            CGImageAlphaInfo::NoneSkipFirst.0 | CGImageComponentInfo::Integer.0 | CGImageByteOrderInfo::Order32Little.0 | CGImagePixelFormatInfo::Packed.0,
        );
        // SAFETY: the provider holds `width` by `height` pixels of
        // `BITS_PER_PIXEL` bits, rows of `width * 4` bytes; no decode array.
        let image = unsafe {
            CGImage::new(
                width,
                height,
                BITS_PER_COMPONENT,
                BITS_PER_PIXEL,
                width.saturating_mul(BYTES_PER_PIXEL),
                Some(&self.colour_space),
                info,
                Some(&provider),
                std::ptr::null(),
                false,
                CGColorRenderingIntent::RenderingIntentDefault,
            )
        }
        .ok_or_else(|| Error::new("an image of the frame"))?;
        CATransaction::begin();
        CATransaction::setDisableActions(true);
        // The layer covers the view, at the view's backing scale, so one
        // pixel of the frame is one of the display's.
        self.layer.setFrame(self.root.bounds());
        self.layer.setContentsScale(self.root.contentsScale());
        // SAFETY: an image is what a layer's contents take.
        unsafe { self.layer.setContents(Some(image.as_ref())) };
        CATransaction::commit();
        self.presented = true;
        Ok(())
    }
}

impl Drop for Surface {
    fn drop(&mut self) {
        self.layer.removeFromSuperlayer();
    }
}
