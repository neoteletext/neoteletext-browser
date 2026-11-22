//! The thirteen first components on teletext (work package U1 of
//! script-api.md; section 8's table of how each kind is built; DEC-7205).
//! Each is drawn with what teletext has, and costs what teletext costs:
//!
//! - **text is the page plane's.** A row of a component begins with the
//!   cell of its colour, a spacing attribute (EN 300 706 clause 12.2), so
//!   a component of `columns` cells shows `columns` − 1 characters, in one
//!   of the eight Level 1 colours. Double height takes a second attribute
//!   cell and the row below;
//! - **a band, a button, a field and every highlight are the overlay's:**
//!   a background over the cells without touching a neighbour, a Colour
//!   Map entry of 0 to 31, paid for from the page's 208 local triplets;
//! - **what the pointer takes is a control** (the engine's decision
//!   E.127): the receiver shows its hover and pressed looks itself. Each
//!   row the pointer can take is one of the page's 32 controls and one
//!   triplet;
//! - **a slider and a progress bar are mosaics,** two steps a cell;
//! - **teletext has one size of character,** so a text that does not fit
//!   is refused or cut, and `fit: "shrink"` cuts.
//!
//! A component is drawn whole into a copy of the page, and the copy is
//! kept only when the page's local enhancement data still fits
//! ([`Teletext::change`]): a component the page has no triplets or
//! controls for is refused by `LOCAL_TRIPLETS` or `MAX_CONTROLS`, and
//! nothing is changed. Erasing a component blanks the cells it wrote,
//! clears the overlay where it set it and removes its controls: the
//! cells of its area it never wrote, where another component may lie,
//! are left alone.

use neoteletext_core::consts::{ALPHA_BLACK, COLOUR_BLACK, COLOUR_WHITE, DOUBLE_HEIGHT, MOSAIC_BIT5, MOSAIC_BLACK, MOSAIC_FULL, SPACE};
use protocol::script::batch::Label;
use protocol::script::teletext::{control_cell, control_handle};

use super::limits::{LAST_COLOUR8, LAST_DOUBLE_HEIGHT_ROW, LAST_ENTRY, MAX_CONTROLS};
use super::model::{Control, OverlayCell, PlaneCell, Rect};
use super::{Teletext, range, type_error};
use crate::host::Reply;
use crate::ui::options::{Align, Area, Fit};
use crate::ui::{Component, Kind, Painter, Shown};

/// The Level 1 colours components use where a script names none (Table
/// 26): yellow for a title and a table's headings, cyan for a link, a
/// hint and a placeholder, blue for what is disabled, green and red for
/// a toggle that is on and off.
const TITLE_INK: u8 = 3;
const LINK_INK: u8 = 6;
const DISABLED_INK: u8 = 4;
const ON_INK: u8 = 2;
const OFF_INK: u8 = 1;
/// The colours of a frame's prompts: the four coloured keys, red, green,
/// yellow and cyan.
const PROMPT_INKS: [u8; 4] = [1, 2, 3, 6];
/// The Colour Map entries of the overlay's backgrounds: blue behind a
/// band, a button, a field and the highlighted item; magenta behind the
/// part that has the focus; and a field's caret, black on white.
const HIGHLIGHT_PAPER: u8 = 4;
const FOCUS_PAPER: u8 = 5;
const CARET_PAPER: u8 = COLOUR_WHITE;
const CARET_INK: u8 = COLOUR_BLACK;
/// The mosaics of a bar (EN 300 706 clause 15.6.1, Table 47): every
/// sixel, the left column of three, and the middle row of two, which is
/// the track and a frame's rule.
const BAR_FULL: u8 = MOSAIC_BIT5 | MOSAIC_FULL | 0x40;
const BAR_HALF: u8 = MOSAIC_BIT5 | 0x15;
const BAR_TRACK: u8 = MOSAIC_BIT5 | 0x0C;
/// The cells a toggle's state takes: its colour's attribute and "OFF".
const TOGGLE_STATE_CELLS: u8 = 4;
/// What a masked field shows for each character.
const MASK: char = '*';

/// One of a component's parts the pointer takes: a control's first cell,
/// its component and which part of it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Part {
    row: u8,
    column: u8,
    component: i32,
    index: usize,
}

/// What a component is drawn as: cells of the plane, backgrounds and a
/// caret on the overlay, and controls.
#[derive(Default)]
struct Drawing {
    cells: Vec<(u8, u8, Vec<PlaneCell>)>,
    overlay: Vec<(Rect, OverlayCell)>,
    controls: Vec<(u8, u8, u8, usize)>,
}

impl Drawing {
    /// A background over `columns` cells of `row` from `column`.
    fn paper(&mut self, row: u8, column: u8, columns: u8, paper: u8) {
        self.overlay.push((Rect { row, column, rows: 1, columns }, OverlayCell { paper: Some(paper), ..OverlayCell::default() }));
    }
}

/// A Level 1 colour a script named, or `default`.
fn ink_of(component: &Component, default: u8) -> Result<u8, Reply> {
    match component.props.ink {
        Some(ink) if i32::from(ink) > LAST_COLOUR8 => Err(range(
            "COLOURS",
            &format!("on teletext a component's ink is a Level 1 colour, 0 to {LAST_COLOUR8}, since it is a spacing attribute; not {ink}"),
        )),
        ink => Ok(ink.unwrap_or(default)),
    }
}

/// The first `count` characters of `text`.
fn cut(text: &str, count: usize) -> String {
    text.chars().take(count).collect()
}

impl Teletext {
    /// The cells of `text` in the component's language, as the engine's
    /// encoder writes them for the page plane.
    fn ui_cells(&self, component: &Component, row: u8, column: u8, text: &str) -> Result<Vec<PlaneCell>, Reply> {
        let language = self.languages.index_of(&component.props.language)?;
        self.languages.plane(row, column, text, language, &[])
    }

    /// One row of a component: the cell of its colour, then `text` in the
    /// `columns` − 1 cells after it, cut to them and laid as `align`
    /// says.
    #[expect(clippy::too_many_arguments, reason = "a row's place, its colour and its text")]
    fn ui_line(&self, drawing: &mut Drawing, component: &Component, row: u8, column: u8, columns: u8, ink: u8, text: &str, align: Align) -> Result<(), Reply> {
        let Some(room) = columns.checked_sub(1).filter(|room| *room > 0) else {
            return Err(range(
                "TELETEXT_ATTRIBUTE_CELL",
                &format!("a row of a component begins with the cell of its colour attribute, so a {} is at least 2 columns wide", component.kind.name()),
            ));
        };
        let mut cells = vec![PlaneCell::code(ALPHA_BLACK | ink)];
        let first = column.saturating_add(1);
        let written = self.ui_cells(component, row, first, &cut(text, usize::from(room)))?;
        let spare = usize::from(room).saturating_sub(written.len());
        let before = match align {
            Align::Left => 0,
            Align::Center => spare.checked_div(2).unwrap_or(0),
            Align::Right => spare,
        };
        cells.extend(core::iter::repeat_n(PlaneCell::BLANK, before));
        cells.extend(written.into_iter().take(usize::from(room)));
        cells.resize(usize::from(columns), PlaneCell::BLANK);
        drawing.cells.push((row, column, cells));
        Ok(())
    }

    /// A bar of mosaics over `columns` cells: the cell of its colour, then
    /// `filled` of `whole` in half cells.
    fn ui_bar(drawing: &mut Drawing, row: u8, column: u8, columns: u8, ink: u8, filled: i64, whole: i64) -> Result<(), Reply> {
        let Some(room) = columns.checked_sub(1).filter(|room| *room > 0) else {
            return Err(range("TELETEXT_ATTRIBUTE_CELL", "a bar begins with the cell of its mosaic colour attribute, so it is at least 2 columns wide"));
        };
        let halves = i64::from(room).saturating_mul(2);
        let lit = filled.saturating_mul(halves).saturating_add(whole.checked_div(2).unwrap_or(0)).checked_div(whole.max(1)).unwrap_or(0).clamp(0, halves);
        let mut cells = vec![PlaneCell::code(MOSAIC_BLACK | ink)];
        for cell in 0..i64::from(room) {
            let code = match lit.saturating_sub(cell.saturating_mul(2)) {
                ..=0 => BAR_TRACK,
                1 => BAR_HALF,
                _ => BAR_FULL,
            };
            cells.push(PlaneCell::code(code));
        }
        drawing.cells.push((row, column, cells));
        Ok(())
    }

    /// What a component is drawn as on teletext.
    #[expect(clippy::too_many_lines, reason = "one arm for each of the thirteen kinds, each a few lines")]
    fn ui_drawing(&self, shown: &Shown<'_>) -> Result<Drawing, Reply> {
        let component = shown.component;
        let props = &component.props;
        let Area { row, column, columns, rows } = component.at;
        let mut drawing = Drawing::default();
        if let Some(paper) = props.paper {
            if i32::from(paper) > LAST_ENTRY {
                return Err(range("COLOUR_MAP_ENTRIES", &format!("a component's paper is a Colour Map entry, 0 to {LAST_ENTRY}, not {paper}")));
            }
            drawing.overlay.push((Rect { row, column, rows, columns }, OverlayCell { paper: Some(paper), ..OverlayCell::default() }));
        }
        let ink = ink_of(component, if props.disabled { DISABLED_INK } else { COLOUR_WHITE })?;
        let focus = |drawing: &mut Drawing| {
            if shown.focused {
                drawing.paper(row, column, columns, FOCUS_PAPER);
            }
        };
        let last_row = row.saturating_add(rows).saturating_sub(1);
        let needs = |needed: u8, what: &str| {
            if rows < needed { Err(range("TELETEXT_ROWS", &format!("{what} takes {needed} rows, and the component has {rows}"))) } else { Ok(()) }
        };
        match component.kind {
            Kind::Frame => {
                let ruled = u8::from(!props.bare);
                let hinted = u8::from(!props.hint.is_empty());
                let prompted = u8::from(!props.prompts.is_empty());
                needs(1_u8.saturating_add(ruled).saturating_add(hinted).saturating_add(prompted), "a frame's title, rule, hint and prompts")?;
                self.ui_line(&mut drawing, component, row, column, columns, ink_of(component, TITLE_INK)?, &props.title, Align::Left)?;
                if !props.bare {
                    Self::ui_bar(&mut drawing, row.saturating_add(1), column, columns, ink_of(component, TITLE_INK)?, 0, 1)?;
                }
                if !props.hint.is_empty() {
                    self.ui_line(&mut drawing, component, last_row.saturating_sub(prompted), column, columns, LINK_INK, &props.hint, Align::Left)?;
                }
                // The prompts share the last row, each under its key's
                // colour.
                let count = u8::try_from(props.prompts.len()).unwrap_or(1).max(1);
                let each = columns.checked_div(count).unwrap_or(columns);
                for ((at, prompt), colour) in (0_u8..).zip(&props.prompts).zip(PROMPT_INKS) {
                    self.ui_line(&mut drawing, component, last_row, column.saturating_add(at.saturating_mul(each)), each, colour, prompt, Align::Left)?;
                }
            }
            Kind::Text => {
                let attributes: u8 = if props.height == 2 { 2 } else { 1 };
                if columns <= attributes {
                    return Err(range(
                        "TELETEXT_ATTRIBUTE_CELL",
                        &format!(
                            "a row of a component begins with the cell of its colour attribute, so a text is at least {} columns wide",
                            attributes.saturating_add(1)
                        ),
                    ));
                }
                let room = usize::from(columns.saturating_sub(attributes));
                let length = props.text.chars().count();
                if props.fit == Fit::Refuse && length > room {
                    return Err(range(
                        "COLUMNS",
                        &format!(
                            "the text is {length} characters and the component shows {room} after its attribute cells; teletext has one size of character"
                        ),
                    ));
                }
                if props.height == 2 {
                    needs(2, "a text of double height")?;
                    if i32::from(row) > LAST_DOUBLE_HEIGHT_ROW {
                        return Err(range(
                            "DOUBLE_HEIGHT_LAST_ROW",
                            &format!("double height takes the row below, so it starts on rows 1 to {LAST_DOUBLE_HEIGHT_ROW}"),
                        ));
                    }
                    // The colour, then Double Height, then the text.
                    self.ui_line(&mut drawing, component, row, column.saturating_add(1), columns.saturating_sub(1), ink, &props.text, props.align)?;
                    if let Some((_, _, cells)) = drawing.cells.last_mut()
                        && let Some(first) = cells.first_mut()
                    {
                        *first = PlaneCell::code(DOUBLE_HEIGHT);
                    }
                    drawing.cells.push((row, column, vec![PlaneCell::code(ALPHA_BLACK | ink)]));
                } else {
                    self.ui_line(&mut drawing, component, row, column, columns, ink, &props.text, props.align)?;
                }
            }
            Kind::Paragraph => {
                let lines = component.lines(self.text_columns(component));
                let page = component.page_lines(lines.len());
                for (line_row, line) in (row..).zip(lines.iter().skip(component.view.top).take(page)) {
                    self.ui_line(&mut drawing, component, line_row, column, columns, ink, line, Align::Left)?;
                }
                // The prompt is shown while more follows, and takes the
                // pointer.
                if component.view.top.saturating_add(page) < lines.len() {
                    self.ui_line(&mut drawing, component, last_row, column, columns, TITLE_INK, &props.more, Align::Right)?;
                    drawing.controls.push((last_row, column, columns, 0));
                }
            }
            Kind::Band => {
                if props.paper.is_none() {
                    drawing.overlay.push((Rect { row, column, rows, columns }, OverlayCell { paper: Some(HIGHLIGHT_PAPER), ..OverlayCell::default() }));
                }
                self.ui_line(&mut drawing, component, row, column, columns, ink, &props.text, Align::Center)?;
            }
            Kind::Link | Kind::Button => {
                let (default, align) = if component.kind == Kind::Link { (LINK_INK, Align::Left) } else { (COLOUR_WHITE, Align::Center) };
                let ink = ink_of(component, if props.disabled { DISABLED_INK } else { default })?;
                self.ui_line(&mut drawing, component, row, column, columns, ink, &props.text, align)?;
                if component.kind == Kind::Button && props.paper.is_none() {
                    drawing.paper(row, column, columns, HIGHLIGHT_PAPER);
                }
                focus(&mut drawing);
                if !props.disabled {
                    drawing.controls.push((row, column, columns, 0));
                }
            }
            Kind::Toggle => {
                let Some(text_columns) = columns.checked_sub(TOGGLE_STATE_CELLS).filter(|left| *left >= 2) else {
                    return Err(range("COLUMNS", "a toggle is at least 6 columns wide: its text and \"OFF\", each after the cell of its colour"));
                };
                self.ui_line(&mut drawing, component, row, column, text_columns, ink, &props.text, Align::Left)?;
                let (state_ink, state) = if props.on { (ON_INK, "ON") } else { (OFF_INK, "OFF") };
                self.ui_line(&mut drawing, component, row, column.saturating_add(text_columns), TOGGLE_STATE_CELLS, state_ink, state, Align::Left)?;
                focus(&mut drawing);
                if !props.disabled {
                    drawing.controls.push((row, column, columns, 0));
                }
            }
            Kind::Menu => {
                for ((line_row, index), item) in (row..).zip(component.view.top..).zip(props.items.iter().skip(component.view.top).take(component.visible())) {
                    let item_ink = if item.disabled || props.disabled { DISABLED_INK } else { ink };
                    self.ui_line(&mut drawing, component, line_row, column, columns, item_ink, &item.text, Align::Left)?;
                    if index == props.selected {
                        drawing.paper(line_row, column, columns, if shown.focused { FOCUS_PAPER } else { HIGHLIGHT_PAPER });
                    }
                    if !item.disabled && !props.disabled {
                        drawing.controls.push((line_row, column, columns, index));
                    }
                }
            }
            Kind::Table => {
                let across = props.columns.iter().fold(1_usize, |sum, one| sum.saturating_add(usize::from(one.width)));
                if across > usize::from(columns) {
                    return Err(range(
                        "COLUMNS",
                        &format!("the table's columns take {across} cells with the cell of the row's colour, and the component has {columns}"),
                    ));
                }
                // A row: the cell of its colour, then each column's text
                // in its own cells.
                let line = |drawing: &mut Drawing, line_row: u8, line_ink: u8, texts: &[String]| -> Result<(), Reply> {
                    let mut cells = vec![PlaneCell::code(ALPHA_BLACK | line_ink)];
                    for (place, one) in props.columns.iter().enumerate() {
                        let width = usize::from(one.width);
                        let text = texts.get(place).map_or("", String::as_str);
                        let written = self.ui_cells(component, line_row, column.saturating_add(1), &cut(text, width))?;
                        let spare = width.saturating_sub(written.len());
                        let before = match one.align {
                            Align::Left => 0,
                            Align::Center => spare.checked_div(2).unwrap_or(0),
                            Align::Right => spare,
                        };
                        cells.extend(core::iter::repeat_n(PlaneCell::BLANK, before));
                        cells.extend(written.into_iter().take(width));
                        cells.extend(core::iter::repeat_n(PlaneCell::BLANK, spare.saturating_sub(before)));
                    }
                    drawing.cells.push((line_row, column, cells));
                    Ok(())
                };
                let titles: Vec<String> = props.columns.iter().map(|one| one.title.clone()).collect();
                line(&mut drawing, row, TITLE_INK, &titles)?;
                for ((line_row, index), texts) in
                    (row.saturating_add(1)..).zip(component.view.top..).zip(props.rows.iter().skip(component.view.top).take(component.visible()))
                {
                    line(&mut drawing, line_row, ink, texts)?;
                    if props.selectable && !props.disabled {
                        if index == props.selected {
                            drawing.paper(line_row, column, columns, if shown.focused { FOCUS_PAPER } else { HIGHLIGHT_PAPER });
                        }
                        drawing.controls.push((line_row, column, columns, index));
                    }
                }
            }
            Kind::Field => {
                let room = usize::from(columns.saturating_sub(1));
                // The cells show the part of the value the caret is in.
                let first = component.view.caret.saturating_add(1).saturating_sub(room);
                let (text, field_ink) = if props.value.is_empty() && !shown.focused {
                    (props.placeholder.clone(), LINK_INK)
                } else {
                    let shown_value: String = props.value.chars().skip(first).take(room).map(|character| if props.masked { MASK } else { character }).collect();
                    (shown_value, ink)
                };
                self.ui_line(&mut drawing, component, row, column, columns, field_ink, &text, Align::Left)?;
                if props.paper.is_none() {
                    drawing.paper(row, column.saturating_add(1), columns.saturating_sub(1), HIGHLIGHT_PAPER);
                }
                if shown.focused {
                    let caret = u8::try_from(component.view.caret.saturating_sub(first)).unwrap_or(0).min(columns.saturating_sub(2));
                    let cell = OverlayCell { paper: Some(CARET_PAPER), ink: Some(CARET_INK), flash: Some(true), ..OverlayCell::default() };
                    drawing.overlay.push((Rect { row, column: column.saturating_add(1).saturating_add(caret), rows: 1, columns: 1 }, cell));
                }
                if !props.disabled {
                    drawing.controls.push((row, column, columns, 0));
                }
            }
            Kind::Slider => {
                Self::ui_bar(&mut drawing, row, column, columns, ink, props.number.saturating_sub(props.min), props.max.saturating_sub(props.min))?;
                focus(&mut drawing);
                if !props.disabled {
                    drawing.controls.push((row, column, columns, 0));
                }
            }
            Kind::Progress => {
                let bar_row = if rows >= 2 {
                    self.ui_line(&mut drawing, component, row, column, columns, ink, &props.text, Align::Left)?;
                    row.saturating_add(1)
                } else {
                    row
                };
                Self::ui_bar(&mut drawing, bar_row, column, columns, ink, props.number, props.max)?;
            }
            Kind::Clock => {
                let (hours, minutes, seconds) = shown.time.parts(props.utc);
                let text = if props.seconds { format!("{hours:02}:{minutes:02}:{seconds:02}") } else { format!("{hours:02}:{minutes:02}") };
                self.ui_line(&mut drawing, component, row, column, columns, ink, &text, Align::Left)?;
            }
        }
        Ok(drawing)
    }
}

impl Painter for Teletext {
    fn text_columns(&self, component: &Component) -> usize {
        usize::from(component.at.columns.saturating_sub(1)).max(1)
    }

    fn paint(&mut self, before: Option<&Component>, now: Option<&Shown<'_>>) -> Result<(), Reply> {
        let drawing = now.map(|shown| self.ui_drawing(shown)).transpose()?.unwrap_or_default();
        let gone = before.map(|component| component.handle);
        // What the component before wrote, and what this one writes.
        let erased: Vec<Rect> = self.ui_drawn.iter().filter(|(component, _)| Some(*component) == gone).map(|(_, rect)| *rect).collect();
        let mut drawn: Vec<(i32, Rect)> = self.ui_drawn.iter().copied().filter(|(component, _)| Some(*component) != gone).collect();
        if let Some(shown) = now {
            let handle = shown.component.handle;
            let spans = drawing.cells.iter().map(|(row, column, cells)| Rect {
                row: *row,
                column: *column,
                rows: 1,
                columns: u8::try_from(cells.len()).unwrap_or(u8::MAX),
            });
            drawn.extend(spans.chain(drawing.overlay.iter().map(|(rect, _)| *rect)).map(|rect| (handle, rect)));
        }
        // The parts as they will be: the others', and the new drawing's.
        let mut parts: Vec<Part> = self.ui_parts.iter().copied().filter(|part| Some(part.component) != gone).collect();
        let removed: Vec<(u8, u8)> = self.ui_parts.iter().filter(|part| Some(part.component) == gone).map(|part| (part.row, part.column)).collect();
        if let Some(shown) = now {
            parts.extend(drawing.controls.iter().map(|(row, column, _, part)| Part {
                row: *row,
                column: *column,
                component: shown.component.handle,
                index: *part,
            }));
        }
        self.change(|model| {
            model.controls.retain(|control| !removed.contains(&(control.row, control.column)));
            for rect in &erased {
                for line in rect.row..rect.row.saturating_add(rect.rows) {
                    model.write(line, rect.column, core::iter::repeat_n(PlaneCell::BLANK, usize::from(rect.columns)));
                }
                model.overlay_each(*rect, |_, cell| *cell = OverlayCell::default());
            }
            for (row, column, cells) in drawing.cells {
                model.write(row, column, cells.into_iter());
            }
            for (rect, look) in &drawing.overlay {
                model.overlay_each(*rect, |_, cell| {
                    cell.ink = look.ink.or(cell.ink);
                    cell.paper = look.paper.or(cell.paper);
                    cell.flash = look.flash.or(cell.flash);
                });
            }
            for (row, column, width, _) in &drawing.controls {
                if model.control_over(*row, *column, column.saturating_add(*width)) {
                    return Err(type_error("a control already holds one of the cells the component's control needs"));
                }
                if model.controls.len() >= MAX_CONTROLS {
                    return Err(range(
                        "MAX_CONTROLS",
                        &format!("a page has at most {MAX_CONTROLS} controls, and each row of a component the pointer takes is one"),
                    ));
                }
                model.controls.push(Control { row: *row, column: *column, width: *width, height: 1, look: 0, scope: 0 });
            }
            Ok(())
        })?;
        // A component's label is its first control's, for a screen
        // reader, which is told of controls.
        let stale: Vec<i32> = removed.iter().filter_map(|(row, column)| control_handle(*row, *column)).collect();
        self.labels.retain(|staged| !stale.contains(&staged.handle));
        if let Some(shown) = now.filter(|shown| !shown.component.props.label.is_empty())
            && let Some(handle) = drawing_first_control(&parts, shown.component.handle)
        {
            self.labels.retain(|staged| staged.handle != handle);
            self.labels.push(Label { handle, label: shown.component.props.label.clone() });
        }
        self.ui_parts = parts;
        self.ui_drawn = drawn;
        Ok(())
    }

    fn part(&self, handle: i32) -> Option<(i32, usize)> {
        let (row, column) = control_cell(handle)?;
        self.ui_parts.iter().find(|part| part.row == row && part.column == column).map(|part| (part.component, part.index))
    }
}

/// The handle of the first control of `component` among `parts`.
fn drawing_first_control(parts: &[Part], component: i32) -> Option<i32> {
    parts.iter().find(|part| part.component == component).and_then(|part| control_handle(part.row, part.column))
}

/// A blank cell's code, which the plane is erased to.
const _: () = assert!(PlaneCell::BLANK.code == SPACE);
