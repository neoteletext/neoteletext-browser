//! Hyperlinks in the browser window (`hyperlinks.md` sections 9, 10 and
//! 12): the underline over the open page, the pointer and the keyboard's
//! focus on links, typed numbers that a page or channel links elsewhere,
//! and the trusted strip's destination and question. Everything a link
//! does is the trusted core's to decide; this only reports where the user
//! is and what they did.

use alloc::collections::BTreeMap;
use alloc::format;
use alloc::vec::Vec;

use super::{Browser, Cursor, Key, Modifiers, Output};
use crate::canvas::{Canvas, PageKey};
use crate::channel::page_index;
use crate::geometry::Rect;
use crate::links::{self, LinkCell, LinkKind, PageLinks, Pointing, Prompt, PromptButton, PromptKind};
use crate::theme::{ADDRESS, BUTTON, FIELD_INK, INSECURE, LINK, LINK_INK, ROW_STATUS, ROW_TITLE, RULE, WHITE};
use crate::ui::{border, centred, left_centred};
use crate::wall::{CellClick, cell_in};

/// The strip's questions take no answer in their first half second, as the
/// trusted core also requires (`hyperlinks.md` section 12).
pub const PROMPT_DELAY_MILLISECONDS: u64 = 500;
/// The question's two buttons, inside the address field at its right.
const PROMPT_BUTTON_WIDTH: f64 = 84.0;
const PROMPT_BUTTON_GAP: f64 = 6.0;
const PROMPT_BUTTON_INSET: f64 = 5.0;
const FOCUS_RING: f64 = 2.0;
/// Space between the field's edge and its text.
const FIELD_PADDING: f64 = 12.0;

/// What the window knows of links.
#[derive(Default)]
pub(super) struct LinkUi {
    /// The links of each page's latest image, as the trusted core sent them.
    pages: BTreeMap<PageKey, PageLinks>,
    /// The page numbers the active channel's descriptor links elsewhere.
    aliases: (u32, Vec<u16>),
    /// What the strip shows of the link under the pointer or the focus.
    pointing: Option<Pointing>,
    /// The strip's question, and when it was asked.
    prompt: Option<(Prompt, u64)>,
    /// The question's button the keyboard is on, once the user moved there.
    prompt_focus: Option<PromptButton>,
    /// The open page's link the keyboard is on.
    focus: Option<usize>,
    /// The open page's link under the pointer.
    hovered: Option<usize>,
    /// What the trusted core was last told the pointer or focus is on.
    sent: Option<LinkCell>,
    /// A page the trusted core opened that the window does not list yet.
    pending: Option<PageKey>,
    /// The question's buttons, where last drawn.
    go: Rect,
    cancel: Rect,
}

impl Browser {
    /// The links of a page's latest image, as the trusted core decided them.
    pub fn set_links(&mut self, key: PageKey, links: PageLinks) {
        self.links.pages.insert(key, links);
        self.links.pages.retain(|key, _| key.channel == self.channel.channel);
        self.repoint();
    }

    /// What the trusted strip shows of the link the pointer or the focus is
    /// on, as the trusted core wrote it.
    pub fn set_pointing(&mut self, pointing: Option<Pointing>) {
        self.links.pointing = pointing;
    }

    /// The question the trusted strip asks, or none. It never takes the
    /// keyboard's focus by itself (requirement 17.11).
    pub fn set_prompt(&mut self, prompt: Option<Prompt>, now: u64) {
        self.links.prompt = prompt.map(|prompt| (prompt, now));
        self.links.prompt_focus = None;
    }

    /// The page numbers the active channel's descriptor links elsewhere.
    pub fn set_aliases(&mut self, channel: u32, numbers: Vec<u16>) {
        self.links.aliases = (channel, numbers);
    }

    /// The trusted core followed a link to `key`: the page opens, zoomed. A
    /// page not listed yet opens when it is.
    pub fn show(&mut self, key: PageKey, now: u64) {
        if key.channel != self.channel.channel {
            return;
        }
        let listed = page_index(key.page).filter(|&index| matches!(self.channel.pages.get(index), Some(Some(_))));
        let Some(index) = listed else {
            self.links.pending = Some(key);
            return;
        };
        self.links.pending = None;
        self.select_page(index, true);
        if self.wall.zoomed().is_none() {
            self.wall.zoom(key, now);
        }
    }

    /// A page arrived: the page the trusted core opened, if it waited for it.
    pub(super) fn show_pending(&mut self, channel: u32, page: u16, now: u64) {
        if let Some(key) = self.links.pending.filter(|key| key.channel == channel && key.page == page) {
            self.show(key, now);
        }
    }

    /// The open page's links, when they belong to its image on screen.
    fn open_links(&self) -> Option<(PageKey, Rect, &PageLinks)> {
        let (key, rect) = self.wall.open()?;
        let links = self.links.pages.get(&key)?;
        (self.versions.get(&key) == Some(&links.version)).then_some((key, rect, links))
    }

    /// The open page's link under the pointer, and the cell.
    fn link_under_pointer(&self) -> Option<(usize, LinkCell)> {
        let (key, rect, links) = self.open_links()?;
        let (column, row) = cell_in(rect, self.pointer)?;
        let index = links.run_at(column, row)?;
        Some((index, LinkCell { key, version: links.version, column, row }))
    }

    /// The cell the keyboard's focus is on: the focused link's first.
    fn focused_cell(&self) -> Option<LinkCell> {
        let (key, _, links) = self.open_links()?;
        let run = links.runs.get(self.links.focus?)?;
        Some(LinkCell { key, version: links.version, column: run.column, row: run.row })
    }

    /// Tells the trusted core where the pointer or the focus is, when that
    /// changed, or when the image under it changed, so the strip always
    /// shows the destination of the image on screen. Returns whether the
    /// window should be drawn again.
    pub(super) fn repoint(&mut self) -> bool {
        let under = self.link_under_pointer();
        if under.is_some() {
            // The pointer on a link takes over from the keyboard.
            self.links.focus = None;
        }
        let hovered = under.map(|(index, _)| index);
        let changed = hovered != self.links.hovered;
        self.links.hovered = hovered;
        let cell = under.map(|(_, cell)| cell).or_else(|| self.focused_cell());
        let same_link = match (cell, self.links.sent) {
            (Some(now), Some(before)) => now.key == before.key && now.version == before.version && self.same_run(now, before),
            (None, None) => true,
            _ => false,
        };
        if !same_link {
            self.links.sent = cell;
            self.outputs.push(Output::Point(cell));
        }
        changed || !same_link
    }

    /// Whether two cells of the open page are in the same link.
    fn same_run(&self, one: LinkCell, other: LinkCell) -> bool {
        self.open_links().is_some_and(|(_, _, links)| links.run_at(one.column, one.row) == links.run_at(other.column, other.row))
    }

    /// The pointer's shape over a link: a hand for a page of this channel,
    /// the shape of a shortcut for one that leaves.
    pub(super) fn link_cursor(&self) -> Option<Cursor> {
        let (_, _, links) = self.open_links()?;
        let run = links.runs.get(self.links.hovered?)?;
        Some(if run.kind.leaves() { Cursor::External } else { Cursor::Hand })
    }

    /// A press on the strip's question. Returns whether it was on it.
    pub(super) fn link_press(&mut self, now: u64) -> bool {
        let Some((prompt, asked)) = &self.links.prompt else { return false };
        let id = prompt.id;
        if self.links.cancel.contains(self.pointer) {
            self.outputs.push(Output::Cancel(id));
            return true;
        }
        if self.links.go.contains(self.pointer) {
            // Not in the question's first half second (`hyperlinks.md`
            // section 12).
            if now.saturating_sub(*asked) >= PROMPT_DELAY_MILLISECONDS {
                self.outputs.push(Output::Confirm(id));
            }
            return true;
        }
        // The rest of the field holds the question, not the address.
        self.regions.field.contains(self.pointer)
    }

    /// The keys links use, when they apply: Tab and Shift-Tab move the focus
    /// between the open page's links, or between the question's buttons;
    /// Enter follows the focused link or presses the focused button; Escape
    /// declines the question or leaves the links. Returns whether the
    /// window should be drawn again, or `None` for a key links leave alone.
    pub(super) fn link_key(&mut self, key: Key<'_>, modifiers: Modifiers, now: u64) -> Option<bool> {
        if let Some((prompt, asked)) = &self.links.prompt {
            let (id, asked) = (prompt.id, *asked);
            return match key {
                Key::Tab => {
                    self.links.prompt_focus = Some(match self.links.prompt_focus {
                        Some(PromptButton::Go) => PromptButton::Cancel,
                        _ => PromptButton::Go,
                    });
                    Some(true)
                }
                // Enter answers only once the user moved the focus to a
                // button, never a key held from before, never in the first
                // half second (requirement 17.11).
                Key::Enter => {
                    match self.links.prompt_focus {
                        Some(PromptButton::Go) if !modifiers.repeat && now.saturating_sub(asked) >= PROMPT_DELAY_MILLISECONDS => {
                            self.outputs.push(Output::Confirm(id));
                        }
                        Some(PromptButton::Cancel) if !modifiers.repeat => self.outputs.push(Output::Cancel(id)),
                        _ => {}
                    }
                    Some(true)
                }
                Key::Escape => {
                    self.outputs.push(Output::Cancel(id));
                    Some(true)
                }
                _ => None,
            };
        }
        match key {
            Key::Tab => {
                let count = self.open_links()?.2.runs.len();
                if count == 0 {
                    return None;
                }
                let next = match (self.links.focus, modifiers.shift) {
                    (None, false) => 0,
                    (None, true) => count.saturating_sub(1),
                    (Some(at), false) => at.saturating_add(1) % count,
                    (Some(at), true) => at.checked_sub(1).unwrap_or(count.saturating_sub(1)),
                };
                self.links.focus = Some(next);
                self.repoint();
                Some(true)
            }
            Key::Enter if !modifiers.repeat => {
                let cell = self.focused_cell()?;
                self.outputs.push(Output::Cell(CellClick { key: cell.key, version: cell.version, column: cell.column, row: cell.row }));
                Some(true)
            }
            Key::Escape if self.links.focus.is_some() => {
                self.links.focus = None;
                self.repoint();
                Some(true)
            }
            _ => None,
        }
    }

    /// A page number typed: when the open page or the channel links it
    /// elsewhere, the trusted core decides where it leads. Returns whether
    /// it went to the trusted core.
    pub(super) fn link_typed(&mut self, number: u16) -> bool {
        let open = self.open_links().map(|(key, _, links)| (key, links.version, links.remapped.contains(&number)));
        let aliased = self.links.aliases.0 == self.channel.channel && self.links.aliases.1.contains(&number);
        let on_page = open.is_some_and(|(_, _, remapped)| remapped);
        if !on_page && !aliased {
            return false;
        }
        self.outputs.push(Output::Typed { number, opened: open.map(|(key, version, _)| (key, version)) });
        true
    }

    /// Draws the open page's links (`hyperlinks.md` section 9.2).
    pub(super) fn draw_links<C: Canvas + ?Sized>(&mut self, canvas: &mut C) {
        let Some((_, rect, links)) = self.open_links() else { return };
        let runs = links.runs.clone();
        canvas.push_clip(self.wall.area());
        links::draw(canvas, rect, &runs, self.links.hovered, self.links.focus);
        canvas.pop_clip();
    }

    /// Draws the strip's question in the address field, in place of the
    /// address, the padlock and the star. Returns whether there is one.
    pub(super) fn draw_prompt<C: Canvas + ?Sized>(&mut self, canvas: &mut C, field: Rect) -> bool {
        let Some((prompt, _)) = &self.links.prompt else {
            self.links.go = Rect::ZERO;
            self.links.cancel = Rect::ZERO;
            return false;
        };
        let (question, answer) = match prompt.kind {
            PromptKind::Leave => (format!("Leave this channel for {}?", prompt.destination), "Go"),
            PromptKind::Web => (format!("Open {} in your web browser?", prompt.destination), "Open"),
        };
        let top = field.y0 + PROMPT_BUTTON_INSET;
        let bottom = field.y1 - PROMPT_BUTTON_INSET;
        let cancel = Rect::new(field.x1 - PROMPT_BUTTON_INSET - PROMPT_BUTTON_WIDTH, top, field.x1 - PROMPT_BUTTON_INSET, bottom);
        let go = Rect::new(cancel.x0 - PROMPT_BUTTON_GAP - PROMPT_BUTTON_WIDTH, top, cancel.x0 - PROMPT_BUTTON_GAP, bottom);
        let mut left = field.x0 + FIELD_PADDING;
        if !prompt.secure {
            let warning = "Not secure";
            left_centred(canvas, Rect::new(left, field.y0, go.x0, field.y1), 0.0, warning, ROW_STATUS, INSECURE);
            left += canvas.text_width(warning, ROW_STATUS) + FIELD_PADDING;
        }
        canvas.push_clip(Rect::new(left, field.y0, go.x0 - PROMPT_BUTTON_GAP, field.y1));
        left_centred(canvas, Rect::new(left, field.y0, go.x0, field.y1), 0.0, &question, ADDRESS, LINK_INK);
        canvas.pop_clip();
        for (rect, label, button) in [(go, answer, PromptButton::Go), (cancel, "Cancel", PromptButton::Cancel)] {
            canvas.fill(rect, 0.0, BUTTON);
            centred(canvas, rect, label, ROW_TITLE, WHITE);
            let colour = if self.links.prompt_focus == Some(button) { LINK } else { RULE };
            border(canvas, rect, FOCUS_RING, colour);
        }
        self.links.go = go;
        self.links.cancel = cancel;
        true
    }

    /// Draws the destination of the link under the pointer or the focus in
    /// the address field, in place of the address. Returns whether there is
    /// one.
    pub(super) fn draw_pointing<C: Canvas + ?Sized>(&mut self, canvas: &mut C, area: Rect) -> bool {
        let Some(pointing) = &self.links.pointing else { return false };
        let arrow = match pointing.kind {
            LinkKind::Page => "\u{2192} ",
            LinkKind::Channel | LinkKind::Web => "\u{2197} ",
        };
        canvas.push_clip(area);
        left_centred(canvas, area, 0.0, &format!("{arrow}{}", pointing.text), ADDRESS, if pointing.kind.leaves() { FIELD_INK } else { LINK_INK });
        canvas.pop_clip();
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::browser::{Frame, StripState, TabInfo};
    use crate::channel::{Channel, PAGES, PageInfo, Status};
    use crate::geometry::Point;
    use crate::links::LinkRun;
    use crate::test_canvas::Recorder;
    use alloc::string::String;
    use alloc::vec;

    const WIDTH: f64 = 1440.0;
    const HEIGHT: f64 = 900.0;
    /// After the zoom's 300 milliseconds: the page is open.
    const OPEN: u64 = 1_000;
    const VERSION: u32 = 5;

    fn channel() -> Channel {
        let mut channel = Channel::empty();
        channel.channel = 7;
        for index in (0..PAGES).filter(|index| index % 3 != 2) {
            channel.pages[index] = Some(PageInfo { title: format!("Page {index}"), status: Status::Loaded });
        }
        channel
    }

    fn run(row: u8, column: u8, length: u8, kind: LinkKind) -> LinkRun {
        LinkRun { row, column, length, double: false, light: false, kind }
    }

    fn page_links(version: u32) -> PageLinks {
        PageLinks { version, runs: vec![run(5, 10, 3, LinkKind::Page), run(7, 0, 12, LinkKind::Web)], remapped: vec![0x302] }
    }

    /// A browser with its first page zoomed and open, its image at `VERSION`.
    fn opened() -> (Browser, Recorder, PageKey, Rect) {
        let mut browser = Browser::new(Frame { left_inset: 0.0, window_buttons: true });
        browser.set_tabs(vec![TabInfo { title: "one".into(), colour: 0, channel: 7, muted: false, private: false }], 0);
        browser.set_strip(StripState { address: "https://news.invalid/".into(), identity: "news.invalid".into(), encrypted: true, ..StripState::default() });
        browser.set_channel(channel());
        let mut canvas = Recorder::new();
        browser.draw(&mut canvas, WIDTH, HEIGHT, 0);
        let (key, _) = browser.wall().shown().next().unwrap();
        browser.wall_mut().zoom(key, 0);
        browser.draw(&mut canvas, WIDTH, HEIGHT, OPEN);
        browser.frame_version(key, VERSION);
        let (open, rect) = browser.wall().open().expect("the page is open");
        assert_eq!(open, key);
        browser.take_outputs();
        (browser, canvas, key, rect)
    }

    fn centre(rect: Rect, column: u8, row: u8) -> Point {
        let (width, height) = (rect.width() / 40.0, rect.height() / 25.0);
        Point::new(rect.x0 + (f64::from(column) + 0.5) * width, rect.y0 + (f64::from(row) + 0.5) * height)
    }

    fn move_to(browser: &mut Browser, point: Point) {
        browser.pointer_moved(point.x, point.y, OPEN);
    }

    fn points(browser: &mut Browser) -> Vec<Option<LinkCell>> {
        browser.take_outputs().into_iter().filter_map(|output| if let Output::Point(cell) = output { Some(cell) } else { None }).collect()
    }

    #[test]
    fn the_pointer_on_a_link_is_reported_once_per_link_with_its_shape() {
        let (mut browser, _, key, rect) = opened();
        browser.set_links(key, page_links(VERSION));
        browser.take_outputs();
        move_to(&mut browser, centre(rect, 11, 5));
        assert_eq!(points(&mut browser), [Some(LinkCell { key, version: VERSION, column: 11, row: 5 })]);
        assert_eq!(browser.cursor(), Cursor::Hand);
        move_to(&mut browser, centre(rect, 12, 5));
        assert_eq!(points(&mut browser), [], "the same link");
        move_to(&mut browser, centre(rect, 3, 7));
        assert_eq!(points(&mut browser).len(), 1);
        assert_eq!(browser.cursor(), Cursor::External, "a link that leaves");
        move_to(&mut browser, centre(rect, 30, 12));
        assert_eq!(points(&mut browser), [None]);
        assert_eq!(browser.cursor(), Cursor::Arrow);
    }

    #[test]
    fn a_new_image_under_the_pointer_is_reported_again() {
        // The trusted core keeps a few images' links; the strip must follow
        // the image on screen, which a flashing page renews many times a
        // second.
        let (mut browser, _, key, rect) = opened();
        browser.set_links(key, page_links(VERSION));
        move_to(&mut browser, centre(rect, 11, 5));
        browser.take_outputs();
        browser.frame_version(key, VERSION + 1);
        assert_eq!(points(&mut browser), [None], "the links of the new image have not come yet");
        browser.set_links(key, page_links(VERSION + 1));
        assert_eq!(points(&mut browser), [Some(LinkCell { key, version: VERSION + 1, column: 11, row: 5 })]);
    }

    #[test]
    fn links_are_drawn_only_over_their_own_image() {
        let (mut browser, mut canvas, key, _) = opened();
        let blue = |canvas: &Recorder| canvas.fills.iter().filter(|(_, colour)| *colour == LINK).count();
        browser.set_links(key, page_links(VERSION - 1));
        canvas.fills.clear();
        browser.draw(&mut canvas, WIDTH, HEIGHT, OPEN);
        assert_eq!(blue(&canvas), 0, "links of an older image are not drawn");
        browser.set_links(key, page_links(VERSION));
        canvas.fills.clear();
        browser.draw(&mut canvas, WIDTH, HEIGHT, OPEN);
        assert_eq!(blue(&canvas), 2);
    }

    #[test]
    fn a_click_names_the_image_it_was_made_on() {
        let (mut browser, mut canvas, key, rect) = opened();
        browser.set_links(key, page_links(VERSION));
        move_to(&mut browser, centre(rect, 11, 5));
        browser.pointer_pressed(&mut canvas, false, OPEN);
        let clicks: Vec<CellClick> =
            browser.take_outputs().into_iter().filter_map(|output| if let Output::Cell(click) = output { Some(click) } else { None }).collect();
        assert_eq!(clicks, [CellClick { key, version: VERSION, column: 11, row: 5 }]);
    }

    #[test]
    fn tab_moves_between_links_and_enter_follows_the_focused_one() {
        let (mut browser, _, key, _) = opened();
        browser.set_links(key, page_links(VERSION));
        browser.take_outputs();
        let plain = Modifiers::default();
        let first = Some(LinkCell { key, version: VERSION, column: 10, row: 5 });
        let second = Some(LinkCell { key, version: VERSION, column: 0, row: 7 });
        assert!(browser.key(Key::Tab, plain, OPEN));
        assert_eq!(points(&mut browser), [first]);
        browser.key(Key::Tab, plain, OPEN);
        assert_eq!(points(&mut browser), [second]);
        browser.key(Key::Tab, Modifiers { shift: true, ..plain }, OPEN);
        assert_eq!(points(&mut browser), [first]);
        browser.key(Key::Enter, Modifiers { repeat: true, ..plain }, OPEN);
        assert!(browser.take_outputs().is_empty(), "a key held from before follows nothing");
        browser.key(Key::Enter, plain, OPEN);
        assert_eq!(browser.take_outputs(), [Output::Cell(CellClick { key, version: VERSION, column: 10, row: 5 })]);
        assert!(browser.key(Key::Escape, plain, OPEN));
        assert_eq!(points(&mut browser), [None]);
        assert!(browser.wall().zoomed().is_some(), "Escape left the links, not the page");
    }

    fn prompt(id: u32) -> Prompt {
        Prompt { id, kind: PromptKind::Web, destination: "https://www.example.invalid/".into(), secure: true }
    }

    #[test]
    fn the_question_waits_half_a_second_and_never_takes_the_focus() {
        let (mut browser, mut canvas, _, _) = opened();
        browser.set_prompt(Some(prompt(3)), OPEN);
        browser.draw(&mut canvas, WIDTH, HEIGHT, OPEN);
        assert!(canvas.texts.iter().any(|(_, text)| text.starts_with("Open https://www.example.invalid/")));
        let go = browser.links.go;
        browser.pointer_moved(f64::midpoint(go.x0, go.x1), f64::midpoint(go.y0, go.y1), OPEN);
        browser.pointer_pressed(&mut canvas, false, OPEN + PROMPT_DELAY_MILLISECONDS - 1);
        assert!(browser.take_outputs().iter().all(|output| !matches!(output, Output::Confirm(_))), "too soon");
        browser.pointer_pressed(&mut canvas, false, OPEN + PROMPT_DELAY_MILLISECONDS);
        assert!(browser.take_outputs().contains(&Output::Confirm(3)));
        // Enter answers only once the focus was moved to a button.
        let later = OPEN + PROMPT_DELAY_MILLISECONDS;
        browser.key(Key::Enter, Modifiers::default(), later);
        assert!(browser.take_outputs().is_empty(), "the question does not take the focus by itself");
        browser.key(Key::Tab, Modifiers::default(), later);
        browser.key(Key::Enter, Modifiers { repeat: true, ..Modifiers::default() }, later);
        assert!(browser.take_outputs().is_empty(), "a key held from before");
        browser.key(Key::Enter, Modifiers::default(), later);
        assert_eq!(browser.take_outputs(), [Output::Confirm(3)]);
        browser.key(Key::Escape, Modifiers::default(), later);
        assert_eq!(browser.take_outputs(), [Output::Cancel(3)]);
    }

    #[test]
    fn the_strip_shows_where_a_link_leads_in_place_of_the_address() {
        let (mut browser, mut canvas, _, _) = opened();
        browser.set_pointing(Some(Pointing { kind: LinkKind::Page, text: "Page 302".into() }));
        canvas.texts.clear();
        browser.draw(&mut canvas, WIDTH, HEIGHT, OPEN);
        let texts: Vec<&String> = canvas.texts.iter().map(|(_, text)| text).collect();
        assert!(texts.iter().any(|text| text.as_str() == "\u{2192} Page 302"));
        assert!(!texts.iter().any(|text| text.contains("news.invalid")), "the address gives way");
    }

    #[test]
    fn a_number_the_page_or_channel_links_goes_to_the_trusted_core() {
        let (mut browser, _, key, _) = opened();
        browser.set_links(key, page_links(VERSION));
        browser.set_aliases(7, vec![0x500]);
        browser.take_outputs();
        browser.key(Key::Text("302"), Modifiers::default(), OPEN);
        assert_eq!(browser.take_outputs(), [Output::Typed { number: 0x302, opened: Some((key, VERSION)) }]);
        browser.key(Key::Text("500"), Modifiers::default(), OPEN);
        assert_eq!(browser.take_outputs(), [Output::Typed { number: 0x500, opened: Some((key, VERSION)) }]);
        browser.key(Key::Text("401"), Modifiers::default(), OPEN);
        assert!(!browser.take_outputs().iter().any(|output| matches!(output, Output::Typed { .. })), "an ordinary number chooses its page");
    }

    #[test]
    fn a_page_the_trusted_core_opens_is_zoomed_now_or_when_it_arrives() {
        let (mut browser, _, _, _) = opened();
        let listed = PageKey { channel: 7, page: 0x301 };
        browser.show(listed, OPEN);
        assert_eq!(browser.wall().zoomed(), Some(listed));
        // Page 303 is not listed yet (every third page is missing).
        let missing = PageKey { channel: 7, page: 0x303 };
        browser.show(missing, OPEN);
        assert_eq!(browser.wall().zoomed(), Some(listed));
        browser.add_page(7, 0x303, "Arrived".into(), OPEN);
        assert_eq!(browser.wall().zoomed(), Some(missing));
    }
}
