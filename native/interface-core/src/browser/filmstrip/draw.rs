//! The filmstrip drawn (REQ-6640 items 1 to 7): a slightly transparent
//! panel under the tab strip, slid out by how far it has opened, and on it
//! one row of pictures, each captioned above with its tab's title and page
//! number, the active tab's outlined, the one the pointer rests on with its
//! close and mute buttons.

use alloc::format;

use super::{Picture, Still};
use crate::browser::Browser;
use crate::browser::banks::BANK_BORDER;
use crate::browser::glyphs::speaker_glyph;
use crate::browser::tab_strip::STRIP_HEIGHT;
use crate::canvas::{Canvas, image_within, regis_contained};
use crate::geometry::Rect;
use crate::text::{border, centred, left_centred, outline};
use crate::theme::{BLACK, BUTTON, CAPTION, GLYPH, MUTED, ROW_STATUS, RULE, SELECTED, WHITE, WINDOW};

/// Pictures across the window: each about a fifth of its width (DEC-6637
/// item 7).
const PICTURES_ACROSS: f64 = 5.0;
/// The room at the row's ends, at its start and when slid to an end.
pub(super) const MARGIN: f64 = 16.0;
/// The room between two pictures.
const GAP: f64 = 16.0;
/// A caption's height: one line of the caption style with room around it.
const CAPTION_HEIGHT: f64 = 26.0;
/// The room above the captions and below the pictures.
const PANEL_TOP: f64 = 8.0;
const PANEL_BOTTOM: f64 = 16.0;
/// How dark the panel's ground makes the window beneath: dark enough for
/// white captions to read on any page, light enough that the window shows
/// faintly through (DEC-6637 item 1, "slightly transparent").
const GROUND: f32 = 0.82;
/// The panel's edge at its foot, in the selection's colour, and the shadow
/// below it, each band and how much it darkens, fading out: the overflow
/// panel's, which told it apart from the window (decision 102).
const EDGE: f64 = 2.0;
const SHADOW_BAND: f64 = 3.0;
const SHADOW: [f32; 4] = [0.5, 0.35, 0.2, 0.1];
/// A picture's close and mute buttons, and their room from its corner.
const BUTTON_SIZE: f64 = 28.0;
const BUTTON_INSET: f64 = 6.0;
/// The room between a caption's title and its page number.
const CAPTION_GAP: f64 = 8.0;

/// The row laid out across a window.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Layout {
    /// A picture's width and height.
    pub(super) picture: f64,
    height: f64,
    /// A picture and its gap.
    pub(super) step: f64,
    /// The slide's least and most (REQ-6640 item 6): at the least the last
    /// picture has come all the way across to the window's left margin; at
    /// the most the first has gone all the way across to its right margin.
    pub(super) bounds: (f64, f64),
    /// The panel's whole height, slid out.
    panel: f64,
}

impl Layout {
    /// The row of `count` pictures across a window `width` wide, each
    /// page `height_per_width` as high as it is wide (the View menu's
    /// page shape, REQ-737).
    pub(super) fn of(width: f64, count: usize, height_per_width: f64) -> Self {
        let picture = ((width - 2.0 * MARGIN - (PICTURES_ACROSS - 1.0) * GAP) / PICTURES_ACROSS).max(0.0);
        let height = picture * height_per_width;
        let step = picture + GAP;
        #[allow(clippy::cast_precision_loss, reason = "a count of tabs, below a hundred")]
        let least = -(count.saturating_sub(1) as f64) * step;
        let most = (width - 2.0 * MARGIN - picture).max(least);
        Self { picture, height, step, bounds: (least, most), panel: PANEL_TOP + CAPTION_HEIGHT + height + PANEL_BOTTOM }
    }
}

impl Browser {
    /// The filmstrip, over everything below the strip, as far as it has
    /// slid out at `now`; nothing while it is under the strip. Its stills
    /// are taken first, from the pages the canvas holds before this frame
    /// replaces them.
    pub(in crate::browser) fn draw_filmstrip<C: Canvas + ?Sized>(&mut self, canvas: &mut C, width: f64, now: u64) {
        self.take_stills(canvas);
        let layout = Layout::of(width, self.tabs.len(), self.wall.shape().height_per_width());
        self.filmstrip.width = width;
        self.filmstrip.bounds = layout.bounds;
        self.filmstrip.step = layout.step;
        self.filmstrip.pictures.clear();
        self.filmstrip.panel = Rect::ZERO;
        let reveal = f64::from(self.filmstrip.reveal.value(now)).clamp(0.0, 1.0);
        if reveal <= 0.0 {
            return;
        }
        let shown = layout.panel * reveal;
        let panel = Rect::new(0.0, STRIP_HEIGHT, width, STRIP_HEIGHT + shown);
        canvas.dim(panel, GROUND);
        let mut band_top = panel.y1;
        for amount in SHADOW {
            canvas.dim(Rect::new(0.0, band_top, width, band_top + SHADOW_BAND), amount);
            band_top += SHADOW_BAND;
        }
        canvas.fill(Rect::new(0.0, panel.y1 - EDGE, width, panel.y1), 0.0, SELECTED);
        // Slid out from under the strip: its top rises out of sight.
        let top = panel.y1 - layout.panel + PANEL_TOP;
        let offset = f64::from(self.filmstrip.slide.value(now));
        canvas.push_clip(Rect::new(0.0, panel.y0, width, panel.y1 - EDGE));
        for index in 0..self.tabs.len() {
            #[allow(clippy::cast_precision_loss, reason = "a tab's place, below a hundred")]
            let x = MARGIN + index as f64 * layout.step + offset;
            if x + layout.picture < 0.0 || x > width {
                continue;
            }
            let caption = Rect::new(x, top, x + layout.picture, top + CAPTION_HEIGHT);
            let picture = Rect::new(x, caption.y1, x + layout.picture, caption.y1 + layout.height);
            let drawn = self.draw_picture(canvas, index, caption, picture);
            self.filmstrip.pictures.push(drawn);
        }
        canvas.pop_clip();
        self.filmstrip.panel = panel;
    }

    /// One tab's caption and picture, with its buttons while the pointer
    /// rests on it.
    fn draw_picture<C: Canvas + ?Sized>(&self, canvas: &mut C, index: usize, caption: Rect, picture: Rect) -> Picture {
        let close =
            Rect::new(picture.x1 - BUTTON_INSET - BUTTON_SIZE, picture.y0 + BUTTON_INSET, picture.x1 - BUTTON_INSET, picture.y0 + BUTTON_INSET + BUTTON_SIZE);
        let speaker = Rect::new(close.x0 - BUTTON_INSET - BUTTON_SIZE, close.y0, close.x0 - BUTTON_INSET, close.y1);
        let region = Picture { index, rect: Rect::new(caption.x0, caption.y0, picture.x1, picture.y1), close, speaker };
        let Some(tab) = self.tabs.get(index) else { return region };
        let still = self.filmstrip.still(tab.channel).copied();
        // The caption: the title, and the page number at the right.
        let number = still.map(|still| format!("P{:03X}", still.page)).unwrap_or_default();
        let number_width = canvas.text_width(&number, ROW_STATUS);
        let title = Rect::new(caption.x0, caption.y0, caption.x1 - number_width - CAPTION_GAP, caption.y1);
        canvas.push_clip(title);
        left_centred(canvas, title, 0.0, &tab.title, CAPTION, WHITE);
        canvas.pop_clip();
        let number_area = Rect::new(caption.x1 - number_width, caption.y0, caption.x1, caption.y1);
        left_centred(canvas, number_area, 0.0, &number, ROW_STATUS, MUTED);
        draw_still(canvas, tab.channel, still, picture);
        let active = index == self.active;
        let hovered = self.filmstrip.hovered == Some(index);
        if active {
            let around = Rect::new(picture.x0 - BANK_BORDER, picture.y0 - BANK_BORDER, picture.x1 + BANK_BORDER, picture.y1 + BANK_BORDER);
            border(canvas, around, BANK_BORDER, WHITE);
        } else if hovered {
            outline(canvas, picture, MUTED);
        } else {
            outline(canvas, picture, RULE);
        }
        if hovered {
            canvas.fill(close, BUTTON_SIZE / 2.0, BUTTON);
            outline(canvas, close, RULE);
            centred(canvas, close, "\u{00D7}", GLYPH, WHITE);
        }
        if hovered || self.filmstrip_speaker_shown(index) {
            canvas.fill(speaker, BUTTON_SIZE / 2.0, BUTTON);
            outline(canvas, speaker, RULE);
            speaker_glyph(canvas, speaker, tab.muted, WHITE);
        }
        region
    }
}

/// A tab's still in `picture`, or its stand-in (REQ-6640 item 3): black in
/// the picture's shape for a page drawn in ReGIS or NAPLPS, else a blank
/// page with its screen, as the wall stands in for a page not yet drawn.
fn draw_still<C: Canvas + ?Sized>(canvas: &mut C, channel: u32, still: Option<Still>, picture: Rect) {
    if still.is_some_and(|still| still.kept) && canvas.still(channel, picture) {
        return;
    }
    canvas.fill(picture, 0.0, WINDOW);
    let image = image_within(picture);
    let shape = if still.is_some_and(|still| still.picture) { regis_contained(image) } else { image };
    canvas.fill(shape, 0.0, BLACK);
}
