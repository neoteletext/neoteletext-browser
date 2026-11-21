//! Page images as the painter holds them: set from a page's palette and
//! dots, kept while the page is shown, and drawn.

use interface_core::canvas::{MAX_STILLS, PAGE_HEIGHT, PAGE_WIDTH, PageKey, PictureLanguage, REGIS_COLOURS, STILL_HEIGHT, STILL_WIDTH, image_within};
use interface_core::geometry::Rect;
use interface_core::theme::Colour;
use std::sync::Arc;

use crate::damage::changed_dots;
use crate::raster::paint::{Pixels, Sampling};
use crate::raster::surface::{Surface, word};
use crate::{DeviceBox, OPAQUE, PAGE_DOTS, PageError, Painter};

/// A page's image as the painter holds it.
pub(crate) struct PageImage {
    /// Its dots; teletext is blocks, sampled nearest so they stay sharp.
    pixels: Arc<Pixels>,
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
    /// Drawn larger or smaller than made, so sampled smoothly.
    pixels: Arc<Pixels>,
    /// Which still it is: a new one is a new number.
    number: u64,
}

impl Still {
    /// Which still it is.
    pub(crate) fn number(&self) -> u64 {
        self.number
    }
}

/// The picture of a page drawn in ReGIS (DEC-6335), 800 by 480 pixels, or
/// in NAPLPS, the teletext raster's 480 by 500 (REQ-5819).
pub(crate) struct RegisImage {
    /// What it is drawn in, which says its size and where it lies.
    pub(crate) language: PictureLanguage,
    /// Its pixels, kept to find what a new picture changes (REQ-6399).
    pixels: Arc<Pixels>,
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

/// Which pixels of a ReGIS or NAPLPS picture a message carries (REQ-6399):
/// a whole picture, or the part of one that changed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RegisPart {
    /// What the picture is drawn in, which says its size.
    pub language: PictureLanguage,
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
    /// A whole picture drawn in `language`, numbered `number`.
    #[must_use]
    pub const fn whole(number: u32, language: PictureLanguage) -> Self {
        Self { language, number, base: 0, x: 0, y: 0, width: language.width(), height: language.height() }
    }
}

/// The bits of a ReGIS pixel that index its sixteen colours: a pixel is
/// one of the two halves of a packed byte.
const INDEX_MASK: u8 = 0x0F;
const _: () = assert!(INDEX_MASK as usize + 1 == REGIS_COLOURS, "a pixel's index reaches every colour and no further");

/// Each of the sixteen colours as an opaque 0RGB word, looked up by index.
fn colour_table(palette: &[[u8; 3]; REGIS_COLOURS]) -> [u32; REGIS_COLOURS] {
    palette.map(|[red, green, blue]| word([red, green, blue, OPAQUE]))
}

/// A picture's pixels in colour: each index's colour of `palette`, opaque.
fn coloured(indices: &[u8], palette: &[[u8; 3]; REGIS_COLOURS]) -> Vec<u32> {
    let table = colour_table(palette);
    indices.iter().map(|&index| table.get(usize::from(index & INDEX_MASK)).copied().unwrap_or_default()).collect()
}

impl RegisImage {
    /// Which image it is.
    pub(crate) fn number(&self) -> u64 {
        self.number
    }
}

/// How near a whole number counts as one: a hundredth of a display pixel
/// across the whole picture, well inside what rounding a layout leaves.
const WHOLE_TOLERANCE: f64 = 0.01;

/// Whether `shown`, a length in display pixels, is a whole multiple, one
/// or more, of `actual`.
fn whole_multiple(shown: f64, actual: f64) -> bool {
    let multiple = (shown / actual).round();
    multiple >= 1.0 && (shown - multiple * actual).abs() < WHOLE_TOLERANCE
}

/// `rgba` as a picture of `language`'s size; refused when it is not.
fn picture(rgba: Vec<u32>, language: PictureLanguage) -> Result<Arc<Pixels>, PageError> {
    Pixels::new(language.width(), language.height(), rgba).map(Arc::new).ok_or(PageError::RegisSize)
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
        let mut rgba = Vec::with_capacity(PAGE_DOTS);
        for &index in indices {
            let [red, green, blue] = *palette.get(usize::from(index)).ok_or(PageError::Index)?;
            rgba.push(word([red, green, blue, OPAQUE]));
        }
        let held = self.pages.get(&key);
        if held.is_some_and(|held| held.screen != screen) {
            // The screen colour is drawn around the page, by other calls,
            // which differ in their own right; the image is whole again.
            self.changed.insert(key, DeviceBox::whole(PAGE_WIDTH, PAGE_HEIGHT));
        } else {
            let dots = held.map_or(DeviceBox::whole(PAGE_WIDTH, PAGE_HEIGHT), |held| changed_dots(&held.pixels, &rgba));
            if dots.is_empty() {
                return Ok(());
            }
            let since = self.changed.get(&key).copied().unwrap_or_default();
            self.changed.insert(key, since.union(dots));
        }
        let pixels = Arc::new(Pixels::new(PAGE_WIDTH, PAGE_HEIGHT, rgba).ok_or(PageError::Size)?);
        let number = self.next_image;
        self.next_image = number.wrapping_add(1);
        self.pages.insert(key, PageImage { pixels, screen, number });
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
        let language = part.language;
        // A picture's size.
        let size = (language.width(), language.height());
        let (left, top, across, down) = (usize::from(part.x), usize::from(part.y), usize::from(part.width), usize::from(part.height));
        let fits = left.saturating_add(across) <= usize::from(size.0) && top.saturating_add(down) <= usize::from(size.1);
        let whole = part.base == 0;
        if !fits || across % 2 != 0 || packed.len() != across.saturating_mul(down) / 2 || (whole && part != RegisPart::whole(part.number, language)) {
            return Err(PageError::RegisSize);
        }
        let [[red, green, blue], ..] = *palette;
        let background = Colour { red, green, blue };
        if !whole {
            let Some(held) = self.regis.get_mut(&key).filter(|held| held.sequence == part.base && held.language == language) else { return Ok(()) };
            held.sequence = part.number;
            // The part's pixels into the picture's indices.
            for (row, line) in packed.chunks_exact((across / 2).max(1)).enumerate() {
                let start = top.saturating_add(row).saturating_mul(usize::from(size.0)).saturating_add(left);
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
                let table = colour_table(palette);
                let data = Arc::make_mut(&mut held.pixels).data_mut();
                for row in 0..down {
                    let start = top.saturating_add(row).saturating_mul(usize::from(size.0)).saturating_add(left);
                    let (Some(place), Some(line)) = (data.get_mut(start..start.saturating_add(across)), held.indices.get(start..start.saturating_add(across)))
                    else {
                        return Err(PageError::RegisSize);
                    };
                    for (pixel, &index) in place.iter_mut().zip(line) {
                        *pixel = table.get(usize::from(index & INDEX_MASK)).copied().unwrap_or_default();
                    }
                }
            } else {
                // New colours: every pixel is coloured again from its index,
                // and what changed colour is drawn again.
                let rgba = coloured(&held.indices, palette);
                changed = changed.union(crate::damage::changed_pixels(&held.pixels, &rgba));
                held.pixels = picture(rgba, language)?;
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
        let mut indices = Vec::with_capacity(language.pixels());
        indices.extend(packed.iter().flat_map(|byte| [byte >> 4, byte & 0x0F]));
        let rgba = coloured(&indices, palette);
        // Only where a whole picture differs from the one held is drawn
        // again; a picture on another background or in another language,
        // or the first, is drawn whole, with what lies around it.
        match self.regis.get_mut(&key).filter(|held| held.background == background && held.language == language) {
            Some(held) => {
                let pixels = crate::damage::changed_pixels(&held.pixels, &rgba);
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
        let pixels = picture(rgba, language)?;
        let number = self.next_image;
        self.next_image = number.wrapping_add(1);
        self.regis.insert(key, RegisImage { language, pixels, background, number, sequence: part.number, palette: *palette, indices });
        Ok(())
    }

    /// Makes a still of `key` under `still` from its latest image, drawn as
    /// a frame draws the page with its screen (REQ-6640 item 3); false when
    /// no image of the page is held. Past [`MAX_STILLS`] the oldest still
    /// goes.
    pub(crate) fn make_still(&mut self, still: u32, key: PageKey) -> bool {
        let Some(screen) = self.pages.get(&key).map(PageImage::screen) else { return false };
        // The page is drawn by the frame's own code onto a surface of the
        // still's size, with one pixel to each point, and the frame's
        // surface is put back after.
        let frame = (core::mem::replace(&mut self.surface, Surface::new(STILL_WIDTH, STILL_HEIGHT)), self.scale);
        self.scale = 1.0;
        self.surface.begin(DeviceBox::whole(STILL_WIDTH, STILL_HEIGHT));
        let whole = Rect::new(0.0, 0.0, f64::from(STILL_WIDTH), f64::from(STILL_HEIGHT));
        self.draw_fill(whole, 0.0, screen);
        self.draw_page(key, image_within(whole));
        self.surface.end();
        let words = core::mem::replace(&mut self.surface, frame.0).words;
        self.scale = frame.1;
        let Some(pixels) = Pixels::new(STILL_WIDTH, STILL_HEIGHT, words) else { return false };
        if self.stills.len() >= MAX_STILLS
            && !self.stills.contains_key(&still)
            && let Some(oldest) = self.stills.iter().min_by_key(|(_, kept)| kept.number).map(|(&number, _)| number)
        {
            self.stills.remove(&oldest);
        }
        let number = self.next_image;
        self.next_image = number.wrapping_add(1);
        self.stills.insert(still, Still { pixels: Arc::new(pixels), number });
        true
    }

    /// Draws a still, scaled to fill `area`.
    pub(crate) fn draw_still(&mut self, still: u32, area: Rect) {
        let Some(pixels) = self.stills.get(&still).map(|kept| Arc::clone(&kept.pixels)) else { return };
        let (width, height) = (f64::from(STILL_WIDTH), f64::from(STILL_HEIGHT));
        self.draw_image(&pixels, Sampling::Smooth, area, (0.0, 0.0, width, height));
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

    /// Row `index` of a page's teletext image, drawn in `row`, and nothing
    /// else of it: row 0 above a picture (renderer parity PAR-31) and row
    /// 24, the coloured-key prompts, beneath it (REQ-6379).
    fn draw_row(&mut self, key: PageKey, index: u16, row: Rect) {
        let Some(pixels) = self.pages.get(&key).map(|page| Arc::clone(&page.pixels)) else { return };
        let (width, height) = (f64::from(PAGE_WIDTH), f64::from(PAGE_HEIGHT));
        let lines = height / f64::from(interface_core::canvas::ROWS);
        let top = lines * f64::from(index);
        // The whole image is placed so that its row lies on `row`, and
        // only that row of it is filled.
        let (down, across) = (row.height() / lines, row.width() / width);
        let place = Rect::new(row.x0, row.y0 - top * down, row.x0 + width * across, row.y0 + (height - top) * down);
        self.draw_image(&pixels, Sampling::Nearest, place, (0.0, top, width, top + lines));
    }

    pub(crate) fn draw_page(&mut self, key: PageKey, area: Rect) {
        if let Some(regis) = self.regis.get(&key) {
            // A ReGIS picture across the image's width, its own proportions
            // kept, on its own background; a NAPLPS picture over the whole
            // image, as the teletext raster it is (REQ-5819).
            let (pixels, background, language) = (Arc::clone(&regis.pixels), regis.background, regis.language);
            let layout = interface_core::canvas::picture_layout(language, area, self.regis_contain, self.regis_keyed.contains(&key));
            let place = layout.picture;
            let (width, height) = (f64::from(language.width()), f64::from(language.height()));
            // At a whole multiple of its size each pixel is a block of
            // display pixels and is drawn as one (REQ-6401); at any other
            // size it is sampled smoothly, or thin lines would be dropped
            // from a picture shown smaller.
            let sampling = if whole_multiple(place.width() * self.scale, width) && whole_multiple(place.height() * self.scale, height) {
                Sampling::Nearest
            } else {
                Sampling::Smooth
            };
            self.draw_fill(area, 0.0, background);
            self.draw_image(&pixels, sampling, place, (0.0, 0.0, width, height));
            // The teletext page's row 0, its status area, header and clock,
            // above the picture, as row 24 is beneath it (PAR-31).
            self.draw_row(key, 0, layout.header);
            if let Some(row) = layout.keys {
                self.draw_row(key, interface_core::canvas::ROWS.saturating_sub(1), row);
            }
            return;
        }
        let Some(pixels) = self.pages.get(&key).map(|page| Arc::clone(&page.pixels)) else { return };
        let (width, height) = (f64::from(PAGE_WIDTH), f64::from(PAGE_HEIGHT));
        self.draw_image(&pixels, Sampling::Nearest, area, (0.0, 0.0, width, height));
    }
}
