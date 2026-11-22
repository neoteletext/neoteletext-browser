//! The thirteen first components on NAPLPS (work package U3 of
//! script-api.md; section 8's table of how each kind is built; DEC-7205:
//! each renderer's components are made for that renderer, and agree with
//! the others' only where that makes sense; this package's own choices are
//! DEC-7404). Each is drawn with what
//! NAPLPS has, and costs what NAPLPS costs:
//!
//! - **what stays still is painted on the backdrop,** on the cell grid
//!   (12 by 20 dots a cell), in any of the colour map's sixteen entries
//!   for ink and for paper. There is no attribute cell: a component of
//!   `columns` cells shows `columns` characters. A change paints the
//!   cells over, which costs stream bytes against the second's
//!   allowance and nothing else;
//! - **text has more than one size.** A `text` of `height` 2 is double
//!   height, and `fit: "shrink"` narrows the character field until the
//!   text fits, where teletext could only cut;
//! - **rules, bars and switches are drawn,** to the dot: a frame's rule
//!   and a link's underline are lines, a button has an outline, a
//!   progress bar and a slider's track are exact to one dot of 480
//!   where teletext has half a cell of 40;
//! - **what the pointer takes is a region** of the cast (section 7.5),
//!   one for each part: a button, a link, a toggle, a field, a slider,
//!   each row of a menu or a selectable table that is shown, a
//!   paragraph's prompt. Its label is what a screen reader says;
//! - **what moves is an actor,** moved by an `ANIMATE` operation with
//!   nothing painted again: a menu's or a table's highlight (an outline
//!   over the chosen row), a slider's thumb, a field's caret (two frames
//!   the receiver cycles), and the focus ring, one for the page, whose
//!   two frames the receiver cycles so that the focus is seen to pulse
//!   with the script not running.
//!
//! **What a component costs against the page's bounds.** Of the page's 56
//! objects (`MAX_OBJECTS` less the channel context's share): one for
//! each part the pointer takes; one more for a menu or a selectable
//! table (its highlight), for a slider (its thumb) and for a focused
//! field (its caret); and one for the page's focus ring. Of its 80
//! symbols (`DEFINABLE_CODES` less the channel context's share): one or
//! two for each different size of highlight, ring, caret and thumb,
//! shared by every component that uses that size. Of the 16 objects
//! that may cycle (`MAX_CYCLING_OBJECTS`): the ring and the caret. A
//! component the page has no object or symbol for is refused by that
//! bound's name and nothing is changed.
//!
//! A component is drawn by a painter program the host writes itself and
//! gives to the same painter a script's programs go to
//! ([`super::writer::Painter`]), so components and the script's own
//! painting cannot put the stream's state out of step. The host keeps
//! each component's program: a new generation, which paints from
//! nothing, runs them again after the script's backdrop (section 7.2).

use protocol::script::ContextId;

use super::cast::{Cast, Defined, UiObject};
use super::limits::{CELL_DOTS, CELL_LINES, FIRST_TEXT_ROW, NAPLPS_COLOURS};
use super::program::Paint;
use super::writer::{self, Painter as StreamPainter};
use super::{Naplps, range, type_error};
use crate::host::Reply;
use crate::limits::MAX_BATCH_BYTES;
use crate::ui::options::{Align, Area, Fit};
use crate::ui::{Component, Kind, Painter, Shown};

/// The colour-map entries components use where a script names none:
/// teletext's eight colours, which every generation begins with
/// (section 7.1). Yellow for a title, headings and the highlight; cyan
/// for a link, a hint and a placeholder; blue behind a band, a button
/// and a field, and for what is disabled; green and red for a toggle on
/// and off; white for text and the focus ring.
const BLACK: u8 = 0;
const RED: u8 = 1;
const GREEN: u8 = 2;
const YELLOW: u8 = 3;
const BLUE: u8 = 4;
const CYAN: u8 = 6;
const WHITE: u8 = 7;
/// The colours of a frame's prompts: the four coloured keys.
const PROMPT_INKS: [u8; 4] = [RED, GREEN, YELLOW, CYAN];
/// What a masked field shows for each character.
const MASK: char = '*';
/// The lines of a rule, an underline and a thick outline.
const RULE_LINES: i32 = 2;
/// A caret's width in dots, and the ticks each of its frames is shown.
const CARET_DOTS: i32 = 2;
const CARET_TICKS: u16 = 30;
/// The ticks each of the focus ring's frames is shown.
const RING_TICKS: u16 = 20;
/// A slider's thumb, in dots and lines.
const THUMB_DOTS: i32 = 6;
const THUMB_LINES: i32 = 16;
/// A bar's height in lines, within its row of twenty.
const BAR_LINES: i32 = 12;
/// A toggle's switch: its width in cells, and its knob's in dots.
const SWITCH_CELLS: i32 = 3;
const KNOB_DOTS: i32 = 14;
/// The narrowest a shrunk character field may be, in dots: below it a
/// 12-dot glyph is no longer read.
const NARROWEST_FIELD: i32 = 5;
/// The layers of a component's objects (animation.md ANIM-12): its
/// regions and moving parts in the scene, the focus ring above them.
const SCENE_LAYER: u8 = 1;
const RING_LAYER: u8 = 2;
/// A text size of the painter's `text` call: normal, and double height.
const SIZE_NORMAL: i32 = 0;
const SIZE_DOUBLE_HEIGHT: i32 = 1;

/// A rectangle of the canvas, in dots.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct Rect {
    x: i32,
    y: i32,
    width: i32,
    height: i32,
}

/// What an object of a component is for.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Role {
    /// A part the pointer takes: which.
    Part(usize),
    /// A menu's or a table's highlight.
    Highlight,
    /// A slider's thumb.
    Thumb,
    /// A field's caret.
    Caret,
}

/// The shapes components' symbols draw.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Shape {
    /// An outline one line thick, in the highlight's colour.
    Outline,
    /// The focus ring's two frames: one line thick, and two.
    Ring,
    RingThick,
    /// A filled rectangle, and nothing: a caret's two frames, a thumb.
    Solid,
    Empty,
}

/// A symbol the components share: its shape and size, its slot, and how
/// many objects draw with it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct Shared {
    shape: Shape,
    width: i32,
    height: i32,
    slot: usize,
    users: usize,
}

/// An object a component owns.
#[derive(Clone, PartialEq, Eq, Debug)]
struct Owned {
    component: i32,
    role: Role,
    number: u16,
    /// What it was last made as.
    object: UiObject,
    /// The shapes of its frames, to give their symbols back.
    shapes: Vec<Shape>,
}

/// One component as it was last drawn: its program, kept for the next
/// generation, and the rectangles it painted, to paint over when it
/// changes or goes.
#[derive(Clone, PartialEq, Eq, Debug)]
struct Drawn {
    component: i32,
    language: String,
    program: Vec<i32>,
    rects: Vec<Rect>,
}

/// What the host keeps for the page's components on NAPLPS.
#[derive(Clone, Debug, Default)]
pub struct Components {
    drawn: Vec<Drawn>,
    owned: Vec<Owned>,
    shared: Vec<Shared>,
    /// The focus ring: its object's number, the component it is on, and
    /// where.
    ring: Option<(u16, i32, Rect)>,
}

impl Components {
    /// The bytes held.
    pub(super) fn state_bytes(&self) -> usize {
        let programs = self.drawn.iter().fold(0_usize, |sum, drawn| {
            sum.saturating_add(drawn.program.len().saturating_mul(size_of::<i32>()))
                .saturating_add(drawn.rects.len().saturating_mul(size_of::<Rect>()))
                .saturating_add(drawn.language.len())
                .saturating_add(size_of::<Drawn>())
        });
        programs.saturating_add(self.owned.len().saturating_mul(size_of::<Owned>())).saturating_add(self.shared.len().saturating_mul(size_of::<Shared>()))
    }

    /// Every component's program, in the order they were made, for a new
    /// generation to run again.
    pub(super) fn programs(&self) -> impl Iterator<Item = (&[i32], &str)> {
        self.drawn.iter().map(|drawn| (drawn.program.as_slice(), drawn.language.as_str()))
    }
}

/// A painter program, written call by call (section 10.2).
#[derive(Default)]
struct Program {
    words: Vec<i32>,
    rects: Vec<Rect>,
}

impl Program {
    fn call(&mut self, call: Paint, ints: &[i32]) {
        self.words.push(call.number());
        self.words.extend_from_slice(ints);
    }

    fn string(&mut self, text: &str) {
        self.words.push(i32::try_from(text.chars().count()).unwrap_or(0));
        self.words.extend(text.chars().map(|character| i32::try_from(u32::from(character)).unwrap_or(0)));
    }

    /// Fills `rect` with `entry`, and notes it as painted.
    fn fill(&mut self, rect: Rect, entry: u8) {
        self.call(Paint::INK, &[i32::from(entry)]);
        self.call(Paint::FILL_RECT, &[rect.x, rect.y, rect.width, rect.height]);
        self.rects.push(rect);
    }

    /// A line `lines` thick from (`x`, `y`) across `width` dots.
    fn rule(&mut self, x: i32, y: i32, width: i32, lines: i32, entry: u8) {
        self.fill(Rect { x, y, width, height: lines }, entry);
    }

    /// The outline of `rect`.
    fn outline(&mut self, rect: Rect, entry: u8) {
        self.call(Paint::INK, &[i32::from(entry)]);
        self.call(Paint::RECT, &[rect.x, rect.y, rect.width, rect.height]);
    }

    /// Text on the cell grid from the cell (`row`, `column`).
    fn text(&mut self, row: i32, column: i32, size: i32, entry: u8, language: &str, text: &str) {
        if text.is_empty() {
            return;
        }
        self.call(Paint::INK, &[i32::from(entry)]);
        self.call(Paint::TEXT, &[row, column, size]);
        self.string(language);
        self.string(text);
    }
}

/// The dots of the cells from (`row`, `column`), `columns` across and
/// `rows` down.
fn cells(row: i32, column: i32, columns: i32, rows: i32) -> Rect {
    Rect {
        x: column.saturating_mul(CELL_DOTS),
        y: row.saturating_sub(FIRST_TEXT_ROW).saturating_mul(CELL_LINES),
        width: columns.saturating_mul(CELL_DOTS),
        height: rows.saturating_mul(CELL_LINES),
    }
}

/// A colour-map entry a script named, or `default`.
fn entry_of(named: Option<u8>, default: u8) -> Result<u8, Reply> {
    match named {
        Some(entry) if i32::from(entry) >= NAPLPS_COLOURS => {
            Err(range("NAPLPS_COLOURS", &format!("on NAPLPS a component's ink and paper are colour-map entries, 0 to 15, not {entry}")))
        }
        entry => Ok(entry.unwrap_or(default)),
    }
}

/// The cells `text` takes in `language`, and the text cut to `room` of them.
fn fitted(text: &str, language: &str, room: usize) -> Result<(String, usize), Reply> {
    let count = |text: &str| writer::runs(text, language).map(|runs| usize::try_from(writer::cells(&runs)).unwrap_or(usize::MAX));
    let whole = count(text)?;
    if whole <= room {
        return Ok((text.to_owned(), whole));
    }
    // A character is at most one cell, so the first `room` characters
    // fit; a letter and its mark are one cell and may leave room over.
    let cut: String = text.chars().take(room).collect();
    let taken = count(&cut)?;
    Ok((cut, taken))
}

/// What a component is drawn as on NAPLPS: a painter program for the
/// backdrop, and the objects of the cast.
struct Drawing {
    program: Program,
    objects: Vec<(Role, UiObject, Vec<Shape>)>,
    /// Where the focus ring lies when the component has the focus.
    focus: Option<Rect>,
}

/// A row of a component: the cells painted in `paper`, then `text` in
/// `ink`, cut to the cells and laid as `align` says.
#[expect(clippy::too_many_arguments, reason = "a row's place, its colours and its text")]
fn line(program: &mut Program, language: &str, row: i32, column: i32, columns: i32, ink: u8, paper: u8, text: &str, align: Align) -> Result<(), Reply> {
    program.fill(cells(row, column, columns, 1), paper);
    let room = usize::try_from(columns).unwrap_or(0);
    let (text, taken) = fitted(text, language, room)?;
    let spare = i32::try_from(room.saturating_sub(taken)).unwrap_or(0);
    let before = match align {
        Align::Left => 0,
        Align::Center => spare.checked_div(2).unwrap_or(0),
        Align::Right => spare,
    };
    program.text(row, column.saturating_add(before), SIZE_NORMAL, ink, language, &text);
    Ok(())
}

/// A region over `rect`, which takes the pointer for a part.
fn region(rect: Rect, label: &str) -> UiObject {
    UiObject {
        region: true,
        x: rect.x,
        y: rect.y,
        width: rect.width,
        height: rect.height,
        layer: SCENE_LAYER,
        frames: Vec::new(),
        detectable: true,
        label: label.to_owned(),
        cycle: None,
    }
}

/// An actor over `rect`, whose frames the caller fills in.
fn actor(rect: Rect, layer: u8, cycle: Option<u16>) -> UiObject {
    UiObject {
        region: false,
        x: rect.x,
        y: rect.y,
        width: rect.width,
        height: rect.height,
        layer,
        frames: Vec::new(),
        detectable: false,
        label: String::new(),
        cycle,
    }
}

/// What a component is drawn as.
#[expect(clippy::too_many_lines, reason = "one arm for each of the thirteen kinds, each a few lines")]
fn drawing(shown: &Shown<'_>, text_columns: usize) -> Result<Drawing, Reply> {
    let component = shown.component;
    let props = &component.props;
    let Area { row, column, columns, rows } = component.at;
    let (row, column, columns, rows) = (i32::from(row), i32::from(column), i32::from(columns), i32::from(rows));
    let language = props.language.as_str();
    let mut program = Program::default();
    let mut objects: Vec<(Role, UiObject, Vec<Shape>)> = Vec::new();
    let ink = entry_of(props.ink, if props.disabled { BLUE } else { WHITE })?;
    let paper = entry_of(props.paper, BLACK)?;
    let whole = cells(row, column, columns, rows);
    let first_row = cells(row, column, columns, 1);
    let last_row = row.saturating_add(rows).saturating_sub(1);
    let needs = |needed: i32, what: &str| {
        if rows < needed { Err(range("ROWS", &format!("{what} takes {needed} rows, and the component has {rows}"))) } else { Ok(()) }
    };
    let mut focus = None;
    match component.kind {
        Kind::Frame => {
            let hinted = i32::from(!props.hint.is_empty());
            let prompted = i32::from(!props.prompts.is_empty());
            needs(1_i32.saturating_add(hinted).saturating_add(prompted), "a frame's title, hint and prompts")?;
            let title_ink = entry_of(props.ink, YELLOW)?;
            line(&mut program, language, row, column, columns, title_ink, paper, &props.title, Align::Left)?;
            if !props.bare {
                // The rule is drawn on the title row's last lines: a
                // line needs no row of its own.
                program.rule(first_row.x, first_row.y.saturating_add(CELL_LINES).saturating_sub(RULE_LINES), first_row.width, RULE_LINES, title_ink);
            }
            if !props.hint.is_empty() {
                line(&mut program, language, last_row.saturating_sub(prompted), column, columns, CYAN, paper, &props.hint, Align::Left)?;
            }
            // The prompts share the last row, each after a square of
            // its key's colour.
            let count = i32::try_from(props.prompts.len()).unwrap_or(1).max(1);
            let each = columns.checked_div(count).unwrap_or(columns);
            for ((place, prompt), colour) in (0_i32..).zip(&props.prompts).zip(PROMPT_INKS) {
                let from = column.saturating_add(place.saturating_mul(each));
                if each < 2 {
                    return Err(range("COLUMNS", "each of a frame's prompts takes at least 2 columns: its key's square and its text"));
                }
                line(&mut program, language, last_row, from.saturating_add(1), each.saturating_sub(1), ink, paper, prompt, Align::Left)?;
                let key = cells(last_row, from, 1, 1);
                program.fill(key, paper);
                program.fill(
                    Rect { x: key.x.saturating_add(2), y: key.y.saturating_add(4), width: CELL_DOTS.saturating_sub(4), height: CELL_LINES.saturating_sub(8) },
                    colour,
                );
            }
        }
        Kind::Text => {
            let tall = props.height == 2;
            if tall {
                needs(2, "a text of double height")?;
            }
            let room = usize::try_from(columns).unwrap_or(0);
            let taken = usize::try_from(writer::cells(&writer::runs(&props.text, language)?)).unwrap_or(usize::MAX);
            program.fill(cells(row, column, columns, if tall { 2 } else { 1 }), paper);
            if taken <= room || props.fit == Fit::Cut {
                let (text, taken) = fitted(&props.text, language, room)?;
                let spare = i32::try_from(room.saturating_sub(taken)).unwrap_or(0);
                let before = match props.align {
                    Align::Left => 0,
                    Align::Center => spare.checked_div(2).unwrap_or(0),
                    Align::Right => spare,
                };
                program.text(row, column.saturating_add(before), if tall { SIZE_DOUBLE_HEIGHT } else { SIZE_NORMAL }, ink, language, &text);
            } else if props.fit == Fit::Refuse {
                return Err(range("COLUMNS", &format!("the text takes {taken} cells and the component has {room}; `fit: \"shrink\"` draws it narrower")));
            } else {
                // NAPLPS draws a character in any field: the text is
                // drawn narrower until it fits, and cut only when a
                // character would be too narrow to read.
                let narrow = first_row.width.checked_div(i32::try_from(taken).unwrap_or(i32::MAX).max(1)).unwrap_or(0).max(NARROWEST_FIELD);
                let fits = usize::try_from(first_row.width.checked_div(narrow).unwrap_or(0)).unwrap_or(0);
                let (text, _) = fitted(&props.text, language, fits)?;
                let height = if tall { CELL_LINES.saturating_mul(2) } else { CELL_LINES };
                program.call(Paint::INK, &[i32::from(ink)]);
                program.call(Paint::TEXT_AT, &[first_row.x, first_row.y, narrow, height, 0, 0, 0, 0, 0, 0]);
                program.string(language);
                program.string(&text);
            }
        }
        Kind::Paragraph => {
            let lines = component.lines(text_columns);
            let page = component.page_lines(lines.len());
            program.fill(whole, paper);
            focus = Some(whole);
            for (line_row, text) in (row..).zip(lines.iter().skip(component.view.top).take(page)) {
                let (text, _) = fitted(text, language, usize::try_from(columns).unwrap_or(0))?;
                program.text(line_row, column, SIZE_NORMAL, ink, language, &text);
            }
            // The prompt is shown while more follows, and takes the pointer.
            if component.view.top.saturating_add(page) < lines.len() {
                line(&mut program, language, last_row, column, columns, YELLOW, paper, &props.more, Align::Right)?;
                objects.push((Role::Part(0), region(cells(last_row, column, columns, 1), &props.label), Vec::new()));
            }
        }
        Kind::Band => {
            let paper = entry_of(props.paper, BLUE)?;
            program.fill(whole, paper);
            let middle = row.saturating_add(rows.saturating_sub(1).checked_div(2).unwrap_or(0));
            let (text, taken) = fitted(&props.text, language, usize::try_from(columns).unwrap_or(0))?;
            let before = columns.saturating_sub(i32::try_from(taken).unwrap_or(0)).checked_div(2).unwrap_or(0);
            program.text(middle, column.saturating_add(before), SIZE_NORMAL, ink, language, &text);
        }
        Kind::Link => {
            let ink = entry_of(props.ink, if props.disabled { BLUE } else { CYAN })?;
            line(&mut program, language, row, column, columns, ink, paper, &props.text, Align::Left)?;
            // The underline, as long as the text.
            let (_, taken) = fitted(&props.text, language, usize::try_from(columns).unwrap_or(0))?;
            let under = i32::try_from(taken).unwrap_or(0).saturating_mul(CELL_DOTS);
            if under > 0 {
                program.rule(first_row.x, first_row.y.saturating_add(CELL_LINES).saturating_sub(1), under, 1, ink);
            }
            focus = Some(first_row);
            if !props.disabled {
                objects.push((Role::Part(0), region(first_row, &props.label), Vec::new()));
            }
        }
        Kind::Button => {
            let paper = entry_of(props.paper, BLUE)?;
            line(&mut program, language, row, column, columns, ink, paper, &props.text, Align::Center)?;
            program.outline(first_row, ink);
            focus = Some(first_row);
            if !props.disabled {
                objects.push((Role::Part(0), region(first_row, &props.label), Vec::new()));
            }
        }
        Kind::Toggle => {
            let Some(text_cells) = columns.checked_sub(SWITCH_CELLS).filter(|left| *left >= 1) else {
                return Err(range("COLUMNS", "a toggle is at least 4 columns wide: its text and its switch of 3"));
            };
            line(&mut program, language, row, column, text_cells, ink, paper, &props.text, Align::Left)?;
            // The switch: a track, and a knob at its right in green when
            // on, at its left in red when off.
            let track = cells(row, column.saturating_add(text_cells), SWITCH_CELLS, 1);
            program.fill(track, paper);
            let inner =
                Rect { x: track.x.saturating_add(2), y: track.y.saturating_add(4), width: track.width.saturating_sub(4), height: CELL_LINES.saturating_sub(8) };
            program.outline(inner, ink);
            let knob_x = if props.on { inner.x.saturating_add(inner.width).saturating_sub(KNOB_DOTS).saturating_sub(2) } else { inner.x.saturating_add(2) };
            program.fill(
                Rect { x: knob_x, y: inner.y.saturating_add(2), width: KNOB_DOTS, height: inner.height.saturating_sub(4) },
                if props.on { GREEN } else { RED },
            );
            focus = Some(first_row);
            if !props.disabled {
                objects.push((Role::Part(0), region(first_row, &props.label), Vec::new()));
            }
        }
        Kind::Menu | Kind::Table => {
            let table = component.kind == Kind::Table;
            program.fill(whole, paper);
            let mut body_row = row;
            if table {
                let across = props.columns.iter().fold(0_i32, |sum, one| sum.saturating_add(i32::from(one.width)));
                if across > columns {
                    return Err(range("COLUMNS", &format!("the table's columns take {across} cells, and the component has {columns}")));
                }
                // The headings, with a rule on their row's last lines.
                let mut from = column;
                for one in &props.columns {
                    line(&mut program, language, row, from, i32::from(one.width), YELLOW, paper, &one.title, one.align)?;
                    from = from.saturating_add(i32::from(one.width));
                }
                program.rule(first_row.x, first_row.y.saturating_add(CELL_LINES).saturating_sub(1), first_row.width, 1, YELLOW);
                body_row = row.saturating_add(1);
            }
            let count = component.count();
            let takes = !props.disabled && (!table || props.selectable);
            for (line_row, index) in (body_row..).zip(component.view.top..count).take(component.visible()) {
                let enabled = takes && (table || props.items.get(index).is_some_and(|item| !item.disabled));
                if table {
                    let mut from = column;
                    for (place, one) in props.columns.iter().enumerate() {
                        let text = props.rows.get(index).and_then(|texts| texts.get(place)).map_or("", String::as_str);
                        line(&mut program, language, line_row, from, i32::from(one.width), ink, paper, text, one.align)?;
                        from = from.saturating_add(i32::from(one.width));
                    }
                } else if let Some(item) = props.items.get(index) {
                    line(&mut program, language, line_row, column, columns, if enabled { ink } else { BLUE }, paper, &item.text, Align::Left)?;
                }
                let over = cells(line_row, column, columns, 1);
                if enabled {
                    objects.push((Role::Part(index), region(over, if objects.is_empty() { &props.label } else { "" }), Vec::new()));
                }
                if takes && index == props.selected {
                    // The highlight: an actor over the chosen row, moved
                    // when the choice moves.
                    objects.push((Role::Highlight, actor(over, SCENE_LAYER, None), vec![Shape::Outline]));
                    focus = Some(over);
                }
            }
        }
        Kind::Field => {
            let paper = entry_of(props.paper, BLUE)?;
            let room = usize::try_from(columns).unwrap_or(1).max(1);
            // The cells show the part of the value the caret is in.
            let first = component.view.caret.saturating_add(1).saturating_sub(room);
            let (text, field_ink) = if props.value.is_empty() && !shown.focused {
                (props.placeholder.clone(), CYAN)
            } else {
                (props.value.chars().skip(first).take(room).map(|character| if props.masked { MASK } else { character }).collect(), ink)
            };
            line(&mut program, language, row, column, columns, field_ink, paper, &text, Align::Left)?;
            focus = Some(first_row);
            if !props.disabled {
                objects.push((Role::Part(0), region(first_row, &props.label), Vec::new()));
            }
            if shown.focused {
                let place = i32::try_from(component.view.caret.saturating_sub(first)).unwrap_or(0).min(columns.saturating_sub(1));
                let caret = Rect {
                    x: first_row.x.saturating_add(place.saturating_mul(CELL_DOTS)),
                    y: first_row.y.saturating_add(2),
                    width: CARET_DOTS,
                    height: CELL_LINES.saturating_sub(4),
                };
                objects.push((Role::Caret, actor(caret, SCENE_LAYER, Some(CARET_TICKS)), vec![Shape::Solid, Shape::Empty]));
            }
        }
        Kind::Slider => {
            if first_row.width < THUMB_DOTS.saturating_mul(2) {
                return Err(range("COLUMNS", "a slider is at least 1 column wide"));
            }
            program.fill(first_row, paper);
            let middle = first_row.y.saturating_add(CELL_LINES.checked_div(2).unwrap_or(0)).saturating_sub(1);
            program.rule(first_row.x, middle, first_row.width, RULE_LINES, ink);
            // The thumb: an actor moved along the track, exact to a dot.
            let span = i64::from(first_row.width.saturating_sub(THUMB_DOTS));
            let range_of = props.max.saturating_sub(props.min).max(1);
            let along = props.number.saturating_sub(props.min).clamp(0, range_of).saturating_mul(span).checked_div(range_of).unwrap_or(0);
            let thumb = Rect {
                x: first_row.x.saturating_add(i32::try_from(along).unwrap_or(0)),
                y: first_row.y.saturating_add(CELL_LINES.saturating_sub(THUMB_LINES).checked_div(2).unwrap_or(0)),
                width: THUMB_DOTS,
                height: THUMB_LINES,
            };
            objects.push((Role::Thumb, actor(thumb, SCENE_LAYER, None), vec![Shape::Solid]));
            focus = Some(first_row);
            if !props.disabled {
                objects.push((Role::Part(0), region(first_row, &props.label), Vec::new()));
            }
        }
        Kind::Progress => {
            let bar_row = if rows >= 2 {
                line(&mut program, language, row, column, columns, ink, paper, &props.text, Align::Left)?;
                row.saturating_add(1)
            } else {
                row
            };
            let over = cells(bar_row, column, columns, 1);
            program.fill(over, paper);
            let bar = Rect {
                x: over.x,
                y: over.y.saturating_add(CELL_LINES.saturating_sub(BAR_LINES).checked_div(2).unwrap_or(0)),
                width: over.width,
                height: BAR_LINES,
            };
            program.outline(bar, ink);
            // Filled to the dot: `value` of `max` of the bar's inside.
            let inside = i64::from(bar.width.saturating_sub(4));
            let most = props.max.max(1);
            let filled = props.number.clamp(0, most).saturating_mul(inside).checked_div(most).unwrap_or(0);
            if filled > 0 {
                program.fill(
                    Rect {
                        x: bar.x.saturating_add(2),
                        y: bar.y.saturating_add(2),
                        width: i32::try_from(filled).unwrap_or(0),
                        height: BAR_LINES.saturating_sub(4),
                    },
                    ink,
                );
            }
        }
        Kind::Clock => {
            let (hours, minutes, seconds) = shown.time.parts(props.utc);
            let text = if props.seconds { format!("{hours:02}:{minutes:02}:{seconds:02}") } else { format!("{hours:02}:{minutes:02}") };
            line(&mut program, language, row, column, columns, ink, paper, &text, Align::Left)?;
        }
    }
    if !shown.focused {
        focus = None;
    }
    Ok(Drawing { program, objects, focus })
}

/// The painter program of a shape of `width` dots by `height` lines.
fn shape_program(shape: Shape, width: i32, height: i32) -> Vec<i32> {
    let mut program = Program::default();
    match shape {
        Shape::Outline => program.outline(Rect { x: 0, y: 0, width, height }, YELLOW),
        Shape::Ring => program.outline(Rect { x: 0, y: 0, width, height }, WHITE),
        Shape::RingThick => {
            program.call(Paint::PEN, &[RULE_LINES, RULE_LINES]);
            program.outline(Rect { x: 0, y: 0, width: width.saturating_sub(1).max(1), height: height.saturating_sub(1).max(1) }, WHITE);
        }
        Shape::Solid => program.fill(Rect { x: 0, y: 0, width, height }, WHITE),
        Shape::Empty => {}
    }
    program.words
}

impl Naplps {
    /// The page's drawing of components, for the page context of a
    /// NAPLPS page with a script of its own: what stays still is painted
    /// on the backdrop, which is the page context's alone (section 3.1).
    pub(crate) fn ui_painter(&mut self, context: ContextId) -> Option<&mut dyn Painter> {
        if self.active && context == ContextId::Page { Some(self) } else { None }
    }

    /// Whether this is the renderer that draws `context`'s components.
    pub(crate) const fn draws_components(&self) -> bool {
        self.active
    }

    /// Runs `program` on the live painter and appends what it writes:
    /// bytes the host owes, held to the second's allowance and the
    /// step's output.
    fn ui_run(&mut self, program: &[i32], language: &str) -> Result<(), Reply> {
        if program.is_empty() {
            return Ok(());
        }
        let mut live = self.begin();
        let (_, second) = self.rooms();
        let room = second.min(MAX_BATCH_BYTES.saturating_sub(self.staged_bytes()));
        let mut out = Vec::new();
        let language = if language.is_empty() { self.language.clone() } else { language.to_owned() };
        live.run(program, &language, &Defined(&self.cast, ContextId::Page), &mut out, room, "NAPLPS_MAX_BYTES_PER_SECOND")?;
        self.append_owed(live, &out);
        Ok(())
    }

    /// The slot of the shared symbol of `shape` at a size, defined if no
    /// component has used it yet.
    fn ui_shared(&mut self, shape: Shape, width: i32, height: i32) -> Result<usize, Reply> {
        if let Some(shared) = self.components.shared.iter_mut().find(|shared| (shared.shape, shared.width, shared.height) == (shape, width, height)) {
            shared.users = shared.users.saturating_add(1);
            return Ok(shared.slot);
        }
        let mut body = Vec::new();
        let mut painter = StreamPainter::symbol(height);
        painter.run(&shape_program(shape, width, height), "", &writer::NoDefinitions, &mut body, super::limits::MAX_MACRO_BYTES, "MAX_MACRO_BYTES")?;
        painter.close(&mut body);
        let slot = self.cast.ui_symbol(width, height, body)?;
        let mut live = self.begin();
        let mut out = Vec::new();
        live.end(&mut out);
        self.cast.symbol_definition(slot, &mut out);
        self.append_owed(live, &out);
        self.components.shared.push(Shared { shape, width, height, slot, users: 1 });
        Ok(slot)
    }

    /// Gives back the shared symbols of `shapes` at a size; one that no
    /// object draws with any more is freed.
    fn ui_release(&mut self, shapes: &[Shape], width: i32, height: i32) {
        for shape in shapes {
            if let Some(shared) = self.components.shared.iter_mut().find(|shared| (shared.shape, shared.width, shared.height) == (*shape, width, height)) {
                shared.users = shared.users.saturating_sub(1);
            }
        }
        let free: Vec<usize> = self.components.shared.iter().filter(|shared| shared.users == 0).map(|shared| shared.slot).collect();
        self.components.shared.retain(|shared| shared.users > 0);
        for slot in free {
            self.cast.ui_free_symbol(slot);
        }
    }

    /// Makes the object `wanted` with frames of `shapes`, or changes the
    /// one numbered `existing` to it; its number.
    fn ui_make(&mut self, existing: Option<u16>, mut wanted: UiObject, shapes: &[Shape]) -> Result<u16, Reply> {
        for shape in shapes {
            let slot = self.ui_shared(*shape, wanted.width, wanted.height)?;
            wanted.frames.push(slot);
        }
        self.cast.ui_object(existing, wanted)
    }

    /// Puts the focus ring on `rect` for `component`, or takes it away.
    fn ui_ring(&mut self, component: i32, rect: Option<Rect>) -> Result<(), Reply> {
        let shapes = [Shape::Ring, Shape::RingThick];
        match (self.components.ring, rect) {
            (Some((number, _, at)), Some(rect)) if (at.width, at.height) == (rect.width, rect.height) => {
                if (at.x, at.y) != (rect.x, rect.y) {
                    self.cast.ui_move(number, rect.x, rect.y)?;
                }
                self.components.ring = Some((number, component, rect));
            }
            (ring, Some(rect)) => {
                let number = self.ui_make(ring.map(|(number, _, _)| number), actor(rect, RING_LAYER, Some(RING_TICKS)), &shapes)?;
                if let Some((_, _, at)) = ring {
                    self.ui_release(&shapes, at.width, at.height);
                }
                self.components.ring = Some((number, component, rect));
            }
            (Some((number, on, at)), None) if on == component => {
                self.cast.ui_remove_object(number);
                self.ui_release(&shapes, at.width, at.height);
                self.components.ring = None;
            }
            _ => {}
        }
        Ok(())
    }

    /// Draws `now` in place of `before`; on a refusal the caller puts
    /// everything back.
    fn ui_paint(&mut self, before: Option<&Component>, now: Option<&Shown<'_>>) -> Result<(), Reply> {
        let gone = before.map(|component| component.handle);
        let drawing = now.map(|shown| drawing(shown, self.text_columns(shown.component))).transpose()?;
        // What the component painted before is painted over in black,
        // then what it is now is painted; a component whose painting is
        // what it was (its highlight, thumb or caret moved, and nothing
        // else) paints nothing, and only its objects change.
        let language = now.map_or("", |shown| shown.component.props.language.as_str()).to_owned();
        let same = drawing.as_ref().is_some_and(|drawing| {
            self.components.drawn.iter().any(|drawn| Some(drawn.component) == gone && drawn.program == drawing.program.words && drawn.language == language)
        });
        if !same {
            let mut program = Program::default();
            for drawn in self.components.drawn.iter().filter(|drawn| Some(drawn.component) == gone) {
                for rect in &drawn.rects {
                    program.fill(*rect, BLACK);
                }
            }
            let mut words = program.words;
            if let Some(drawing) = &drawing {
                words.extend_from_slice(&drawing.program.words);
            }
            self.ui_run(&words, &language)?;
        }
        self.components.drawn.retain(|drawn| Some(drawn.component) != gone);
        // The objects: those the component has and still wants are moved
        // or changed, the rest go, and the new ones are made.
        let handle = now.map(|shown| shown.component.handle);
        let mut had: Vec<Owned> = Vec::new();
        self.components.owned.retain(|owned| {
            let its = Some(owned.component) == gone;
            if its {
                had.push(owned.clone());
            }
            !its
        });
        let wanted = drawing.as_ref().map_or(&[][..], |drawing| drawing.objects.as_slice());
        for (role, object, shapes) in wanted {
            let Some(component) = handle else { break };
            let old = had.iter().position(|owned| owned.role == *role).map(|place| had.swap_remove(place));
            let number = match old {
                Some(old) if old.object == *object && old.shapes == *shapes => old.number,
                // An actor of the same size is moved, with nothing painted or sent again.
                Some(old)
                    if !object.region
                        && old.shapes == *shapes
                        && (old.object.width, old.object.height, old.object.cycle) == (object.width, object.height, object.cycle) =>
                {
                    self.cast.ui_move(old.number, object.x, object.y)?;
                    old.number
                }
                Some(old) => {
                    let number = self.ui_make(Some(old.number), object.clone(), shapes)?;
                    self.ui_release(&old.shapes, old.object.width, old.object.height);
                    number
                }
                None => self.ui_make(None, object.clone(), shapes)?,
            };
            let mut kept = object.clone();
            kept.frames.clear();
            self.components.owned.push(Owned { component, role: *role, number, object: kept, shapes: shapes.clone() });
        }
        for old in had {
            self.cast.ui_remove_object(old.number);
            self.ui_release(&old.shapes, old.object.width, old.object.height);
        }
        // The focus ring follows the component that has the focus.
        if let Some(component) = handle.or(gone) {
            self.ui_ring(component, drawing.as_ref().and_then(|drawing| drawing.focus))?;
        }
        if let (Some(component), Some(drawing)) = (handle, drawing) {
            self.components.drawn.push(Drawn { component, language, program: drawing.program.words, rects: drawing.program.rects });
        }
        Ok(())
    }
}

impl Painter for Naplps {
    fn text_columns(&self, component: &Component) -> usize {
        usize::from(component.at.columns).max(1)
    }

    fn paint(&mut self, before: Option<&Component>, now: Option<&Shown<'_>>) -> Result<(), Reply> {
        if !self.active {
            return Err(type_error("the page is not drawn in NAPLPS by a script of its own"));
        }
        // All or nothing: a refusal part of the way puts everything back.
        let saved = self.clone();
        let painted = self.ui_paint(before, now);
        if painted.is_err() {
            *self = saved;
        }
        painted
    }

    fn part(&self, handle: i32) -> Option<(i32, usize)> {
        let number = Cast::object_number(handle)?;
        self.components.owned.iter().find_map(|owned| match owned.role {
            Role::Part(index) if owned.number == number => Some((owned.component, index)),
            _ => None,
        })
    }
}
