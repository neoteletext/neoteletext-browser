//! Page images as the painter holds them: set from a page's palette and
//! dots, kept while the page is shown, and drawn.

use interface_core::canvas::{PAGE_HEIGHT, PAGE_WIDTH, PageKey, REGIS_COLOURS, REGIS_HEIGHT, REGIS_WIDTH};
use interface_core::geometry::Rect;
use interface_core::theme::Colour;
use std::sync::Arc;
use vello_cpu::kurbo::{self, Vec2};
use vello_cpu::peniko::{ImageAlphaType, ImageQuality, ImageSampler};
use vello_cpu::{Image, ImageSource, PixelMetadata, Pixmap};

use crate::damage::changed_dots;
use crate::{DeviceBox, OPAQUE, PAGE_DOTS, PageError, Painter, RGBA_BYTES};

/// A page's image as the painter holds it.
pub(crate) struct PageImage {
    image: Image,
    pixmap: Arc<Pixmap>,
    /// The colour of the screen around it.
    screen: Colour,
    /// Which image it is: a new one is a new number.
    number: u64,
}

impl PageImage {
    /// The colour of the screen around it.
    pub(crate) fn screen(&self) -> Colour {
        self.screen
    }

    /// Which image it is.
    pub(crate) fn number(&self) -> u64 {
        self.number
    }
}

/// The picture of a page drawn in ReGIS (DEC-6335): 800 by 480 pixels.
pub(crate) struct RegisImage {
    image: Image,
    /// Colour 0 of its sixteen: what is drawn above and below it.
    background: Colour,
    number: u64,
}

impl RegisImage {
    /// Which image it is.
    pub(crate) fn number(&self) -> u64 {
        self.number
    }
}

/// A ReGIS picture's size, as `vello_cpu` takes it.
const REGIS_SIZE: (u16, u16) = (REGIS_WIDTH, REGIS_HEIGHT);
/// Its pixels.
const REGIS_PIXELS: usize = REGIS_WIDTH as usize * REGIS_HEIGHT as usize;

/// Teletext is blocks: nearest-neighbour sampling keeps them sharp.
pub(crate) fn image(pixmap: Arc<Pixmap>) -> Image {
    Image { image: ImageSource::Pixmap(pixmap), sampler: ImageSampler { quality: ImageQuality::Low, ..ImageSampler::default() } }
}

fn pixmap(rgba: Vec<u8>) -> Arc<Pixmap> {
    // Every teletext pixel is opaque, which lets the renderer skip blending.
    let metadata = PixelMetadata { may_have_transparency: false, alpha_type: ImageAlphaType::Alpha };
    Arc::new(Pixmap::from_parts(rgba, PAGE_WIDTH, PAGE_HEIGHT, metadata))
}

impl Painter {
    /// Replaces a page's image whole (requirement 9.6), from palette indices.
    ///
    /// # Errors
    /// When the indices are not a full page or one is outside the palette.
    pub fn set_page(&mut self, key: PageKey, palette: &[[u8; 3]], screen: u8, indices: &[u8]) -> Result<(), PageError> {
        if indices.len() != PAGE_DOTS {
            return Err(PageError::Size);
        }
        let [red, green, blue] = *palette.get(usize::from(screen)).ok_or(PageError::Index)?;
        let screen = Colour { red, green, blue };
        let mut rgba = Vec::with_capacity(PAGE_DOTS * RGBA_BYTES);
        for &index in indices {
            let [red, green, blue] = *palette.get(usize::from(index)).ok_or(PageError::Index)?;
            rgba.extend_from_slice(&[red, green, blue, OPAQUE]);
        }
        let held = self.pages.get(&key);
        if held.is_some_and(|held| held.screen != screen) {
            // The screen colour is drawn around the page, by other calls,
            // which differ in their own right; the image is whole again.
            self.changed.insert(key, DeviceBox::whole(PAGE_WIDTH, PAGE_HEIGHT));
        } else {
            let dots = held.map_or(DeviceBox::whole(PAGE_WIDTH, PAGE_HEIGHT), |held| changed_dots(&held.pixmap, &rgba));
            if dots.is_empty() {
                return Ok(());
            }
            let since = self.changed.get(&key).copied().unwrap_or_default();
            self.changed.insert(key, since.union(dots));
        }
        let pixmap = pixmap(rgba);
        let number = self.next_image;
        self.next_image = number.wrapping_add(1);
        self.pages.insert(key, PageImage { image: image(Arc::clone(&pixmap)), pixmap, screen, number });
        Ok(())
    }

    /// Replaces the picture of a page drawn in ReGIS (DEC-6335), which is
    /// drawn in the page's place: sixteen colours, and 800 by 480 pixels
    /// packed two to a byte.
    ///
    /// # Errors
    /// When the pixels are not a whole picture.
    pub fn set_regis(&mut self, key: PageKey, palette: &[[u8; 3]; REGIS_COLOURS], packed: &[u8]) -> Result<(), PageError> {
        if packed.len() != REGIS_PIXELS / 2 {
            return Err(PageError::RegisSize);
        }
        let mut rgba = Vec::with_capacity(REGIS_PIXELS * RGBA_BYTES);
        for byte in packed {
            for index in [byte >> 4, byte & 0x0F] {
                let [red, green, blue] = palette.get(usize::from(index)).copied().unwrap_or_default();
                rgba.extend_from_slice(&[red, green, blue, OPAQUE]);
            }
        }
        let metadata = PixelMetadata { may_have_transparency: false, alpha_type: ImageAlphaType::Alpha };
        let pixmap = Arc::new(Pixmap::from_parts(rgba, REGIS_SIZE.0, REGIS_SIZE.1, metadata));
        // The picture is shown smaller than it is on most windows, so it
        // is sampled smoothly: nearest-neighbour would drop thin lines.
        let image = Image { image: ImageSource::Pixmap(pixmap), sampler: ImageSampler { quality: ImageQuality::Medium, ..ImageSampler::default() } };
        let [[red, green, blue], ..] = *palette;
        let number = self.next_image;
        self.next_image = number.wrapping_add(1);
        self.regis.insert(key, RegisImage { image, background: Colour { red, green, blue }, number });
        // Where the page is drawn, all of it is drawn again.
        self.changed.insert(key, DeviceBox::whole(PAGE_WIDTH, PAGE_HEIGHT));
        Ok(())
    }

    /// Forgets the images of pages not in `keep`.
    pub fn retain_pages(&mut self, keep: impl Fn(&PageKey) -> bool) {
        self.pages.retain(|key, _| keep(key));
        self.regis.retain(|key, _| keep(key));
    }

    pub(crate) fn draw_page(&mut self, key: PageKey, area: Rect) {
        if let Some(regis) = self.regis.get(&key) {
            // The picture across the image's width, its own proportions
            // kept, on its own background.
            let (image, background) = (regis.image.clone(), regis.background);
            let place = interface_core::canvas::regis_within(area);
            let (width, height) = (f64::from(REGIS_SIZE.0), f64::from(REGIS_SIZE.1));
            self.context.set_transform(self.base());
            self.context.set_paint(crate::colour(background));
            self.context.fill_rect(&crate::rect(area));
            let placement = self.base().pre_translate(Vec2::new(place.x0, place.y0)).pre_scale_non_uniform(place.width() / width, place.height() / height);
            self.context.set_transform(placement);
            self.context.set_paint(image);
            self.context.fill_rect(&kurbo::Rect::new(0.0, 0.0, width, height));
            return;
        }
        let Some(page) = self.pages.get(&key) else { return };
        let (width, height) = (f64::from(PAGE_WIDTH), f64::from(PAGE_HEIGHT));
        let image = page.image.clone();
        let placement = self.base().pre_translate(Vec2::new(area.x0, area.y0)).pre_scale_non_uniform(area.width() / width, area.height() / height);
        self.context.set_transform(placement);
        self.context.set_paint(image);
        self.context.fill_rect(&kurbo::Rect::new(0.0, 0.0, width, height));
    }
}
