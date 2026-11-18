//! Page images as the painter holds them: set from a page's palette and
//! dots, kept while the page is shown, and drawn.

use interface_core::canvas::{MAX_STILLS, PAGE_HEIGHT, PAGE_WIDTH, PageKey, REGIS_COLOURS, REGIS_HEIGHT, REGIS_WIDTH, STILL_HEIGHT, STILL_WIDTH, image_within};
use interface_core::geometry::Rect;
use interface_core::theme::Colour;
use std::sync::Arc;
use vello_cpu::kurbo::{self, Vec2};
use vello_cpu::peniko::{ImageAlphaType, ImageQuality, ImageSampler};
use vello_cpu::{Image, ImageSource, PixelMetadata, Pixmap, RenderContext};

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

/// A still of a page, made once at [`STILL_WIDTH`] by [`STILL_HEIGHT`]
/// pixels from what the page area draws (REQ-6640 item 3).
pub(crate) struct Still {
    image: Image,
    /// Which still it is: a new one is a new number.
    number: u64,
}

impl Still {
    /// Which still it is.
    pub(crate) fn number(&self) -> u64 {
        self.number
    }
}

/// The picture of a page drawn in ReGIS (DEC-6335): 800 by 480 pixels.
pub(crate) struct RegisImage {
    /// Its pixels, kept to find what a new picture changes (REQ-6399).
    pixmap: Arc<Pixmap>,
    /// Colour 0 of its sixteen: what is drawn above and below it.
    background: Colour,
    number: u64,
    /// Which picture of its page the decoder sent it as: a part is laid
    /// over the picture it names as its base and no other.
    sequence: u32,
    /// Its sixteen colours and each pixel's index into them: a part that
    /// changes only the colours, as a NAPLPS BLINK does, carries no pixels
    /// and the picture is coloured again here (animation.md ANIM-5).
    palette: [[u8; 3]; REGIS_COLOURS],
    indices: Vec<u8>,
}

/// Which pixels of a ReGIS picture a message carries (REQ-6399): a whole
/// picture, or the part of one that changed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RegisPart {
    /// The picture's number among those sent for its page.
    pub number: u32,
    /// The number of the picture this is a part of; 0 for a whole picture.
    pub base: u32,
    /// The left column and top row of the pixels, in the picture.
    pub x: u16,
    /// The top row.
    pub y: u16,
    /// Columns: even, as pixels are packed two to a byte.
    pub width: u16,
    /// Rows.
    pub height: u16,
}

impl RegisPart {
    /// A whole picture, numbered `number`.
    #[must_use]
    pub const fn whole(number: u32) -> Self {
        Self { number, base: 0, x: 0, y: 0, width: REGIS_SIZE.0, height: REGIS_SIZE.1 }
    }
}

/// A picture's pixels in colour: each index's colour of `palette`, opaque.
fn coloured(indices: &[u8], palette: &[[u8; 3]; REGIS_COLOURS]) -> Vec<u8> {
    let mut rgba = Vec::with_capacity(indices.len().saturating_mul(RGBA_BYTES));
    for &index in indices {
        let [red, green, blue] = palette.get(usize::from(index)).copied().unwrap_or_default();
        rgba.extend_from_slice(&[red, green, blue, OPAQUE]);
    }
    rgba
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

/// How near a whole number counts as one: a hundredth of a display pixel
/// across the whole picture, well inside what rounding a layout leaves.
const WHOLE_TOLERANCE: f64 = 0.01;

/// Whether `shown`, a length in display pixels, is a whole multiple, one
/// or more, of `actual`.
fn whole_multiple(shown: f64, actual: f64) -> bool {
    let multiple = (shown / actual).round();
    multiple >= 1.0 && (shown - multiple * actual).abs() < WHOLE_TOLERANCE
}

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

    /// Takes the picture of a page drawn in ReGIS (DEC-6335), which is
    /// drawn in the page's place, or the part of it that changed
    /// (REQ-6399): sixteen colours, the pixels of `part` packed two to a
    /// byte. A part is laid over the picture it names as its base; one
    /// that names another picture than the one held is dropped, as a
    /// page's changes are when its image is no longer held.
    ///
    /// # Errors
    /// When the pixels are not the part's, or the part does not lie in
    /// the picture.
    pub fn set_regis(&mut self, key: PageKey, palette: &[[u8; 3]; REGIS_COLOURS], part: RegisPart, packed: &[u8]) -> Result<(), PageError> {
        let (left, top, across, down) = (usize::from(part.x), usize::from(part.y), usize::from(part.width), usize::from(part.height));
        let fits = left.saturating_add(across) <= usize::from(REGIS_SIZE.0) && top.saturating_add(down) <= usize::from(REGIS_SIZE.1);
        let whole = part.base == 0;
        if !fits || across % 2 != 0 || packed.len() != across.saturating_mul(down) / 2 || (whole && part != RegisPart::whole(part.number)) {
            return Err(PageError::RegisSize);
        }
        let colour = |index: u8| palette.get(usize::from(index)).copied().unwrap_or_default();
        let [[red, green, blue], ..] = *palette;
        let background = Colour { red, green, blue };
        if !whole {
            let Some(held) = self.regis.get_mut(&key).filter(|held| held.sequence == part.base) else { return Ok(()) };
            held.sequence = part.number;
            // The part's pixels into the picture's indices.
            for (row, line) in packed.chunks_exact((across / 2).max(1)).enumerate() {
                let start = top.saturating_add(row).saturating_mul(usize::from(REGIS_SIZE.0)).saturating_add(left);
                let Some(place) = held.indices.get_mut(start..start.saturating_add(across)) else { return Err(PageError::RegisSize) };
                for (pixel, index) in place.iter_mut().zip(line.iter().flat_map(|byte| [byte >> 4, byte & 0x0F])) {
                    *pixel = index;
                }
            }
            let mut changed = DeviceBox { x0: part.x, y0: part.y, x1: part.x.saturating_add(part.width), y1: part.y.saturating_add(part.height) };
            if held.palette == *palette {
                if packed.is_empty() {
                    return Ok(());
                }
                let data = Arc::make_mut(&mut held.pixmap).data_as_u8_slice_mut();
                let row_bytes = across.saturating_mul(RGBA_BYTES);
                for row in 0..down {
                    let start = top.saturating_add(row).saturating_mul(usize::from(REGIS_SIZE.0)).saturating_add(left);
                    let (Some(place), Some(line)) = (
                        data.get_mut(start.saturating_mul(RGBA_BYTES)..start.saturating_mul(RGBA_BYTES).saturating_add(row_bytes)),
                        held.indices.get(start..start.saturating_add(across)),
                    ) else {
                        return Err(PageError::RegisSize);
                    };
                    for (pixel, &index) in place.as_chunks_mut::<RGBA_BYTES>().0.iter_mut().zip(line) {
                        let [red, green, blue] = colour(index);
                        *pixel = [red, green, blue, OPAQUE];
                    }
                }
            } else {
                // New colours: every pixel is coloured again from its index,
                // and what changed colour is drawn again.
                let rgba = coloured(&held.indices, palette);
                changed = changed.union(crate::damage::changed_pixels(&held.pixmap, &rgba, REGIS_SIZE.0));
                let metadata = PixelMetadata { may_have_transparency: false, alpha_type: ImageAlphaType::Alpha };
                held.pixmap = Arc::new(Pixmap::from_parts(rgba, REGIS_SIZE.0, REGIS_SIZE.1, metadata));
                held.palette = *palette;
                if held.background != background {
                    // Colour 0 is drawn around the picture too.
                    held.background = background;
                    self.changed.insert(key, DeviceBox::whole(PAGE_WIDTH, PAGE_HEIGHT));
                }
            }
            held.number = self.next_image;
            self.next_image = self.next_image.wrapping_add(1);
            if !changed.is_empty() {
                self.regis_changed.entry(key).or_default().add(changed);
            }
            return Ok(());
        }
        let mut indices = Vec::with_capacity(REGIS_PIXELS);
        indices.extend(packed.iter().flat_map(|byte| [byte >> 4, byte & 0x0F]));
        let rgba = coloured(&indices, palette);
        // Only where a whole picture differs from the one held is drawn
        // again; a picture on another background, or the first, is drawn
        // whole, with what lies around it.
        match self.regis.get_mut(&key).filter(|held| held.background == background) {
            Some(held) => {
                let pixels = crate::damage::changed_pixels(&held.pixmap, &rgba, REGIS_SIZE.0);
                if pixels.is_empty() {
                    // The same picture again: only its number is new.
                    held.sequence = part.number;
                    held.palette = *palette;
                    held.indices = indices;
                    return Ok(());
                }
                self.regis_changed.entry(key).or_default().add(pixels);
            }
            None => {
                self.changed.insert(key, DeviceBox::whole(PAGE_WIDTH, PAGE_HEIGHT));
            }
        }
        let metadata = PixelMetadata { may_have_transparency: false, alpha_type: ImageAlphaType::Alpha };
        let pixmap = Arc::new(Pixmap::from_parts(rgba, REGIS_SIZE.0, REGIS_SIZE.1, metadata));
        let number = self.next_image;
        self.next_image = number.wrapping_add(1);
        self.regis.insert(key, RegisImage { pixmap, background, number, sequence: part.number, palette: *palette, indices });
        Ok(())
    }

    /// Makes a still of `key` under `still` from its latest image, drawn as
    /// a frame draws the page with its screen (REQ-6640 item 3); false when
    /// no image of the page is held. Past [`MAX_STILLS`] the oldest still
    /// goes.
    pub(crate) fn make_still(&mut self, still: u32, key: PageKey) -> bool {
        let Some(screen) = self.pages.get(&key).map(PageImage::screen) else { return false };
        // The page is drawn by the frame's own code into a context of the
        // still's size, with no offset and one pixel to each point, and
        // the frame's context is put back after.
        let frame = (core::mem::replace(&mut self.context, RenderContext::new(STILL_WIDTH, STILL_HEIGHT)), self.origin, self.scale);
        self.origin = (0.0, 0.0);
        self.scale = 1.0;
        let whole = Rect::new(0.0, 0.0, f64::from(STILL_WIDTH), f64::from(STILL_HEIGHT));
        self.draw_fill(whole, 0.0, screen);
        self.draw_page(key, image_within(whole));
        self.context.flush();
        let mut pixmap = Pixmap::new(STILL_WIDTH, STILL_HEIGHT);
        self.context.render(&mut pixmap, &mut self.resources);
        (self.context, self.origin, self.scale) = frame;
        if self.stills.len() >= MAX_STILLS
            && !self.stills.contains_key(&still)
            && let Some(oldest) = self.stills.iter().min_by_key(|(_, kept)| kept.number).map(|(&number, _)| number)
        {
            self.stills.remove(&oldest);
        }
        let number = self.next_image;
        self.next_image = number.wrapping_add(1);
        // Drawn larger or smaller than made, so sampled smoothly.
        let image = Image { image: ImageSource::Pixmap(Arc::new(pixmap)), sampler: ImageSampler { quality: ImageQuality::Medium, ..ImageSampler::default() } };
        self.stills.insert(still, Still { image, number });
        true
    }

    /// Draws a still, scaled to fill `area`.
    pub(crate) fn draw_still(&mut self, still: u32, area: Rect) {
        let Some(image) = self.stills.get(&still).map(|kept| kept.image.clone()) else { return };
        let (width, height) = (f64::from(STILL_WIDTH), f64::from(STILL_HEIGHT));
        let placement = self.base().pre_translate(Vec2::new(area.x0, area.y0)).pre_scale_non_uniform(area.width() / width, area.height() / height);
        self.context.set_transform(placement);
        self.context.set_paint(image);
        self.context.fill_rect(&kurbo::Rect::new(0.0, 0.0, width, height));
    }

    /// Forgets the images of pages not in `keep`.
    pub fn retain_pages(&mut self, keep: impl Fn(&PageKey) -> bool) {
        self.pages.retain(|key, _| keep(key));
        self.regis.retain(|key, _| keep(key));
        self.regis_changed.retain(|key, _| keep(key));
    }

    /// How a ReGIS picture is fitted from the next frame: contained in the
    /// rectangle its page is drawn in, or in the teletext page's band of
    /// it (REQ-6363).
    pub const fn set_regis_contain(&mut self, contain: bool) {
        self.regis_contain = contain;
    }

    /// The pages drawn in ReGIS that show row 24 of their teletext page,
    /// the coloured-key prompts, beneath their picture from the next frame
    /// (REQ-6379).
    pub fn set_regis_keyed(&mut self, keyed: Vec<PageKey>) {
        self.regis_keyed = keyed;
    }

    /// Row 24 of a page's teletext image, drawn in `row`: the last of the
    /// image's 25 rows, and nothing else of it.
    fn draw_prompts(&mut self, key: PageKey, row: Rect) {
        let Some(page) = self.pages.get(&key) else { return };
        let (width, height) = (f64::from(PAGE_WIDTH), f64::from(PAGE_HEIGHT));
        let lines = height / f64::from(interface_core::canvas::ROWS);
        let (image, top) = (page.image.clone(), height - lines);
        // The whole image is placed so that its last row lies on `row`,
        // and only that row of it is filled.
        let placement = self
            .base()
            .pre_translate(Vec2::new(row.x0, row.y0))
            .pre_scale_non_uniform(row.width() / width, row.height() / lines)
            .pre_translate(Vec2::new(0.0, -top));
        self.context.set_transform(placement);
        self.context.set_paint(image);
        self.context.fill_rect(&kurbo::Rect::new(0.0, top, width, height));
    }

    pub(crate) fn draw_page(&mut self, key: PageKey, area: Rect) {
        if let Some(regis) = self.regis.get(&key) {
            // The picture across the image's width, its own proportions
            // kept, on its own background.
            let (pixmap, background) = (Arc::clone(&regis.pixmap), regis.background);
            let layout = interface_core::canvas::regis_layout(area, self.regis_contain, self.regis_keyed.contains(&key));
            let place = layout.picture;
            let (width, height) = (f64::from(REGIS_SIZE.0), f64::from(REGIS_SIZE.1));
            // At a whole multiple of its size each pixel is a block of
            // display pixels and is drawn as one (REQ-6401); at any other
            // size it is sampled smoothly, or thin lines would be dropped
            // from a picture shown smaller.
            let quality = if whole_multiple(place.width() * self.scale, width) && whole_multiple(place.height() * self.scale, height) {
                ImageQuality::Low
            } else {
                ImageQuality::Medium
            };
            let image = Image { image: ImageSource::Pixmap(pixmap), sampler: ImageSampler { quality, ..ImageSampler::default() } };
            self.context.set_transform(self.base());
            self.context.set_paint(crate::colour(background));
            self.context.fill_rect(&crate::rect(area));
            let placement = self.base().pre_translate(Vec2::new(place.x0, place.y0)).pre_scale_non_uniform(place.width() / width, place.height() / height);
            self.context.set_transform(placement);
            self.context.set_paint(image);
            self.context.fill_rect(&kurbo::Rect::new(0.0, 0.0, width, height));
            if let Some(row) = layout.keys {
                self.draw_prompts(key, row);
            }
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
