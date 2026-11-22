//! The picture of a run: the decoder's own engine (`decoder::Engine`)
//! holds the page, a step's batch is applied to it by the decoder's own
//! code, and a snapshot is the decoder's own snapshot of it (script-api.md
//! section 16), so that a PNG is what the browser would show.
//!
//! Nothing of a batch is turned into packets or records here. The
//! decoder's side of a step (work package F4) is called as the decoder's
//! own loop calls it (`decoder::scripts`): the rates of section 10.5 are
//! counted (`decoder::scripts::rates`), the batch is applied to the open
//! page as one atomic frame (`Engine::apply_script`), the paintings it
//! asks to watch are watched (`Engine::script_watch`), and its sound
//! operations are carried out (`Engine::script_sound`). What is under
//! the pointer is the decoder's picking (`Engine::script_pick`), and the
//! watched drawings that became whole are the decoder's report
//! (`Engine::script_painted`). A batch the decoder refuses stops the
//! run, as it fails the script host in the browser.
//!
//! A picture is what the decoder paints, tick by tick, as its own loop
//! paints it: after a tick's batch the animation is brought to the tick
//! (`Engine::animation_step`: cycles, paint rates, WAIT and BLINK), and
//! the page's ReGIS or NAPLPS picture is taken as the decoder sends it
//! to the interface (`Engine::take_pictures`), whole at first and then
//! the areas that changed, each laid over the picture kept here as the
//! interface lays it. A teletext page is rendered at the tick
//! (`Engine::render_at`), with its sprites and at its flash phase. The
//! engine's settings are left as a browser's are at first: drawings
//! paint gradually and nothing is reduced, so a snapshot is the page at
//! that tick, part painted where the browser's would be.

use decoder::scripts::rates::{Rates, output_bytes};
use decoder::{Engine, ScriptPage};
use neoteletext_core::config::PresentationLevel;
use neoteletext_core::consts::{CELL_HEIGHT, CELL_WIDTH, PAGE_WIDTH};
use neoteletext_core::limits::SUBCODE_ANY;
use neoteletext_image::palette::Rgba;
use neoteletext_image::raster::{Raster, png};
use protocol::decoder::View;
use protocol::regis::RegisPicture;
use protocol::script::batch::Batch;
use protocol::script::{Level, Renderer};

use crate::limits::MICROSECONDS_PER_TICK;

/// The page and sub-page a run has open.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Place {
    /// The page, as three hexadecimal digits.
    pub page: u16,
    /// The sub-code.
    pub subcode: u16,
}

/// The language text is encoded for when a manifest names none the
/// engine has a set for: the decoder's own choice (`decoder::scripts`).
pub const DEFAULT_LANGUAGE: &str = "en";

/// The picture as last drawn whole: its width, a colour-map index for
/// each dot and the colours.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Shown {
    width: usize,
    indices: Vec<u8>,
    palette: Vec<[u8; 3]>,
}

/// The page of a run, in the decoder's engine.
pub struct Screen {
    engine: Engine,
    place: Place,
    /// What the page context draws in, and the designation of its
    /// language: set when the contexts are opened.
    renderer: Renderer,
    designation: Option<u8>,
    /// Whether the page's packet X/28/0 has been written with the page
    /// context's language, as the decoder writes it with the first batch
    /// that writes the page.
    designated: bool,
    rates: Rates,
    /// The page's ReGIS or NAPLPS picture as painted so far; none while
    /// the decoder has painted none, and for a teletext page.
    picture: Option<Shown>,
    /// The bytes of output every batch carried, as the decoder counts
    /// them against its rate.
    pub output_bytes: u64,
}

impl Screen {
    /// A screen showing `place`, with nothing fed.
    #[must_use]
    pub fn new(place: Place) -> Self {
        let mut engine = Engine::new();
        engine.open_page(Some(place.page));
        engine.set_sound_heard(true);
        Self { engine, place, renderer: Renderer::Teletext, designation: None, designated: false, rates: Rates::default(), picture: None, output_bytes: 0 }
    }

    /// The page and sub-page shown.
    #[must_use]
    pub const fn place(&self) -> Place {
        self.place
    }

    /// The page as the decoder's script code names it.
    const fn at(&self) -> ScriptPage {
        ScriptPage { page: self.place.page, sub: self.place.subcode, renderer: self.renderer, designation: self.designation }
    }

    /// Feeds a page file, a T42 or a TTI file as the engine's readers
    /// tell them apart, whole.
    pub fn page_file(&mut self, bytes: &[u8]) {
        self.engine.feed_whole(false, false, bytes);
        self.engine.finish();
    }

    /// What the page files fed say of their scripts: each sub-page's page
    /// and sub-code, and the name its `JS` line or `SCRIPT_REF` record
    /// gives (section 12.4), as the decoder's readers found them.
    pub fn script_names(&mut self) -> Vec<(u16, u16, Option<String>)> {
        self.engine.take_script_names()
    }

    /// The sub-page the engine shows of the page, when the run did not
    /// name one: the sub-code is then set to it, and to 0, no sub-pages,
    /// for a page the engine holds nothing of (a JavaScript page).
    pub fn settle_subpage(&mut self) {
        if self.place.subcode == SUBCODE_ANY {
            self.place.subcode = self.engine.shown_subcode(self.place.page).unwrap_or(0);
        }
        self.engine.set_view(View { page: self.place.page, subpage: Some(self.place.subcode), ..View::default() });
        self.engine.animation_showing(&[self.place.page]);
    }

    /// The renderer the page as fed is drawn with, as the decoder says
    /// it: none when the decoder holds nothing of the page.
    #[must_use]
    pub fn held_renderer(&self) -> Option<Renderer> {
        self.engine.script_renderer(self.place.page, self.place.subcode)
    }

    /// The presentation level the decoder's engine draws at, as a
    /// context's opening says it.
    #[must_use]
    pub fn level(&self) -> Level {
        match self.engine.config().level {
            PresentationLevel::Level1 => Level::One,
            PresentationLevel::Level15 => Level::OneFive,
            PresentationLevel::Level25 => Level::TwoFive,
            PresentationLevel::Level35 => Level::ThreeFive,
        }
    }

    /// The page context opens: what it draws in and the language its
    /// text is encoded for. A JavaScript page opens blank (section 12.3).
    pub fn open(&mut self, renderer: Renderer, language: &str) {
        use neoteletext_core::charset::language::language_value;
        self.renderer = renderer;
        self.designation =
            (renderer == Renderer::Teletext).then(|| language_value(language, true).or_else(|| language_value(DEFAULT_LANGUAGE, true))).flatten();
        // A JavaScript page opens blank (section 12.3). The decoder holds
        // nothing of it, and makes the page when its script arrives: a
        // page of any renderer is carried by a teletext page of its
        // number, as a channel's is (`decoder::scripts`, TODO-7406).
        if self.held_renderer().is_none() {
            let carrier = ScriptPage { page: self.place.page, sub: self.place.subcode, renderer: Renderer::Teletext, designation: None };
            self.engine.script_blank_page(carrier);
        }
    }

    /// Moves the engine's clock to `tick`.
    pub fn set_tick(&mut self, tick: u64) {
        self.engine.set_now_us(tick.saturating_mul(MICROSECONDS_PER_TICK));
    }

    /// Applies the batch of `tick`'s step to the page, as the decoder
    /// does. `draws` is false for a batch that draws nothing: a close's.
    ///
    /// # Errors
    /// What the decoder fails a script host for: a rate passed, or a
    /// batch it refuses whole.
    pub fn apply(&mut self, tick: u64, batch: &Batch, draws: bool) -> Result<(), String> {
        self.rates.count(tick, batch).map_err(|passed| format!("{}: the scripts passed a rate the decoder holds", passed.0))?;
        self.output_bytes = self.output_bytes.saturating_add(u64::try_from(output_bytes(batch)).unwrap_or(u64::MAX));
        if draws {
            let at = self.at();
            let designate = !self.designated && at.designation.is_some();
            self.engine.apply_script(at, batch, designate).map_err(|refused| format!("the decoder refused the batch: {}", refused.0))?;
            if batch.teletext_rows.is_some() || batch.teletext_enhancement.is_some() || batch.teletext_settings.is_some() {
                self.designated = true;
            }
            if let Some(painted) = batch.watch.as_ref().and_then(|watch| watch.painted.as_ref()) {
                self.engine.script_watch(at, painted);
            }
        }
        if let Some(operations) = &batch.sound {
            self.engine.script_sound(self.place.page, operations);
        }
        Ok(())
    }

    /// The watched drawings that became whole since the last call, by
    /// their handles: the decoder's report for the next step.
    pub fn painted(&mut self) -> Vec<i32> {
        let at = self.at();
        self.engine.script_painted(at)
    }

    /// What is under the pointer at `x`, `y` in the renderer's own units
    /// (section 4.4: a teletext page's cell, else the picture's pixel or
    /// dot), by the script host's handles, topmost first: the decoder's
    /// picking.
    #[must_use]
    pub fn pick(&self, x: i32, y: i32) -> Vec<i32> {
        let (Ok(x), Ok(y)) = (u16::try_from(x), u16::try_from(y)) else { return Vec::new() };
        // The decoder picks by the picture's pixel; a cell is picked at
        // its top left.
        let pixel = |cell: u16, size: usize| u16::try_from(usize::from(cell).saturating_mul(size)).unwrap_or(u16::MAX);
        let (x, y) = if self.renderer == Renderer::Teletext { (pixel(x, CELL_WIDTH), pixel(y, CELL_HEIGHT)) } else { (x, y) };
        self.engine.script_pick(self.at(), x, y)
    }

    /// The voice operations the page's own records and the scripts'
    /// sound operations caused since the last call.
    pub fn take_sounds(&mut self) -> Vec<protocol::audio::Audio> {
        self.engine.take_sounds()
    }

    /// Lays a picture the decoder sends, whole or a part, over the one
    /// kept, as the interface does.
    fn lay(&mut self, picture: &RegisPicture) {
        let (wide, high) = (usize::from(picture.language.width()), usize::from(picture.language.height()));
        let shown = self.picture.get_or_insert_with(Shown::default);
        if picture.whole() || shown.width != wide {
            shown.width = wide;
            shown.indices = vec![0; wide.saturating_mul(high)];
        }
        shown.palette = picture.palette.to_vec();
        let area = picture.area;
        let across = usize::from(area.width);
        let mut dots = Vec::with_capacity(picture.packed.len().saturating_mul(2));
        for byte in &picture.packed {
            dots.push(byte >> 4);
            dots.push(byte & 0x0F);
        }
        for (line, dots) in dots.chunks(across.max(1)).enumerate().take(usize::from(area.height)) {
            let start = usize::from(area.y).saturating_add(line).saturating_mul(shown.width).saturating_add(usize::from(area.x));
            if let Some(target) = shown.indices.get_mut(start..start.saturating_add(dots.len())) {
                target.copy_from_slice(dots);
            }
        }
    }

    /// Paints the tick, as the decoder's loop does after the scripts'
    /// batch: the animation is brought to the tick, and the picture the
    /// decoder would send, if it changed, is laid over the one kept.
    pub fn paint(&mut self) {
        let now = self.engine.now_us();
        let showing = [self.place.page];
        self.engine.animation_step(&showing, now);
        let _ = self.engine.take_animated(&showing);
        let (regis, naplps) = self.engine.take_pictures(self.place.page, Some(self.place.subcode), now);
        for drawn in [regis, naplps].into_iter().flatten() {
            self.lay(&drawn.picture);
            for part in &drawn.more {
                self.lay(part);
            }
        }
    }

    /// The page as a PNG, as painted at the tick: the ReGIS or NAPLPS
    /// picture the decoder has sent, or the teletext page rendered.
    ///
    /// # Errors
    /// When the decoder has nothing of the page to draw, or the
    /// PNG writer refuses the picture.
    pub fn snapshot(&mut self) -> Result<Vec<u8>, String> {
        let shown = if let Some(picture) = &self.picture {
            picture.clone()
        } else {
            let rendered = self.engine.render_at(self.place.page, self.engine.now_us()).ok_or("the decoder holds nothing of the page to draw")?;
            Shown { width: PAGE_WIDTH, indices: rendered.indices, palette: rendered.palette }
        };
        let raster = Raster {
            width: shown.width,
            height: shown.indices.len().checked_div(shown.width).unwrap_or(0),
            indices: shown.indices,
            palette: shown.palette.iter().map(|[r, g, b]| Rgba { r: *r, g: *g, b: *b, a: u8::MAX }).collect(),
        };
        png(&raster).map_err(|error| format!("the PNG writer refused the picture: {error:?}"))
    }
}
