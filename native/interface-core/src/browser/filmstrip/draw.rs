//! The filmstrip drawn (REQ-6640 items 1 to 7): a slightly transparent
//! panel under the tab strip, slid out by how far it has opened, and on it
//! one row of pictures, each captioned above with its tab's title and page
//! number, the active tab's outlined, the one the pointer rests on with its
//! close and mute buttons.

use alloc::format;
use alloc::string::String;

use super::{Picture, Still};
use crate::browser::Browser;
use crate::browser::banks::BANK_BORDER;
use crate::browser::glyphs::speaker_glyph;
use crate::browser::tab_strip::STRIP_HEIGHT;
use crate::canvas::{Canvas, Style, image_within};
use crate::geometry::Rect;
use crate::text::{border, centred, left_centred, outline};
use crate::theme::{BLACK, BLUE, BUTTON, CAPTION, CYAN, GLYPH, MUTED, ROW_STATUS, RULE, SELECTED, TAB_COLOURS, WHITE, WINDOW, YELLOW};

/// Pictures across the window: each about a fifth of its width (DEC-6637
/// item 7).
const PICTURES_ACROSS: f64 = 5.0;
/// The room at the row's ends, at its start and when slid to an end.
pub(super) const MARGIN: f64 = 16.0;
/// The room between two pictures: more than a caption's gap to its page
/// number, so that each caption reads as its own picture's.
const GAP: f64 = 24.0;
/// A caption's height: one line of the caption style with room around it.
const CAPTION_HEIGHT: f64 = 26.0;
/// The room above the captions and below the pictures.
const PANEL_TOP: f64 = 8.0;
const PANEL_BOTTOM: f64 = 16.0;
/// How dark the panel's ground makes the window beneath: dark enough for
/// white captions to read on any page, light enough that the window shows
/// faintly through (DEC-6637 item 1, "slightly transparent").
const GROUND: f32 = 0.92;
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
    /// Where the picture of `tab`, shown alone, starts across (REQ-6921
    /// item 1): centred directly under the tab as the strip shows it, moved
    /// only as far as keeps it whole within the window's margins; none
    /// while the tab is not on the strip.
    pub(super) fn alone_left(&self, layout: &Layout, tab: usize) -> Option<f64> {
        let region = self.regions.tabs.iter().find(|region| region.index == tab)?;
        let centred = f64::midpoint(region.tab.x0, region.tab.x1) - layout.picture / 2.0;
        let most = (self.filmstrip.width - MARGIN - layout.picture).max(MARGIN);
        Some(centred.clamp(MARGIN, most))
    }

    /// The row's slide that puts the picture of `tab` where it shows alone
    /// (DEC-6920 item 6): the picture alone is drawn at this slide too, so
    /// that it stays exactly where it is when the rest of the row appears.
    pub(super) fn alone_slide(&self, layout: &Layout, tab: usize) -> Option<f32> {
        Some(super::narrow(self.alone_left(layout, tab)? - row_left(layout, tab, 0.0)))
    }

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
            self.ask_thumbnails(now);
            return;
        }
        // REQ-6921: in its first stage only the rested tab's picture shows,
        // under its tab, on a panel only as wide as it and its margins.
        let alone = self.filmstrip.alone.and_then(|alone| Some((alone.tab, row_left(&layout, alone.tab, self.alone_slide(&layout, alone.tab)?))));
        let (panel_left, panel_right) = alone.map_or((0.0, width), |(_, left)| (left - MARGIN, left + layout.picture + MARGIN));
        let shown = layout.panel * reveal;
        let panel = Rect::new(panel_left, STRIP_HEIGHT, panel_right, STRIP_HEIGHT + shown);
        canvas.dim(panel, GROUND);
        let mut band_top = panel.y1;
        for amount in SHADOW {
            canvas.dim(Rect::new(panel_left, band_top, panel_right, band_top + SHADOW_BAND), amount);
            band_top += SHADOW_BAND;
        }
        canvas.fill(Rect::new(panel_left, panel.y1 - EDGE, panel_right, panel.y1), 0.0, SELECTED);
        // Slid out from under the strip: its top rises out of sight.
        let top = panel.y1 - layout.panel + PANEL_TOP;
        let offset = self.filmstrip.slide.value(now);
        canvas.push_clip(Rect::new(panel_left, panel.y0, panel_right, panel.y1 - EDGE));
        // The row slides under a still pointer: the picture it rests on is
        // found again as each is laid out.
        let shown = self.filmstrip.shown();
        self.filmstrip.hovered = None;
        for index in 0..self.tabs.len() {
            let in_row = row_left(&layout, index, offset);
            let x = match alone {
                Some((tab, left)) if tab == index => left,
                Some(_) => continue,
                None => in_row,
            };
            if x + layout.picture < 0.0 || x > width {
                continue;
            }
            let caption = Rect::new(x, top, x + layout.picture, top + CAPTION_HEIGHT);
            let picture = Rect::new(x, caption.y1, x + layout.picture, caption.y1 + layout.height);
            if shown && Rect::new(caption.x0, caption.y0, picture.x1, picture.y1).contains(self.pointer) {
                self.filmstrip.hovered = Some(index);
            }
            let drawn = self.draw_picture(canvas, index, caption, picture);
            self.filmstrip.pictures.push(drawn);
        }
        canvas.pop_clip();
        self.filmstrip.panel = panel;
        self.ask_thumbnails(now);
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
        // The caption: the title, cut to leave room for the page number,
        // which follows it, so that each reads as the picture's below.
        let number = still.map(|still| still.page).or(tab.page).map(|page| format!("P{page:03X}")).unwrap_or_default();
        let number_width = canvas.text_width(&number, ROW_STATUS);
        let room = if number.is_empty() { caption.width() } else { caption.width() - number_width - CAPTION_GAP };
        let title_width = canvas.text_width(&tab.title, CAPTION).min(room.max(0.0));
        let title = Rect::new(caption.x0, caption.y0, caption.x0 + title_width, caption.y1);
        canvas.push_clip(title);
        left_centred(canvas, title, 0.0, &tab.title, CAPTION, WHITE);
        canvas.pop_clip();
        let number_area = Rect::new(title.x1 + CAPTION_GAP, caption.y0, title.x1 + CAPTION_GAP + number_width, caption.y1);
        left_centred(canvas, number_area, 0.0, &number, ROW_STATUS, MUTED);
        if !draw_still(canvas, tab.channel, still, picture) {
            draw_card(canvas, tab, picture);
        }
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

/// A tab's still in `picture`; false, drawing nothing, when it has none.
fn draw_still<C: Canvas + ?Sized>(canvas: &mut C, channel: u32, still: Option<Still>, picture: Rect) -> bool {
    still.is_some_and(|still| still.kept) && canvas.still(channel, picture)
}

/// The teletext page's rows and columns, as the card lays them out
/// (EN 300 706 clause 2: 25 rows of 40 characters).
const CARD_ROWS: f64 = 25.0;
const CARD_COLUMNS: f64 = 40.0;
/// The most characters on one of the card's rows: the row's 40 less a
/// cell at each side, as a page leaves for its attributes.
const CARD_LINE: usize = 38;
/// The card's rows (`docs/tab-thumbnails.md` section 12), counted from 0
/// as teletext counts them.
const CARD_NAME_ROW: f64 = 1.0;
const CARD_PAGE_ROW: f64 = 9.0;
const CARD_TITLE_ROW: f64 = 13.0;
const CARD_TITLE_ROWS: usize = 2;
const CARD_NOTE_ROW: f64 = 23.0;
/// How much the card is darkened: a stand-in at the filmstrip's dimmed
/// strength, so it reads as no picture of the page (REQ-6887 item 2).
const CARD_DIM: f32 = 0.35;
/// The dashed border's dash and gap, and its width, in points.
const DASH: f64 = 6.0;
const DASH_GAP: f64 = 4.0;
const DASH_WIDTH: f64 = 2.0;
/// What the card says at its foot.
const NO_PICTURE: &str = "No picture yet";

/// `text` cut to at most `count` characters.
fn cut(text: &str, count: usize) -> &str {
    text.char_indices().nth(count).map_or(text, |(at, _)| text.get(..at).unwrap_or(text))
}

/// The stand-in card of a tab with no still (REQ-6887): drawn as a
/// teletext page from what the browser knows of the tab, the channel's
/// name in a band of the tab's colour, the page number in double height,
/// the page's title and "No picture yet", darkened and in a dashed border
/// so that it is never taken for a picture of the page. Nothing of it is
/// kept.
fn draw_card<C: Canvas + ?Sized>(canvas: &mut C, tab: &crate::browser::TabInfo, picture: Rect) {
    canvas.fill(picture, 0.0, WINDOW);
    let page = image_within(picture);
    canvas.fill(page, 0.0, BLACK);
    let row = page.height() / CARD_ROWS;
    let column = page.width() / CARD_COLUMNS;
    let at = |line: f64, rows: f64| Rect::new(page.x0 + column, page.y0 + line * row, page.x1 - column, page.y0 + (line + rows) * row);
    let style = |rows: f64| Style { size: narrow_size(row * rows), monospace: true, bold: false, caps: false };
    let (band, ink) = TAB_COLOURS.get(usize::from(tab.colour)).copied().unwrap_or((BLUE, WHITE));
    let name = at(CARD_NAME_ROW, 1.0);
    canvas.fill(Rect::new(page.x0, name.y0, page.x1, name.y1), 0.0, band);
    canvas.push_clip(page);
    canvas.text(name, cut(&tab.title, CARD_LINE), style(1.0), ink);
    let number = match (tab.page, tab.subpage) {
        (Some(page), Some(sub)) => format!("P{page:03X}/{sub:04X}"),
        (Some(page), None) => format!("P{page:03X}"),
        (None, _) => String::from("Start page"),
    };
    canvas.text(at(CARD_PAGE_ROW, 2.0), &number, style(2.0), YELLOW);
    let mut rest = tab.page_title.as_str();
    for line in 0..CARD_TITLE_ROWS {
        let shown = cut(rest, CARD_LINE);
        #[allow(clippy::cast_precision_loss, reason = "a row of two")]
        canvas.text(at(CARD_TITLE_ROW + line as f64, 1.0), shown, style(1.0), WHITE);
        rest = rest.get(shown.len()..).unwrap_or_default().trim_start();
    }
    canvas.text(at(CARD_NOTE_ROW, 1.0), NO_PICTURE, style(1.0), CYAN);
    canvas.pop_clip();
    canvas.dim(picture, CARD_DIM);
    dashed_border(canvas, picture);
}

/// A dashed border just inside `rect`, in the panel's edge colour.
fn dashed_border<C: Canvas + ?Sized>(canvas: &mut C, rect: Rect) {
    let mut x = rect.x0;
    while x < rect.x1 {
        let end = (x + DASH).min(rect.x1);
        canvas.fill(Rect::new(x, rect.y0, end, rect.y0 + DASH_WIDTH), 0.0, SELECTED);
        canvas.fill(Rect::new(x, rect.y1 - DASH_WIDTH, end, rect.y1), 0.0, SELECTED);
        x = end + DASH_GAP;
    }
    let mut y = rect.y0;
    while y < rect.y1 {
        let end = (y + DASH).min(rect.y1);
        canvas.fill(Rect::new(rect.x0, y, rect.x0 + DASH_WIDTH, end), 0.0, SELECTED);
        canvas.fill(Rect::new(rect.x1 - DASH_WIDTH, y, rect.x1, end), 0.0, SELECTED);
        y = end + DASH_GAP;
    }
}

/// A text size in points, held within `f32`.
#[expect(clippy::cast_possible_truncation, reason = "a row's height, far within f32")]
fn narrow_size(points: f64) -> f32 {
    points as f32
}

/// Where picture `index` of the row starts across with the row slid by
/// `slide`: the one sum both the picture alone and the row use, so that
/// the same slide puts a picture at the same place in both.
fn row_left(layout: &Layout, index: usize, slide: f32) -> f64 {
    #[allow(clippy::cast_precision_loss, reason = "a tab's place, below a hundred")]
    let place = index as f64 * layout.step;
    MARGIN + place + f64::from(slide)
}
