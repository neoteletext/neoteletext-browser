//! A tab pressed and dragged: chosen at once, moved along the strip when
//! let go there, or out of the window when let go off it
//! (`multiple-windows.md`), and where a tab from another window would go.

use super::{DRAG_THRESHOLD, Drag, STRIP_HEIGHT, TEAR_DISTANCE, whole};
use crate::browser::{Browser, Output};
use crate::geometry::{Point, Rect};

impl Browser {
    /// A press on the strip's tabs; returns whether it was there. A tab is
    /// chosen at once and may then be dragged. The count is the
    /// filmstrip's (`filmstrip_press`).
    pub(in crate::browser) fn tabs_press(&mut self, point: Point) -> bool {
        let Some(region) = self.regions.tabs.iter().rev().find(|region| region.tab.contains(point)).copied() else { return false };
        // This press is taken before the strip's, which ends editing.
        self.stop_editing();
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

    /// The pointer moved at `now`: the filmstrip's waits start and end
    /// (`filmstrip_moved`), and a pressed tab is dragged. Returns whether
    /// anything changed.
    pub(in crate::browser) fn tabs_moved(&mut self, point: Point, now: u64) -> bool {
        let mut changed = self.filmstrip_moved(point, now);
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
        // Let go off the strip, below it or outside the window: the tab
        // leaves this window (`multiple-windows.md`).
        let point = self.pointer;
        let strip = Rect::new(0.0, 0.0, self.regions.strip.x1, STRIP_HEIGHT + TEAR_DISTANCE);
        if drag.moving && !strip.contains(point) {
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
