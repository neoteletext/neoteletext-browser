//! A tab pressed and dragged: chosen at once, moved along the strip when
//! let go there, or out of the window when let go off it
//! (`multiple-windows.md`), and where a tab from another window would go.

use super::{DRAG_THRESHOLD, Drag, STRIP_HEIGHT, TEAR_DISTANCE, whole};
use crate::browser::{Browser, Output};
use crate::geometry::{Point, Rect};

impl Browser {
    /// A press on the strip's tabs, its count or the overflow panel;
    /// returns whether it was there. A tab is chosen at once and may then
    /// be dragged.
    pub(in crate::browser) fn tabs_press(&mut self, point: Point) -> bool {
        if self.tab_strip.count.contains(point) {
            self.tab_strip.pinned = !self.tab_strip.pinned;
            self.tab_strip.open = self.tab_strip.pinned || self.tab_strip.open;
            return true;
        }
        let Some(region) = self.regions.tabs.iter().rev().find(|region| region.tab.contains(point)).copied() else {
            return self.tab_strip.panel.contains(point);
        };
        if region.close.contains(point) {
            self.outputs.push(Output::CloseTab(region.index));
        } else if region.speaker.is_some_and(|speaker| speaker.contains(point)) {
            self.outputs.push(Output::MuteTab(region.index));
        } else {
            self.outputs.push(Output::SelectTab(region.index));
            self.tab_strip.drag = Some(Drag { index: region.index, start: point, moving: false, target: region.index });
        }
        true
    }

    /// The pointer moved: the panel opens or goes, and a pressed tab is
    /// dragged. Returns whether anything changed.
    pub(in crate::browser) fn tabs_moved(&mut self, point: Point) -> bool {
        let strip = Rect::new(0.0, 0.0, self.regions.strip.x1, STRIP_HEIGHT);
        let on = strip.contains(point) || self.tab_strip.panel.contains(point);
        let open = self.tab_strip.overflowing && (on || self.tab_strip.drag.is_some_and(|drag| drag.moving));
        let mut changed = open != self.tab_strip.open;
        self.tab_strip.open = open;
        if !on {
            self.tab_strip.pinned = false;
        }
        if let Some(mut drag) = self.tab_strip.drag {
            if !drag.moving && libm::hypot(point.x - drag.start.x, point.y - drag.start.y) > DRAG_THRESHOLD {
                drag.moving = true;
            }
            if drag.moving {
                if let Some(region) = self.regions.tabs.iter().rev().find(|region| region.tab.contains(point)) {
                    drag.target = region.index;
                }
                changed = true;
            }
            self.tab_strip.drag = Some(drag);
        }
        changed
    }

    /// The button was released: a dragged tab goes where it was let go.
    pub(in crate::browser) fn tabs_released(&mut self) {
        let Some(drag) = self.tab_strip.drag.take() else { return };
        // Let go off the strip and the panel, below them or outside the
        // window: the tab leaves this window (`multiple-windows.md`).
        let point = self.pointer;
        let strip = Rect::new(0.0, 0.0, self.regions.strip.x1, STRIP_HEIGHT + TEAR_DISTANCE);
        if drag.moving && !strip.contains(point) && !self.tab_strip.panel.contains(point) {
            self.outputs.push(Output::TabOut { index: drag.index, x: whole(point.x), y: whole(point.y) });
            return;
        }
        if drag.moving && drag.target != drag.index {
            self.outputs.push(Output::MoveTab { from: drag.index, to: drag.target });
        }
    }

    /// Where a tab let go at `point` of this window, in logical pixels,
    /// would go on its strip: before the tab there, or at the end; none
    /// off the strip.
    #[must_use]
    pub fn tab_place_at(&self, point: Point) -> Option<usize> {
        if !self.regions.strip.contains(point) {
            return None;
        }
        let shown = self.regions.tabs.iter().find(|region| region.tab.contains(point)).map(|region| region.index);
        Some(shown.unwrap_or(self.tabs.len()))
    }

    /// Escape while dragging: the tab stays where it was.
    pub(in crate::browser) fn tabs_cancel(&mut self) -> bool {
        self.tab_strip.drag.take().is_some_and(|drag| drag.moving)
    }
}
