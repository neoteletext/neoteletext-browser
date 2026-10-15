//! The tab: its state, layout, input and drawing. Because this code lays
//! everything out itself, it knows where every control is, and hit-tests the
//! pointer against the same rectangles it draws.

use std::time::{Duration, Instant};

use vello_cpu::kurbo::{Point, Rect};

use crate::pages::{PAGE_COUNT, Page, pages};
use crate::paint::{
    ACCENT, CAPTION, DETAIL, FIELD, FRAME, INK, LABEL, LINE, LINE_SOFT, LIST_TITLE, MUTED, OPTION, Painter, RAISED, READOUT,
    SMALL_MONO, TITLE, UNIT,
};
use crate::pose::Pose;
use crate::size::{
    DEFAULT_WIDTH, LIST_STOP, Layout, MAX_WIDTH, SWITCH_DWELL_SECONDS, clamp_width, grid_width, layout_for_width,
    max_width_for_window, next_layout,
};
use crate::tween::{EASE_OUT, TRANSFORM_EASE, Tween};

// ---- Header geometry, logical pixels ----
const HEADER_HEIGHT: f64 = 92.0;
const GUTTER: f64 = 24.0;
const LABEL_TOP: f64 = 14.0;
const CONTROL_TOP: f64 = 33.0;
const CONTROL_HEIGHT: f64 = 26.0;
const SCALE_TOP: f64 = 63.0;
const SLIDER_LEFT: f64 = GUTTER;
const SLIDER_WIDTH: f64 = 256.0;
const TRACK_HEIGHT: f64 = 6.0;
const THUMB_WIDTH: f64 = 10.0;
const THUMB_BORDER: f64 = 2.0;
const CONTROL_GAP: f64 = 22.0;
const READOUT_LEFT: f64 = SLIDER_LEFT + SLIDER_WIDTH + CONTROL_GAP;
const READOUT_WIDTH: f64 = 140.0;
const PICKER_LEFT: f64 = READOUT_LEFT + READOUT_WIDTH + CONTROL_GAP;
const PICKER_PADDING: f64 = 2.0;
const OPTION_PADDING: f64 = 12.0;
const OPTION_HEIGHT: f64 = 22.0;
const MOSAIC_SIZE: f64 = 36.0;
const MOSAIC_RADIUS: f64 = 8.0;
const IDENTITY_GAP: f64 = 12.0;
/// One press of an arrow key moves the slider this far.
const KEY_STEP: f32 = 8.0;

// ---- Wall geometry ----
const WALL_TOP_PADDING: f64 = 28.0;
const WALL_BOTTOM_PADDING: f64 = 40.0;
const COLUMN_GAP: f64 = 14.0;
const ROW_GAP: f64 = 28.0;
const CAPTION_GAP: f64 = 8.0;
const CAPTION_HEIGHT: f64 = 18.0;
const FRAME_RADIUS: f64 = 7.0;
/// Native teletext raster, 480 by 500 dots.
const PAGE_ASPECT: f64 = 500.0 / 480.0;
const LIST_MAX_WIDTH: f64 = 864.0;
const LIST_PAGE_WIDTH: f64 = 176.0;
const LIST_ROW_PADDING: f64 = 12.0;
const LIST_GAP: f64 = 20.0;
const SCROLL_BAR_WIDTH: f64 = 4.0;
/// Strips overlap their neighbour by this share of their height, so the
/// edges between them never show the background.
const STRIP_OVERLAP: f64 = 0.04;

/// The prototype's CSS transitions: width 160 ms ease-out, transforms 420 ms.
const WIDTH_TRANSITION: Duration = Duration::from_millis(160);
const TRANSFORM_TRANSITION: Duration = Duration::from_millis(420);
const CHANNEL_TITLE: &str = "Stand-in channel";

#[derive(Clone, Copy, PartialEq, Eq)]
enum WallTransform {
    Flat,
    Tilt,
    Fan,
}

impl WallTransform {
    const ALL: [Self; 3] = [Self::Flat, Self::Tilt, Self::Fan];

    fn label(self) -> &'static str {
        match self {
            Self::Flat => "Flat",
            Self::Tilt => "Tilt",
            Self::Fan => "Fan",
        }
    }
}

pub struct Tab {
    pages: Vec<Page>,
    width: f32,
    max: f32,
    layout: Layout,
    pending: Option<(Layout, Instant)>,
    transform: WallTransform,
    shown_width: Tween,
    tilt: Tween,
    fan: Tween,
    scroll: f64,
    pointer: Point,
    dragging: bool,
    /// The picker's options, where they were last drawn.
    options: [Rect; 3],
}

impl Tab {
    pub fn new() -> Self {
        Self {
            pages: pages(),
            width: DEFAULT_WIDTH,
            max: MAX_WIDTH,
            layout: layout_for_width(DEFAULT_WIDTH),
            pending: None,
            transform: WallTransform::Flat,
            shown_width: Tween::new(DEFAULT_WIDTH, WIDTH_TRANSITION, EASE_OUT),
            tilt: Tween::new(0.0, TRANSFORM_TRANSITION, TRANSFORM_EASE),
            fan: Tween::new(0.0, TRANSFORM_TRANSITION, TRANSFORM_EASE),
            scroll: 0.0,
            pointer: Point::ORIGIN,
            dragging: false,
            options: [Rect::ZERO; 3],
        }
    }

    fn set_width(&mut self, width: f32, now: Instant) {
        self.width = clamp_width(width, self.max);
        self.shown_width.set(self.width, now);
    }

    fn width_at(&self, x: f64) -> f32 {
        let fraction = ((x - SLIDER_LEFT - THUMB_WIDTH / 2.0) / (SLIDER_WIDTH - THUMB_WIDTH)).clamp(0.0, 1.0);
        #[allow(clippy::cast_possible_truncation, reason = "a fraction between 0 and 1")]
        let fraction = fraction as f32;
        LIST_STOP + fraction * (self.max - LIST_STOP)
    }

    pub fn pointer_moved(&mut self, x: f64, y: f64, now: Instant) -> bool {
        self.pointer = Point::new(x, y);
        if self.dragging {
            self.set_width(self.width_at(x), now);
        }
        self.dragging
    }

    pub fn pointer_pressed(&mut self, now: Instant) -> bool {
        if Rect::new(SLIDER_LEFT, CONTROL_TOP, SLIDER_LEFT + SLIDER_WIDTH, CONTROL_TOP + CONTROL_HEIGHT).contains(self.pointer) {
            self.dragging = true;
            self.set_width(self.width_at(self.pointer.x), now);
            return true;
        }
        for (choice, rect) in WallTransform::ALL.into_iter().zip(self.options) {
            if rect.contains(self.pointer) {
                self.transform = choice;
                self.tilt.set(f32::from(u8::from(choice == WallTransform::Tilt)), now);
                self.fan.set(f32::from(u8::from(choice == WallTransform::Fan)), now);
                return true;
            }
        }
        false
    }

    pub fn pointer_released(&mut self) -> bool {
        self.dragging = false;
        false
    }

    pub fn scroll_by(&mut self, distance: f64) -> bool {
        self.scroll = (self.scroll + distance).max(0.0);
        true
    }

    pub fn step(&mut self, direction: i8, now: Instant) -> bool {
        self.set_width(self.width + f32::from(direction) * KEY_STEP, now);
        true
    }

    /// The layout trails the thumb through the hysteresis band and a short
    /// dwell, so a fast sweep across the band does not switch it. Returns
    /// whether the layout changed.
    pub fn settle(&mut self, now: Instant) -> bool {
        let target = next_layout(self.layout, self.width);
        if target == self.layout {
            self.pending = None;
            return false;
        }
        match self.pending {
            Some((waiting_for, since)) if waiting_for == target => {
                if now.saturating_duration_since(since) >= Duration::from_secs_f32(SWITCH_DWELL_SECONDS) {
                    self.layout = target;
                    self.pending = None;
                    return true;
                }
                false
            }
            _ => {
                self.pending = Some((target, now));
                false
            }
        }
    }

    pub fn next_deadline(&self) -> Option<Instant> {
        self.pending.map(|(_, since)| since + Duration::from_secs_f32(SWITCH_DWELL_SECONDS))
    }

    pub fn animating(&self, now: Instant) -> bool {
        self.shown_width.moving(now) || self.tilt.moving(now) || self.fan.moving(now)
    }

    /// Draws the whole window; `width` and `height` are logical pixels.
    pub fn draw(&mut self, painter: &mut Painter, width: f64, height: f64, now: Instant) {
        #[allow(clippy::cast_possible_truncation, reason = "window widths fit easily in f32")]
        let window_width = width as f32;
        self.max = max_width_for_window(window_width);
        if self.width > self.max {
            self.set_width(self.width, now);
        }
        painter.fill(Rect::new(0.0, 0.0, width, height), 0.0, crate::paint::WELL);
        let content = match self.layout {
            Layout::Grid => self.draw_grid(painter, width, height, now),
            Layout::List => self.draw_list(painter, width, height),
        };
        let view = height - HEADER_HEIGHT;
        let total = content + WALL_TOP_PADDING + WALL_BOTTOM_PADDING;
        let max_scroll = (total - view).max(0.0);
        self.scroll = self.scroll.min(max_scroll);
        if max_scroll > 0.0 {
            let bar = view * view / total;
            let top = HEADER_HEIGHT + (view - bar) * self.scroll / max_scroll;
            let left = width - SCROLL_BAR_WIDTH * 2.0;
            painter.fill(Rect::new(left, top, left + SCROLL_BAR_WIDTH, top + bar), SCROLL_BAR_WIDTH / 2.0, LINE);
        }
        self.draw_header(painter, width);
    }

    fn draw_page(painter: &mut Painter, page: &Page, index: usize, rect: Rect, centre: Point, pose: &Pose) {
        let corner = |x: f64, y: f64| pose.place(index, centre, Point::new(x, y));
        if pose.tilt <= f64::EPSILON {
            painter.image(&page.image, page.width, page.height, corner(rect.x0, rect.y0), corner(rect.x1, rect.y0), corner(rect.x0, rect.y1));
            return;
        }
        // Tilted, each strip of the page gets its own exact affine transform.
        #[allow(clippy::cast_precision_loss, reason = "a few strips")]
        let strip = rect.height() / page.strips.len() as f64;
        for (number, image) in page.strips.iter().enumerate() {
            #[allow(clippy::cast_precision_loss, reason = "a few strips")]
            let top = rect.y0 + number as f64 * strip;
            let bottom = top + strip * (1.0 + STRIP_OVERLAP);
            painter.image(image, page.width, page.strip_height, corner(rect.x0, top), corner(rect.x1, top), corner(rect.x0, bottom));
        }
    }

    /// Draws the grid and returns its height.
    fn draw_grid(&self, painter: &mut Painter, width: f64, height: f64, now: Instant) -> f64 {
        let available = width - GUTTER * 2.0;
        let card_width = f64::from(grid_width(self.shown_width.value(now)));
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss, reason = "a small, positive column count")]
        let columns = (((available + COLUMN_GAP) / (card_width + COLUMN_GAP)).floor() as usize).max(1);
        let rows = self.pages.len().div_ceil(columns);
        let page_height = card_width * PAGE_ASPECT;
        let card_height = page_height + CAPTION_GAP + CAPTION_HEIGHT;
        #[allow(clippy::cast_precision_loss, reason = "small counts")]
        let content = rows as f64 * card_height + rows.saturating_sub(1) as f64 * ROW_GAP;
        let top = HEADER_HEIGHT + WALL_TOP_PADDING - self.scroll;
        let pose = Pose {
            tilt: f64::from(self.tilt.value(now)),
            fan: f64::from(self.fan.value(now)),
            wall_origin: Point::new(width / 2.0, top),
            perspective_origin: Point::new(width / 2.0, top + content / 2.0),
        };
        for (index, page) in self.pages.iter().enumerate() {
            let (row, column) = (index / columns, index % columns);
            let in_row = columns.min(self.pages.len() - row * columns);
            #[allow(clippy::cast_precision_loss, reason = "small counts")]
            let row_width = in_row as f64 * card_width + (in_row - 1) as f64 * COLUMN_GAP;
            #[allow(clippy::cast_precision_loss, reason = "small counts")]
            let left = width / 2.0 - row_width / 2.0 + column as f64 * (card_width + COLUMN_GAP);
            #[allow(clippy::cast_precision_loss, reason = "small counts")]
            let card_top = top + row as f64 * (card_height + ROW_GAP);
            if !pose.transformed() && (card_top > height || card_top + card_height < HEADER_HEIGHT) {
                continue;
            }
            let rect = Rect::new(left, card_top, left + card_width, card_top + page_height);
            let centre = Point::new(left + card_width / 2.0, card_top + card_height / 2.0);
            if !pose.transformed() {
                painter.fill(rect, FRAME_RADIUS, FRAME);
            }
            Self::draw_page(painter, page, index, rect, centre, &pose);
            // Captions are carried along by the transforms but not turned.
            let caption = pose.place(index, centre, Point::new(left, card_top + page_height + CAPTION_GAP));
            painter.text(caption.x, caption.y, &format!("P{:03X}", page.number), CAPTION, INK);
            let channel = format!("channel {}", page.channel);
            let channel_width = painter.width(&channel, SMALL_MONO);
            let channel_at = pose.place(index, centre, Point::new(left + card_width - channel_width, card_top + page_height + CAPTION_GAP + 2.0));
            painter.text(channel_at.x, channel_at.y, &channel, SMALL_MONO, MUTED);
        }
        content
    }

    /// Draws the list and returns its height.
    fn draw_list(&self, painter: &mut Painter, width: f64, height: f64) -> f64 {
        let list_width = (width - GUTTER * 2.0).min(LIST_MAX_WIDTH);
        let left = width / 2.0 - list_width / 2.0;
        let page_height = LIST_PAGE_WIDTH * PAGE_ASPECT;
        let row_height = page_height + LIST_ROW_PADDING * 2.0;
        let top = HEADER_HEIGHT + WALL_TOP_PADDING - self.scroll;
        for (index, page) in self.pages.iter().enumerate() {
            #[allow(clippy::cast_precision_loss, reason = "small counts")]
            let row_top = top + index as f64 * row_height;
            if row_top > height || row_top + row_height < HEADER_HEIGHT {
                continue;
            }
            if index == 0 {
                painter.fill(Rect::new(left, row_top, left + list_width, row_top + 1.0), 0.0, LINE);
            }
            painter.fill(Rect::new(left, row_top + row_height, left + list_width, row_top + row_height + 1.0), 0.0, LINE);
            let rect = Rect::new(left, row_top + LIST_ROW_PADDING, left + LIST_PAGE_WIDTH, row_top + LIST_ROW_PADDING + page_height);
            Self::draw_page(painter, page, index, rect, Point::ORIGIN, &Pose::FLAT);
            let text_left = rect.x1 + LIST_GAP;
            let middle = rect.y0 + page_height / 2.0;
            painter.text(text_left, middle - 26.0, &format!("P{:03X}", page.number), LIST_TITLE, INK);
            let channel = format!("channel {}", page.channel);
            let channel_width = painter.width(&channel, SMALL_MONO);
            painter.text(left + list_width - channel_width, middle - 22.0, &channel, SMALL_MONO, MUTED);
            painter.text(text_left, middle + 4.0, &format!("PAGE {:03X}    PICTURE 480 × 500    RENDERER vello_cpu", page.number), DETAIL, MUTED);
        }
        #[allow(clippy::cast_precision_loss, reason = "small counts")]
        let height = row_height * self.pages.len() as f64;
        height
    }

    fn draw_header(&mut self, painter: &mut Painter, width: f64) {
        painter.fill(Rect::new(0.0, 0.0, width, HEADER_HEIGHT), 0.0, RAISED);
        painter.fill(Rect::new(0.0, HEADER_HEIGHT - 1.0, width, HEADER_HEIGHT), 0.0, LINE_SOFT);

        // The slider: label, track filled up to the thumb, scale.
        painter.text(SLIDER_LEFT, LABEL_TOP, "THUMBNAIL WIDTH", LABEL, MUTED);
        let fraction = f64::from((self.width - LIST_STOP) / (self.max - LIST_STOP));
        let track_top = CONTROL_TOP + (CONTROL_HEIGHT - TRACK_HEIGHT) / 2.0;
        painter.fill(Rect::new(SLIDER_LEFT, track_top, SLIDER_LEFT + SLIDER_WIDTH, track_top + TRACK_HEIGHT), TRACK_HEIGHT / 2.0, LINE);
        let thumb_left = SLIDER_LEFT + fraction * (SLIDER_WIDTH - THUMB_WIDTH);
        painter.fill(Rect::new(SLIDER_LEFT, track_top, thumb_left + THUMB_WIDTH / 2.0, track_top + TRACK_HEIGHT), TRACK_HEIGHT / 2.0, ACCENT);
        painter.fill(Rect::new(thumb_left, CONTROL_TOP, thumb_left + THUMB_WIDTH, CONTROL_TOP + CONTROL_HEIGHT), 3.0, ACCENT);
        painter.fill(
            Rect::new(thumb_left + THUMB_BORDER, CONTROL_TOP + THUMB_BORDER, thumb_left + THUMB_WIDTH - THUMB_BORDER, CONTROL_TOP + CONTROL_HEIGHT - THUMB_BORDER),
            1.5,
            FIELD,
        );
        painter.text(SLIDER_LEFT, SCALE_TOP, "List", LABEL, if self.layout == Layout::List { ACCENT } else { MUTED });
        let max = format!("{}", self.max.round());
        let max_width = painter.width(&max, SMALL_MONO);
        painter.text(SLIDER_LEFT + SLIDER_WIDTH - max_width, SCALE_TOP, &max, SMALL_MONO, MUTED);

        // The readout.
        if self.layout == Layout::List {
            painter.text(READOUT_LEFT, CONTROL_TOP, "List", READOUT, INK);
        } else {
            let value = format!("{}", grid_width(self.width).round());
            let value_width = painter.width(&value, READOUT);
            painter.text(READOUT_LEFT, CONTROL_TOP, &value, READOUT, INK);
            painter.text(READOUT_LEFT + value_width + 4.0, CONTROL_TOP + 7.0, "px", UNIT, MUTED);
        }

        // The transform picker.
        painter.text(PICKER_LEFT, LABEL_TOP, "CSS TRANSFORM", LABEL, MUTED);
        let option_top = CONTROL_TOP + (CONTROL_HEIGHT - OPTION_HEIGHT) / 2.0;
        let mut left = PICKER_LEFT + PICKER_PADDING;
        for (slot, choice) in WallTransform::ALL.into_iter().enumerate() {
            let option_width = painter.width(choice.label(), OPTION) + OPTION_PADDING * 2.0;
            self.options[slot] = Rect::new(left, option_top, left + option_width, option_top + OPTION_HEIGHT);
            left += option_width;
        }
        painter.fill(Rect::new(PICKER_LEFT, option_top - PICKER_PADDING, left + PICKER_PADDING, option_top + OPTION_HEIGHT + PICKER_PADDING), 8.0, LINE_SOFT);
        for (choice, rect) in WallTransform::ALL.into_iter().zip(self.options) {
            let selected = choice == self.transform;
            if selected {
                painter.fill(rect, 6.0, FIELD);
            }
            painter.text(rect.x0 + OPTION_PADDING, rect.y0 + 4.0, choice.label(), OPTION, if selected { INK } else { MUTED });
        }

        // The channel's identity, at the right edge.
        let meta = format!("{PAGE_COUNT} teletext pages");
        let text_width = painter.width(CHANNEL_TITLE, TITLE).max(painter.width(&meta, OPTION));
        let text_left = width - GUTTER - text_width;
        let mosaic_left = text_left - IDENTITY_GAP - MOSAIC_SIZE;
        let mosaic_top = CONTROL_TOP + CONTROL_HEIGHT / 2.0 - MOSAIC_SIZE / 2.0 - 6.0;
        painter.mosaic(Rect::new(mosaic_left, mosaic_top, mosaic_left + MOSAIC_SIZE, mosaic_top + MOSAIC_SIZE), MOSAIC_RADIUS);
        painter.text(text_left, mosaic_top - 4.0, CHANNEL_TITLE, TITLE, INK);
        painter.text(text_left, mosaic_top + 22.0, &meta, OPTION, MUTED);
    }
}
