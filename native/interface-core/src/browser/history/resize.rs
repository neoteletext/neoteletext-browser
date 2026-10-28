//! The history panel's width: either side edge pulled with the main button
//! makes the panel wider or narrower, as the console's grabber makes the
//! console taller or shorter (`console_panel/resize.rs`, decision 120).
//! The panel stays centred over the window below the strip, so both edges
//! move together. It is never narrower than a short title and its address
//! need, and never so wide that no page shows either side of it. The width
//! is kept for the rest of the run, as the console's height is
//! (decision 127).

use super::{CHECK_SIZE, GAP, HistoryUi, PADDING, SMALL_BUTTON, TIME_WIDTH};
use crate::browser::Browser;
use crate::canvas::Canvas;
use crate::geometry::{Point, Rect};
use crate::theme::{DIM, MUTED};

/// The room a row gives its title, and the same again its address, at the
/// narrowest panel: about eighteen characters of the row's face, enough
/// for a channel's name and a page number or a short page title.
const SHORT_TEXT_WIDTH: f64 = 150.0;
/// The narrowest panel: a row's check box, its time, a short title and
/// address with the gaps between them, and its menu button, inside the
/// panel's padding (the parts `draw.rs` lays a row out from).
pub(super) const MIN_WIDTH: f64 = 2.0 * PADDING + CHECK_SIZE + GAP + TIME_WIDTH + 2.0 * SHORT_TEXT_WIDTH + 2.0 * GAP + SMALL_BUTTON;
/// What the widest panel leaves uncovered on each side of the window below
/// the strip: a strip of the pages, so the panel still reads as over them,
/// and room for each edge's grip clear of the window's own edge, where the
/// system's window resize takes the pointer.
const UNCOVERED: f64 = 40.0;
/// How far either side of an edge it can be gripped: an edge is a line,
/// too thin to find by itself. Less than the panel's padding, so the grip
/// never reaches the close button or a row's menu button.
const GRIP_REACH: f64 = 6.0;
/// The handle drawn just inside each edge, so it can be seen and found:
/// the console grabber's turned on its side.
const HANDLE_WIDTH: f64 = 3.0;
const HANDLE_HEIGHT: f64 = 48.0;

/// Which of the panel's side edges is held.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Edge {
    Left,
    Right,
}

/// An edge held, and where on it, from the edge, while it is pulled.
#[derive(Clone, Copy, Debug)]
pub(super) struct Grip {
    edge: Edge,
    offset: f64,
}

impl Browser {
    /// The panel's width in the window below the strip, `body`: as pulled,
    /// within its bounds.
    pub(super) fn history_width(&self, body: Rect) -> f64 {
        self.history.width.clamp(MIN_WIDTH, widest(body))
    }

    /// The grips over the panel's side edges at `area`, and their handles,
    /// lit while the pointer is on one or pulling it.
    pub(super) fn draw_history_edges<C: Canvas + ?Sized>(&mut self, canvas: &mut C, area: Rect) {
        self.history.edges = [
            (Edge::Left, Rect::new(area.x0 - GRIP_REACH, area.y0, area.x0 + GRIP_REACH, area.y1)),
            (Edge::Right, Rect::new(area.x1 - GRIP_REACH, area.y0, area.x1 + GRIP_REACH, area.y1)),
        ];
        let lit = self.over_history_edge();
        let middle = f64::midpoint(area.y0, area.y1);
        for x in [area.x0 + GRIP_REACH / 2.0, area.x1 - GRIP_REACH / 2.0] {
            let handle = Rect::new(x - HANDLE_WIDTH / 2.0, middle - HANDLE_HEIGHT / 2.0, x + HANDLE_WIDTH / 2.0, middle + HANDLE_HEIGHT / 2.0);
            canvas.fill(handle, HANDLE_WIDTH / 2.0, if lit { MUTED } else { DIM });
        }
    }

    /// A press on an edge's grip grips that edge where it was pressed.
    pub(super) fn history_edge_press(&mut self, point: Point) -> bool {
        let Some(&(edge, _)) = self.history.edges.iter().find(|(_, rect)| rect.contains(point)) else { return false };
        let panel = self.history.controls.panel;
        let at = match edge {
            Edge::Left => panel.x0,
            Edge::Right => panel.x1,
        };
        self.history.grip = Some(Grip { edge, offset: point.x - at });
        true
    }

    /// The pointer moved while an edge is gripped: that edge follows it,
    /// and the other mirrors it about the panel's centre. Returns whether
    /// the window needs drawing.
    pub(in crate::browser) fn history_edge_moved(&mut self, point: Point) -> bool {
        let Some(grip) = self.history.grip else { return false };
        let centre = f64::midpoint(self.body.x0, self.body.x1);
        let at = point.x - grip.offset;
        let half = match grip.edge {
            Edge::Left => centre - at,
            Edge::Right => at - centre,
        };
        self.history.width = 2.0 * half;
        true
    }

    /// The main button let go: the panel keeps the width it was left at,
    /// within the bounds the window gives it.
    pub(in crate::browser) fn history_edge_released(&mut self) {
        if self.history.grip.take().is_some() {
            self.history.width = self.history_width(self.body);
        }
    }

    /// Whether the pointer is over an edge's grip or pulling one.
    pub(in crate::browser) fn over_history_edge(&self) -> bool {
        self.history.open && (self.history.grip.is_some() || self.history.edges.iter().any(|(_, rect)| rect.contains(self.pointer)))
    }
}

impl HistoryUi {
    /// The panel closed: no edge can be held or found until it is drawn again.
    pub(super) fn forget_edges(&mut self) {
        self.grip = None;
        self.edges = [(Edge::Left, Rect::ZERO), (Edge::Right, Rect::ZERO)];
    }
}

/// The widest panel in the window below the strip, `body`: a strip of the
/// pages left showing each side, or the narrowest panel in a window too
/// small for that.
fn widest(body: Rect) -> f64 {
    (body.width() - 2.0 * UNCOVERED).max(MIN_WIDTH)
}
