//! The drawing calls of a frame, kept to be compared with the last
//! frame's: each call the interface core makes, recorded with the box it
//! covers.

use interface_core::canvas::{Canvas, PageKey, Style};
use interface_core::geometry::Rect;
use interface_core::theme::Colour;

use crate::pages::PageImage;
use crate::text::INK_OVERHANG;
use crate::{DeviceBox, Painter};

/// One drawing call, kept to be compared with the last frame's and drawn
/// where the frame changed.
#[derive(Debug, PartialEq)]
pub(crate) enum Command {
    Fill {
        area: Rect,
        radius: f64,
        paint: Colour,
    },
    Polygon {
        points: Vec<interface_core::geometry::Point>,
        paint: Colour,
    },
    Dim {
        area: Rect,
        amount: f32,
    },
    PushClip(Rect),
    PopClip,
    Gradient {
        area: Rect,
        radius: f64,
        stops: Vec<(f32, Colour)>,
    },
    /// A page's image, by the number of the image it was.
    Page {
        key: PageKey,
        area: Rect,
        image: u64,
    },
    Text {
        area: Rect,
        text: String,
        style: Style,
        paint: Colour,
    },
}

/// A call as kept: with the clips it was drawn inside, and the device
/// pixels it can change.
#[derive(Debug)]
pub(crate) struct Recorded {
    command: Command,
    /// The clips around it, an index into [`Frame::clips`].
    clip: usize,
    /// The pixels it can change: its shape's, inside its clips. A clip's
    /// own are its rect's, since what it holds changes only there.
    bounds: DeviceBox,
}

impl Recorded {
    /// The call.
    pub(crate) fn command(&self) -> &Command {
        &self.command
    }

    /// The clips around it, an index into the frame's sets of clips.
    pub(crate) fn clip(&self) -> usize {
        self.clip
    }

    /// The pixels it can change.
    pub(crate) fn bounds(&self) -> DeviceBox {
        self.bounds
    }
}

/// A frame's calls.
#[derive(Debug, Default)]
pub(crate) struct Frame {
    calls: Vec<Recorded>,
    /// Every set of clips a call was drawn inside, outermost first, and the
    /// pixels they leave; the first is none, the whole surface.
    clips: Vec<(Vec<Rect>, DeviceBox)>,
    /// While drawing, the clips now around a call, as indices into `clips`.
    stack: Vec<usize>,
}

impl Frame {
    pub(crate) fn new(surface: DeviceBox) -> Self {
        Self { calls: Vec::new(), clips: vec![(Vec::new(), surface)], stack: vec![0] }
    }

    /// The frame's calls, in the order drawn.
    pub(crate) fn calls(&self) -> &[Recorded] {
        &self.calls
    }

    /// The pixels the set of clips `clip` leaves, none for a set there is
    /// not.
    pub(crate) fn clip_pixels(&self, clip: usize) -> DeviceBox {
        self.clips.get(clip).map_or_else(DeviceBox::default, |(_, area)| *area)
    }

    fn top(&self) -> usize {
        self.stack.last().copied().unwrap_or(0)
    }

    fn clip_box(&self) -> DeviceBox {
        self.clips.get(self.top()).map_or_else(DeviceBox::default, |(_, area)| *area)
    }

    pub(crate) fn rects(&self, clip: usize) -> &[Rect] {
        self.clips.get(clip).map_or(&[], |(rects, _)| rects.as_slice())
    }
}

impl Painter {
    /// Keeps a painting call, with the pixels it can change.
    fn record(&mut self, command: Command, shape: Rect) {
        let bounds = DeviceBox::covering(shape, self.scale, self.surface()).intersect(self.frame.clip_box());
        let clip = self.frame.top();
        self.frame.calls.push(Recorded { command, clip, bounds });
    }
}

/// The box around `points`.
pub(crate) fn around(points: &[interface_core::geometry::Point]) -> Rect {
    let mut found = Rect { x0: f64::INFINITY, y0: f64::INFINITY, x1: f64::NEG_INFINITY, y1: f64::NEG_INFINITY };
    for at in points {
        found = Rect { x0: found.x0.min(at.x), y0: found.y0.min(at.y), x1: found.x1.max(at.x), y1: found.y1.max(at.y) };
    }
    if points.is_empty() { Rect::default() } else { found }
}

/// Each call is kept, to be drawn by [`Painter::render`] where the frame
/// changed.
impl Canvas for Painter {
    fn fill(&mut self, area: Rect, radius: f64, paint: Colour) {
        self.record(Command::Fill { area, radius, paint }, area);
    }

    fn polygon(&mut self, points: &[interface_core::geometry::Point], paint: Colour) {
        if points.is_empty() {
            return;
        }
        self.record(Command::Polygon { points: points.to_vec(), paint }, around(points));
    }

    fn dim(&mut self, area: Rect, amount: f32) {
        if amount <= 0.0 {
            return;
        }
        self.record(Command::Dim { area, amount }, area);
    }

    fn push_clip(&mut self, area: Rect) {
        let parent = self.frame.top();
        let (mut rects, outer) = self.frame.clips.get(parent).cloned().unwrap_or_default();
        rects.push(area);
        let inside = DeviceBox::covering(area, self.scale, self.surface()).intersect(outer);
        let clip = self.frame.clips.len();
        self.frame.clips.push((rects, inside));
        // A clip changes what it holds only inside itself.
        self.frame.calls.push(Recorded { command: Command::PushClip(area), clip: parent, bounds: inside });
        self.frame.stack.push(clip);
    }

    fn pop_clip(&mut self) {
        if self.frame.stack.len() <= 1 {
            return;
        }
        let bounds = self.frame.clip_box();
        self.frame.stack.pop();
        let clip = self.frame.top();
        self.frame.calls.push(Recorded { command: Command::PopClip, clip, bounds });
    }

    fn gradient(&mut self, area: Rect, radius: f64, stops: &[(f32, Colour)]) {
        self.record(Command::Gradient { area, radius, stops: stops.to_vec() }, area);
    }

    fn screen(&mut self, key: PageKey) -> Option<Colour> {
        self.pages.get(&key).map(PageImage::screen)
    }

    fn page(&mut self, key: PageKey, area: Rect) -> bool {
        let Some(page) = self.pages.get(&key) else { return false };
        let image = page.number();
        self.record(Command::Page { key, area, image }, area);
        true
    }

    fn text(&mut self, area: Rect, text: &str, style: Style, paint: Colour) {
        // Text is drawn from the area's corner and cut at its edges; the
        // area is often far larger than the text, so the text's own box is
        // what it can change.
        let Some((width, height)) = self.layout(text, style).map(|layout| (f64::from(layout.width()), f64::from(layout.height()))) else { return };
        let over = f64::from(style.size) * INK_OVERHANG;
        let ink = Rect { x0: area.x0 - over, y0: area.y0 - over, x1: (area.x0 + width + over).min(area.x1), y1: (area.y0 + height + over).min(area.y1) };
        let shape = Rect { x0: ink.x0.max(area.x0), y0: ink.y0.max(area.y0), ..ink };
        self.record(Command::Text { area, text: text.to_owned(), style, paint }, shape);
    }

    fn text_width(&mut self, text: &str, style: Style) -> f64 {
        self.layout(text, style).map_or(0.0, |layout| f64::from(layout.width()))
    }

    fn line_height(&mut self, style: Style) -> f64 {
        self.layout("X", Style { caps: false, ..style }).map_or(0.0, |layout| f64::from(layout.height()))
    }
}
