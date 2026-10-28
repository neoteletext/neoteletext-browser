//! The keys of the lists at the left: on the wall of pages, a click on the
//! channel's magazines or on the page list gives that list the keys, and
//! Up and Down then move through it, even on a page whose channel takes
//! the keys, until a click anywhere else or Escape gives them back. With
//! the page alone, a magazine or a page clicked opens a page, which has
//! the keys (decision 126).

use crate::browser::{Browser, Key};
use crate::canvas::Canvas;
use crate::channel::BANKS;
use crate::controls::PageControl;
use crate::geometry::Rect;
use crate::text::border;
use crate::theme::LINK;

/// The ring round the list that has the keys.
const FOCUS_RING: f64 = 2.0;

/// The list that has the keys.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ListFocus {
    /// The channel's magazines: Up and Down choose the one before or after.
    Magazines,
    /// The chosen magazine's pages: Up and Down choose the page before or
    /// after, and Enter opens it.
    Pages,
    /// The open page's sub-pages: Up and Down show the sub-page before or
    /// after.
    SubPages,
}

impl Browser {
    /// A key while a list has the keys: `None` when no list has them, or
    /// the key is not the list's.
    pub(super) fn list_key(&mut self, key: Key<'_>, now: u64) -> Option<bool> {
        let focus = self.list_focus?;
        match (focus, key) {
            (_, Key::Escape) => self.list_focus = None,
            (ListFocus::Magazines, Key::Up | Key::Down) => {
                let next = if key == Key::Down { self.bank.saturating_add(1).min(BANKS - 1) } else { self.bank.saturating_sub(1) };
                if next != self.bank {
                    self.choose_bank(next);
                }
            }
            (ListFocus::Pages, Key::Up | Key::Down) => self.step_list(key == Key::Down),
            (ListFocus::SubPages, Key::Up | Key::Down) => {
                if self.controls_open() && self.page_controls.controls.subpages > 1 {
                    self.ask_control(PageControl::SubPage(key == Key::Down));
                }
            }
            (ListFocus::Pages, Key::Enter) => {
                let key = self.selected.and_then(|index| self.page_key(index))?;
                self.wall.zoom(key, now);
                self.list_focus = None;
            }
            _ => return None,
        }
        Some(true)
    }

    /// A ring round `panel` when `focus` is the list that has the keys.
    pub(super) fn draw_list_focus<C: Canvas + ?Sized>(&self, canvas: &mut C, panel: Rect, focus: ListFocus) {
        if self.list_focus == Some(focus) {
            border(canvas, panel, FOCUS_RING, LINK);
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::browser::fixture::{browser, middle, press_at};
    use crate::browser::{Key, Modifiers};

    #[test]
    fn a_clicked_list_takes_up_and_down_until_a_click_elsewhere() {
        let (mut browser, mut canvas) = browser();
        let second = browser.regions.banks[1];
        press_at(&mut browser, &mut canvas, second);
        assert_eq!(browser.bank, 1);
        assert!(browser.key(Key::Down, Modifiers::default(), 0));
        assert_eq!(browser.bank, 2, "the magazines take Down");
        browser.key(Key::Up, Modifiers::default(), 0);
        browser.key(Key::Up, Modifiers::default(), 0);
        browser.key(Key::Up, Modifiers::default(), 0);
        assert_eq!(browser.bank, 0, "and stop at the first");
        browser.draw(&mut canvas, crate::browser::fixture::WIDTH, crate::browser::fixture::HEIGHT, 0);
        let row = browser.regions.rows[0].0;
        press_at(&mut browser, &mut canvas, row);
        let first = browser.selected;
        browser.key(Key::Down, Modifiers::default(), 0);
        assert_ne!(browser.selected, first, "the page list takes Down");
        assert_eq!(browser.bank, 0, "and the magazine stays");
        let point = middle(browser.regions.cells);
        browser.pointer_moved(point.x, point.y, 0);
        browser.pointer_pressed(&mut canvas, false, 0);
        assert_eq!(browser.list_focus, None, "a click elsewhere takes the keys from the list");
    }
}
