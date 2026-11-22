//! The thirteen first components on ReGIS (work package U2 of
//! script-api.md; section 8's table of how each kind is built; DEC-7205).
//! Each is drawn with what ReGIS v2 has, and costs what ReGIS costs:
//!
//! - **a component is a group of numbered drawings** under one top-level
//!   drawing in the scene layer, which carries its label. They are the
//!   page's drawings, on the page's own table: they count against
//!   `REGIS_MAX_DRAWINGS` and `REGIS_MAX_TOTAL_CONTENTS_BYTES` with the
//!   script's, and a component the page has no room for is refused by
//!   that bound's name with nothing changed;
//! - **what changes is an attribute wherever one will do.** A component
//!   is described afresh at every change as the drawings it wants, and
//!   only what differs from the drawings it has is written: a slider's
//!   thumb is moved by its `x`, a menu and a table are scrolled by the
//!   `y` of the group their rows hang from under a clip rectangle, a
//!   toggle's knob and the ring of the focus are shown and hidden, a
//!   progress bar is the clip rectangle of its filling, a field's text
//!   is moved under its clip as the caret passes its end, and the
//!   highlight is the source of two rows. Contents are written again
//!   only where the text itself changed (`regis-v2.md`, "Attributes");
//! - **the receiver answers the pointer itself.** Each thing the pointer
//!   takes is a detectable drawing with a hover look and a pressed look
//!   (`regis-v2.md`, "Viewer state"): hidden drawings of the component,
//!   shared by all its rows. The drawing paints every pixel of its
//!   rectangle, so that picking finds it between the letters, and its
//!   text is a plain child, which a look leaves drawn and whose pixels
//!   are picked as its parent's (`regis-v2.md`, "Picking");
//! - **a field's caret is the receiver's too:** a drawing with two
//!   frames, cycled by the receiver's clock (`docs/animation.md`
//!   ANIM-14);
//! - **text is any size.** A text too long for its cells is drawn in
//!   narrower cells, down to half width, before it is cut (`fit:
//!   "shrink"`), and double height is a taller cell. There is no
//!   attribute cell: a component of n columns shows n characters.
//!
//! The grid is section 8's: a cell is 20 by 20 pixels, and row `r` is
//! the pixels from y = 20 `r`.

use std::collections::HashMap;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash as _, Hasher as _};

use neoteletext_rules::animate::code;
use protocol::script::{Bounds, ContextId};

use super::painter::written_text;
use super::{Drawing, FLAG_DETECTABLE, FLAG_VISIBLE, HANDLE_NUMBERS};
use crate::host::{Host, Reply, range};
use crate::limits::regis::{DEFAULT_LAYER, MAX_TEXT_CHARS, OPTION_NOT_GIVEN, PAINT_RATE_UNSET, REGIS_COLOURS};
use crate::ops::{Property, RegisPaint};
use crate::ui::limits::COMPONENT_HANDLE_BASE;
use crate::ui::options::{Align, Area, Colour, Fit};
use crate::ui::{Component, Kind, Painter, Shown};

/// A cell's side in pixels (section 8, "Placement").
const CELL: i32 = 20;
/// The narrowest cell a text is drawn in before it is cut: half a cell.
const NARROWEST_CELL: i32 = CELL / 2;
/// The widths of the unit cell, which is a multiple of the character
/// cell's 8 pixels (the VT330/VT340 Programmer Reference Manual, volume
/// 2, chapter 7, "Unit Cell Size"): two for a cell of 16 pixels or more,
/// one for a narrower.
const UNIT_WIDE: i32 = 16;
const UNIT_NARROW: i32 = 8;

/// The colour-map entries of the eight colours every renderer names, in
/// the order of their numbers (black, red, green, yellow, blue, magenta,
/// cyan, white): where the VT340's default colour map has each, white
/// being its lightest grey (the manual, chapter 2, table 2-3). A script
/// that sets one of these entries changes every component drawn in it,
/// as ReGIS has it.
const NAMED: [i32; 8] = [0, 2, 3, 6, 1, 4, 5, 15];
/// The entries components use where a script names none, by the default
/// colour map: the lightest grey for text, yellow for a title and a
/// table's headings, cyan for a link, a hint and a placeholder, dark grey
/// for what is disabled and for a track, green and red for a toggle on
/// and off.
const TEXT_INK: i32 = 15;
const TITLE_INK: i32 = 6;
const LINK_INK: i32 = 5;
const DISABLED_INK: i32 = 8;
const TRACK_INK: i32 = 8;
const ON_INK: i32 = 3;
const OFF_INK: i32 = 2;
const PROMPT_TEXT_INK: i32 = 0;
/// The colours of a frame's prompts: the four coloured keys, red, green,
/// yellow and cyan.
const PROMPT_INKS: [i32; 4] = [2, 3, 6, 5];
/// The backgrounds: the screen's behind what the pointer takes; blue
/// behind a band and the highlighted row; the dimmer blue behind a button
/// and a field; the dimmer cyan under the pointer and cyan while pressed;
/// magenta for the ring of the focus.
const SCREEN_PAPER: i32 = 0;
const BAND_PAPER: i32 = 1;
const HIGHLIGHT_PAPER: i32 = 1;
const CONTROL_PAPER: i32 = 9;
const HOVER_PAPER: i32 = 13;
const PRESSED_PAPER: i32 = 5;
const FOCUS_INK: i32 = 4;

/// The depths of a component's drawings among their siblings: what lies
/// over the part's background, and the ring and the caret over that.
const OVER: i32 = 1;
const TOPMOST: i32 = 2;
/// A slider's thumb, a toggle's switch and knob, and a bar: their sizes
/// in pixels, each inside one cell's height.
const THUMB_WIDTH: i32 = 8;
const THUMB_HEIGHT: i32 = 16;
const TRACK_HEIGHT: i32 = 4;
const SWITCH_WIDTH: i32 = 2 * CELL;
const SWITCH_HEIGHT: i32 = 12;
const KNOB_SIDE: i32 = 8;
const BAR_HEIGHT: i32 = 12;
const RULE_HEIGHT: i32 = 2;
const CARET_WIDTH: i32 = 2;
/// The margin, in pixels, between a shape and the box it lies in.
const MARGIN: i32 = 2;
/// The ticks each of the caret's two frames is shown: the half period of
/// the text cursor's blink (`docs/animation.md`, `BLINK_HALF_TICKS`).
const CARET_HALF_PERIOD_TICKS: i32 = 32;
/// What a masked field shows for each character.
const MASK: char = '*';
/// The writing mode of text: overlay, which writes the letters' pixels
/// and leaves the background behind them (the painter's `mode`).
const MODE_OVERLAY: i32 = 0;
/// The frames of a cycle between two.
const FIRST_FRAME: i32 = 0;
const SECOND_FRAME: i32 = 1;

/// Which of a component's drawings one is.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(super) enum Role {
    /// The top-level drawing the others hang from.
    Root,
    /// Hidden drawings others are drawn as: a row's background, plain
    /// and highlighted, and the looks under the pointer and pressed.
    Plain,
    Highlight,
    Hover,
    Pressed,
    /// The outline shown while the component has the focus.
    Ring,
    /// The clipped window a table's rows are seen through, and the group
    /// that is moved to scroll them.
    Body,
    Scroll,
    /// An item or a row, and its text.
    Row(u16),
    RowText(u16),
    /// The text of a component of one part.
    Text,
    /// A paragraph's prompt, and its text.
    More,
    MoreText,
    /// A toggle's knob, off and on.
    KnobOff,
    KnobOn,
    /// A slider's thumb.
    Thumb,
    /// A progress bar's filling.
    Fill,
    /// A field's caret and its two frames.
    Caret,
    CaretOn,
    CaretOff,
}

/// One drawing a component wants.
struct Want {
    role: Role,
    parent: Option<Role>,
    x: i32,
    y: i32,
    depth: i32,
    visible: bool,
    detectable: bool,
    clip: Option<[i32; 4]>,
    source: Option<Role>,
    hover: Option<Role>,
    pressed: Option<Role>,
    /// Whether it cycles between the caret's two frames.
    blinks: bool,
    label: String,
    program: Vec<i32>,
    /// The part of the component the pointer takes here.
    part: Option<usize>,
}

impl Want {
    /// A shown drawing with no contents, at its parent's origin.
    const fn new(role: Role, parent: Option<Role>) -> Self {
        Self {
            role,
            parent,
            x: 0,
            y: 0,
            depth: 0,
            visible: true,
            detectable: false,
            clip: None,
            source: None,
            hover: None,
            pressed: None,
            blinks: false,
            label: String::new(),
            program: Vec::new(),
            part: None,
        }
    }

    /// A hidden child of the root, drawn only where another names it.
    const fn hidden(role: Role) -> Self {
        let mut want = Self::new(role, Some(Role::Root));
        want.visible = false;
        want
    }

    /// At `x`, `y` from its parent's origin.
    const fn at(mut self, x: i32, y: i32) -> Self {
        self.x = x;
        self.y = y;
        self
    }

    /// With the contents `program` writes.
    fn drawn(mut self, program: Vec<i32>) -> Self {
        self.program = program;
        self
    }
}

/// One drawing a component has.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Node {
    component: i32,
    role: Role,
    number: u16,
    /// What tells its contents' program from another.
    program: u64,
    part: Option<u16>,
}

/// What the page's components are drawn as, and what is owed them.
#[derive(Default)]
pub(super) struct Held {
    pub(super) nodes: Vec<Node>,
    /// Whether the host is changing drawings as the components' painter:
    /// the drawings it makes are components', and it reaches no other.
    pub(super) painting: bool,
    /// The bytes of the vocabulary's own state while the painter has it.
    lent: usize,
    /// Whether the components are to be drawn again: a commit was
    /// refused, and the drawings are as they were before its step.
    owed: bool,
    /// Whether that was tried in a step whose commit is not yet known.
    tried: bool,
}

impl Held {
    /// The bytes held for the components' drawings.
    pub(super) fn held_bytes(&self) -> usize {
        self.nodes.len().saturating_mul(size_of::<Node>()).saturating_add(self.lent)
    }

    /// A commit was refused: the drawings are `nodes` again, as the step
    /// began, and the components as they now are owe a drawing. Not when
    /// the refused step was itself that drawing, which would be tried
    /// without end.
    pub(super) fn put_back(&mut self, nodes: Vec<Node>) {
        self.nodes = nodes;
        self.owed = !self.tried;
    }

    /// A step's commit was made.
    pub(super) const fn committed(&mut self) {
        self.tried = false;
    }
}

/// A painter program being written.
#[derive(Default)]
struct Program(Vec<i32>);

/// One line of text to write: where, in how many cells, and how.
struct Line<'a> {
    x: i32,
    y: i32,
    /// The cells it may take.
    room: i32,
    text: &'a str,
    align: Align,
    ink: i32,
    tall: bool,
    fit: Fit,
}

/// The languages a text is written in: the one its component names, and
/// the context's.
#[derive(Clone, Copy)]
struct Languages<'a> {
    named: &'a str,
    own: &'a str,
}

impl Program {
    /// A filled rectangle; nothing when it has no pixel.
    fn fill(&mut self, x: i32, y: i32, width: i32, height: i32, ink: i32) {
        if width < 1 || height < 1 {
            return;
        }
        let body = [RegisPaint::RECT.number(), x, y, width, height];
        self.0.extend_from_slice(&[RegisPaint::INK.number(), ink, RegisPaint::FILL.number(), i32::try_from(body.len()).unwrap_or(0)]);
        self.0.extend_from_slice(&body);
    }

    /// A rectangle's outline.
    fn outline(&mut self, x: i32, y: i32, width: i32, height: i32, ink: i32) {
        if width < 1 || height < 1 {
            return;
        }
        self.0.extend_from_slice(&[RegisPaint::INK.number(), ink, RegisPaint::RECT.number(), x, y, width, height]);
    }

    /// A string as the painter's `text` call takes one: its length, then
    /// its code points.
    fn string(&mut self, text: &str) {
        self.0.push(i32::try_from(text.chars().count()).unwrap_or(i32::MAX));
        self.0.extend(text.chars().map(|character| i32::try_from(u32::from(character)).unwrap_or(0)));
    }

    /// One line of text in the provided sets, in overlay writing: cut or
    /// narrowed to its room as `fit` says, and laid as `align` says.
    fn line(&mut self, languages: Languages<'_>, line: &Line<'_>) -> Result<(), Reply> {
        let cells_of = |text: &str| written_text(text, languages.named, languages.own).map(|(_, cells)| cells);
        // A text that is not refused is cut to what a string may hold
        // before the renderer's own bound would refuse it.
        let mut shown: String = if line.fit == Fit::Refuse { line.text.to_owned() } else { line.text.chars().take(MAX_TEXT_CHARS).collect() };
        let mut cells = cells_of(&shown)?;
        if cells == 0 {
            return Ok(());
        }
        let room_pixels = line.room.saturating_mul(CELL);
        let mut advance = CELL;
        if cells > line.room {
            match line.fit {
                Fit::Refuse => {
                    return Err(range(
                        "UI_GRID_COLUMNS",
                        &format!(
                            "the text takes {cells} cells and the component has {}; fit: \"shrink\" draws it in narrower cells, and \"cut\" cuts it",
                            line.room
                        ),
                    ));
                }
                Fit::Shrink => advance = room_pixels.checked_div(cells).unwrap_or(CELL).max(NARROWEST_CELL),
                Fit::Cut => {}
            }
            // What still does not fit is cut: the longest beginning of
            // the text whose cells do, found by halving.
            let most = room_pixels.checked_div(advance).unwrap_or(0);
            if cells > most {
                let characters: Vec<char> = shown.chars().collect();
                let (mut fits, mut over) = (0_usize, characters.len());
                while over.saturating_sub(fits) > 1 {
                    let middle = fits.saturating_add(over).checked_div(2).unwrap_or(fits);
                    let beginning: String = characters.iter().take(middle).collect();
                    if cells_of(&beginning)? <= most {
                        fits = middle;
                    } else {
                        over = middle;
                    }
                }
                shown = characters.iter().take(fits).collect();
                cells = cells_of(&shown)?;
                if cells == 0 {
                    return Ok(());
                }
            }
        }
        let spare = room_pixels.saturating_sub(cells.saturating_mul(advance)).max(0);
        let before = match line.align {
            Align::Left => 0,
            Align::Center => spare.checked_div(2).unwrap_or(0),
            Align::Right => spare,
        };
        let height = if line.tall { CELL.saturating_mul(2) } else { CELL };
        let unit = if advance >= UNIT_WIDE { UNIT_WIDE } else { UNIT_NARROW };
        let none = OPTION_NOT_GIVEN;
        self.0.extend_from_slice(&[RegisPaint::INK.number(), line.ink, RegisPaint::MODE.number(), MODE_OVERLAY]);
        self.0.extend_from_slice(&[RegisPaint::MOVE_TO.number(), line.x.saturating_add(before), line.y]);
        // The display cell is the advance, and the unit cell the letter
        // drawn in it (the painter's `text`: size, cell, unit, height,
        // tilt, italic, spacing).
        self.0.extend_from_slice(&[RegisPaint::TEXT.number(), none, advance, height, unit, height, none, none, none, none, none]);
        self.string(languages.named);
        self.string(&shown);
        Ok(())
    }
}

/// The colour-map entry of a colour a script named, or `default`.
fn entry(colour: Option<Colour>, default: i32) -> Result<i32, Reply> {
    let Some(colour) = colour else { return Ok(default) };
    if let Some(named) = NAMED.get(usize::from(colour)) {
        return Ok(*named);
    }
    if usize::from(colour) >= REGIS_COLOURS {
        return Err(range(
            "REGIS_COLOURS",
            &format!("on ReGIS a component's colour is one of the eight names or a colour-map entry, 8 to {}; not {colour}", REGIS_COLOURS.saturating_sub(1)),
        ));
    }
    Ok(i32::from(colour))
}

/// A number of cells in pixels.
fn pixels(cells: impl Into<i32>) -> i32 {
    cells.into().saturating_mul(CELL)
}

/// The place of an item or a row as a role holds it.
fn place(index: usize) -> u16 {
    u16::try_from(index).unwrap_or(u16::MAX)
}

/// `needed` rows, or the refusal.
fn needs(component: &Component, needed: u8, what: &str) -> Result<(), Reply> {
    if component.at.rows < needed {
        return Err(range("UI_GRID_ROWS", &format!("{what} takes {needed} rows, and the component has {}", component.at.rows)));
    }
    Ok(())
}

/// What makes a component's drawings: its place, its colours and its
/// languages, and the drawings wanted so far.
struct Maker<'a> {
    shown: &'a Shown<'a>,
    languages: Languages<'a>,
    width: i32,
    height: i32,
    ink: i32,
    wants: Vec<Want>,
}

impl<'a> Maker<'a> {
    fn new(shown: &'a Shown<'a>, own: &'a str) -> Result<Self, Reply> {
        let component = shown.component;
        let Area { columns, rows, .. } = component.at;
        let ink = entry(component.props.ink, if component.props.disabled { DISABLED_INK } else { TEXT_INK })?;
        Ok(Self { shown, languages: Languages { named: &component.props.language, own }, width: pixels(columns), height: pixels(rows), ink, wants: Vec::new() })
    }

    /// The root: at the component's cells, over the components made
    /// before it, with its label and `program`.
    fn root(&mut self, program: Program) {
        let component = self.shown.component;
        let mut root = Want::new(Role::Root, None).at(pixels(component.at.column), pixels(component.at.row)).drawn(program.0);
        // Components lie in the order they were made, whatever numbers
        // their drawings were given.
        root.depth = component.handle.saturating_sub(COMPONENT_HANDLE_BASE).clamp(0, i32::from(i16::MAX));
        root.label.clone_from(&component.props.label);
        self.wants.push(root);
    }

    /// A program that begins with the component's paper over `width` by
    /// `height` pixels: the one a script named, or `default` when the
    /// kind has one.
    fn papered(&self, width: i32, height: i32, default: Option<i32>) -> Result<Program, Reply> {
        let mut program = Program::default();
        let paper = match (self.shown.component.props.paper, default) {
            (None, None) => return Ok(program),
            (named, default) => entry(named, default.unwrap_or(SCREEN_PAPER))?,
        };
        program.fill(0, 0, width, height, paper);
        Ok(program)
    }

    /// One line of the component's text at `x`, `y` in `room` cells.
    #[expect(clippy::too_many_arguments, reason = "a line's place, its room, its text and how it lies")]
    fn line(&self, program: &mut Program, x: i32, y: i32, room: i32, text: &str, align: Align, ink: i32) -> Result<(), Reply> {
        program.line(self.languages, &Line { x, y, room, text, align, ink, tall: false, fit: Fit::Cut })
    }

    /// The looks the receiver draws under the pointer and while pressed,
    /// for parts `width` pixels wide: hidden drawings of the component,
    /// below `parent`. The looks of the root itself hang from nothing,
    /// since no drawing reaches itself through its looks and their
    /// parents (`regis-v2.md`, "Lifetime and memory").
    fn looks(&mut self, width: i32, parent: Option<Role>) {
        for (role, paper) in [(Role::Hover, HOVER_PAPER), (Role::Pressed, PRESSED_PAPER)] {
            let mut program = Program::default();
            program.fill(0, 0, width, CELL, paper);
            let mut look = Want::hidden(role).drawn(program.0);
            look.parent = parent;
            self.wants.push(look);
        }
    }

    /// The ring shown while the component has the focus, over `width` by
    /// `height` pixels of it.
    fn ring(&mut self, width: i32, height: i32) {
        let mut program = Program::default();
        program.outline(0, 0, width, height, FOCUS_INK);
        let mut ring = Want::new(Role::Ring, Some(Role::Root)).drawn(program.0);
        ring.depth = TOPMOST;
        ring.visible = self.shown.focused;
        self.wants.push(ring);
    }

    /// Makes the root what the pointer takes, unless the component is
    /// disabled: detectable, with the two looks, as part 0.
    fn takes_pointer(&mut self) {
        if self.shown.component.props.disabled {
            return;
        }
        let width = self.width;
        self.looks(width, None);
        if let Some(root) = self.wants.first_mut() {
            root.detectable = true;
            root.hover = Some(Role::Hover);
            root.pressed = Some(Role::Pressed);
            root.part = Some(0);
        }
    }

    /// A child of the root with one line of text over its background.
    fn text_child(&mut self, role: Role, parent: Role, room: i32, text: &str, align: Align, ink: i32) -> Result<(), Reply> {
        let mut program = Program::default();
        self.line(&mut program, 0, 0, room, text, align, ink)?;
        let mut child = Want::new(role, Some(parent)).drawn(program.0);
        child.depth = OVER;
        self.wants.push(child);
        Ok(())
    }

    /// The rows of a menu or a table: the backgrounds they are drawn as,
    /// the group moved to scroll them under `window`, and each row with
    /// its text, written by `text`. A row the pointer takes is drawn as
    /// its background, plain or highlighted, with the two looks.
    fn rows(
        &mut self,
        window: Role,
        count: usize,
        taken: &dyn Fn(usize) -> bool,
        text: &dyn Fn(&Self, usize, &mut Program) -> Result<(), Reply>,
    ) -> Result<(), Reply> {
        let component = self.shown.component;
        let width = self.width;
        let any_taken = (0..count).any(taken);
        if any_taken {
            let paper = entry(component.props.paper, SCREEN_PAPER)?;
            for (role, colour) in [(Role::Plain, paper), (Role::Highlight, HIGHLIGHT_PAPER)] {
                let mut program = Program::default();
                program.fill(0, 0, width, CELL, colour);
                self.wants.push(Want::hidden(role).drawn(program.0));
            }
            self.looks(width, Some(Role::Root));
        }
        let top = i32::try_from(component.view.top).unwrap_or(i32::MAX);
        self.wants.push(Want::new(Role::Scroll, Some(window)).at(0, pixels(top).saturating_neg()));
        for index in 0..count {
            let y = pixels(i32::try_from(index).unwrap_or(i32::MAX));
            let mut program = Program::default();
            text(self, index, &mut program)?;
            let highlighted = index == component.props.selected && (component.kind == Kind::Menu || component.props.selectable);
            if taken(index) || (highlighted && any_taken) {
                let mut row = Want::new(Role::Row(place(index)), Some(Role::Scroll)).at(0, y);
                row.source = Some(if highlighted { Role::Highlight } else { Role::Plain });
                if taken(index) {
                    row.detectable = true;
                    row.hover = Some(Role::Hover);
                    row.pressed = Some(Role::Pressed);
                    row.part = Some(index);
                }
                self.wants.push(row);
                let mut words = Want::new(Role::RowText(place(index)), Some(Role::Row(place(index)))).drawn(program.0);
                words.depth = OVER;
                self.wants.push(words);
            } else {
                self.wants.push(Want::new(Role::RowText(place(index)), Some(Role::Scroll)).at(0, y).drawn(program.0));
            }
        }
        Ok(())
    }

    /// The drawings of the component, by its kind.
    #[expect(clippy::too_many_lines, reason = "one arm for each of the thirteen kinds, each a few lines")]
    fn make(mut self) -> Result<Vec<Want>, Reply> {
        let shown = self.shown;
        let component = shown.component;
        let props = &component.props;
        let Area { columns, rows, .. } = component.at;
        let (width, height, ink) = (self.width, self.height, self.ink);
        let room = i32::from(columns);
        let last_row_y = pixels(rows.saturating_sub(1));
        match component.kind {
            Kind::Frame => {
                let ruled = u8::from(!props.bare);
                let hinted = u8::from(!props.hint.is_empty());
                let prompted = u8::from(!props.prompts.is_empty());
                needs(component, 1_u8.saturating_add(ruled).saturating_add(hinted).saturating_add(prompted), "a frame's title, rule, hint and prompts")?;
                let title_ink = entry(props.ink, TITLE_INK)?;
                let mut program = self.papered(width, height, None)?;
                self.line(&mut program, 0, 0, room, &props.title, Align::Left, title_ink)?;
                if !props.bare {
                    // A rule across the middle of the row under the title.
                    program.fill(0, CELL.saturating_add(CELL.saturating_sub(RULE_HEIGHT).checked_div(2).unwrap_or(0)), width, RULE_HEIGHT, title_ink);
                }
                if !props.hint.is_empty() {
                    self.line(&mut program, 0, last_row_y.saturating_sub(pixels(prompted)), room, &props.hint, Align::Left, LINK_INK)?;
                }
                // The prompts share the last row, each a block of its
                // key's colour with its text in black.
                let count = i32::try_from(props.prompts.len()).unwrap_or(1).max(1);
                let each = room.checked_div(count).unwrap_or(room);
                for ((at, prompt), colour) in (0_i32..).zip(&props.prompts).zip(PROMPT_INKS) {
                    let x = pixels(at.saturating_mul(each));
                    program.fill(x.saturating_add(1), last_row_y.saturating_add(1), pixels(each).saturating_sub(MARGIN), CELL.saturating_sub(MARGIN), colour);
                    self.line(&mut program, x, last_row_y, each, prompt, Align::Center, PROMPT_TEXT_INK)?;
                }
                self.root(program);
            }
            Kind::Text => {
                let tall = props.height == 2;
                if tall {
                    needs(component, 2, "a text of double height")?;
                }
                let mut program = self.papered(width, height, None)?;
                program.line(self.languages, &Line { x: 0, y: 0, room, text: &props.text, align: props.align, ink, tall, fit: props.fit })?;
                self.root(program);
            }
            Kind::Paragraph => {
                let lines = component.lines(usize::from(columns));
                let page = component.page_lines(lines.len());
                let mut program = self.papered(width, height, None)?;
                for (at, line) in (0_i32..).zip(lines.iter().skip(component.view.top).take(page)) {
                    self.line(&mut program, 0, pixels(at), room, line, Align::Left, ink)?;
                }
                self.root(program);
                // The prompt is shown while more follows, and takes the
                // pointer.
                if component.view.top.saturating_add(page) < lines.len() {
                    self.looks(width, Some(Role::Root));
                    let mut background = Program::default();
                    background.fill(0, 0, width, CELL, entry(props.paper, SCREEN_PAPER)?);
                    let mut more = Want::new(Role::More, Some(Role::Root)).at(0, last_row_y).drawn(background.0);
                    more.depth = OVER;
                    more.detectable = true;
                    more.hover = Some(Role::Hover);
                    more.pressed = Some(Role::Pressed);
                    more.part = Some(0);
                    self.wants.push(more);
                    self.text_child(Role::MoreText, Role::More, room, &props.more, Align::Right, TITLE_INK)?;
                }
                self.ring(width, height);
            }
            Kind::Band => {
                let mut program = self.papered(width, height, Some(BAND_PAPER))?;
                self.line(&mut program, 0, 0, room, &props.text, Align::Center, ink)?;
                self.root(program);
            }
            Kind::Link | Kind::Button => {
                let button = component.kind == Kind::Button;
                let (paper, default, align) = if button { (CONTROL_PAPER, TEXT_INK, Align::Center) } else { (SCREEN_PAPER, LINK_INK, Align::Left) };
                let program = self.papered(width, CELL, Some(paper))?;
                self.root(program);
                self.takes_pointer();
                let text_ink = entry(props.ink, if props.disabled { DISABLED_INK } else { default })?;
                self.text_child(Role::Text, Role::Root, room, &props.text, align, text_ink)?;
                self.ring(width, CELL);
            }
            Kind::Toggle => {
                let switch_cells = SWITCH_WIDTH.checked_div(CELL).unwrap_or(0);
                if room <= switch_cells {
                    return Err(range(
                        "UI_GRID_COLUMNS",
                        &format!("a toggle is at least {} columns wide: its switch takes {switch_cells}", switch_cells.saturating_add(1)),
                    ));
                }
                // The switch lies at the right: its outline, and a knob
                // at each end of which one is shown.
                let switch_x = width.saturating_sub(SWITCH_WIDTH);
                let switch_y = CELL.saturating_sub(SWITCH_HEIGHT).checked_div(2).unwrap_or(0);
                let mut program = self.papered(width, CELL, Some(SCREEN_PAPER))?;
                program.outline(switch_x, switch_y, SWITCH_WIDTH, SWITCH_HEIGHT, if props.disabled { DISABLED_INK } else { TEXT_INK });
                self.root(program);
                self.takes_pointer();
                self.text_child(Role::Text, Role::Root, room.saturating_sub(switch_cells), &props.text, Align::Left, ink)?;
                let knob_y = switch_y.saturating_add(MARGIN);
                let off_x = switch_x.saturating_add(MARGIN);
                let on_x = switch_x.saturating_add(SWITCH_WIDTH).saturating_sub(MARGIN).saturating_sub(KNOB_SIDE.saturating_mul(2));
                for (role, x, colour, on) in [(Role::KnobOff, off_x, OFF_INK, false), (Role::KnobOn, on_x, ON_INK, true)] {
                    let mut knob = Program::default();
                    knob.fill(0, 0, KNOB_SIDE.saturating_mul(2), KNOB_SIDE, colour);
                    let mut want = Want::new(role, Some(Role::Root)).at(x, knob_y).drawn(knob.0);
                    want.depth = OVER;
                    want.visible = props.on == on;
                    self.wants.push(want);
                }
                self.ring(width, CELL);
            }
            Kind::Menu => {
                let visible = i32::try_from(component.visible()).unwrap_or(i32::MAX);
                let program = self.papered(width, height, None)?;
                self.root(program);
                if let Some(root) = self.wants.first_mut() {
                    root.clip = Some([0, 0, width, pixels(visible)]);
                }
                let taken = |index: usize| !props.disabled && props.items.get(index).is_some_and(|item| !item.disabled);
                self.rows(Role::Root, props.items.len(), &taken, &|maker, index, program| {
                    let Some(item) = props.items.get(index) else { return Ok(()) };
                    let item_ink = if item.disabled || props.disabled { DISABLED_INK } else { maker.ink };
                    maker.line(program, 0, 0, room, &item.text, Align::Left, item_ink)
                })?;
                self.ring(width, pixels(visible));
            }
            Kind::Table => {
                let across = props.columns.iter().fold(0_i32, |sum, one| sum.saturating_add(i32::from(one.width)));
                if across > room {
                    return Err(range("UI_GRID_COLUMNS", &format!("the table's columns take {across} cells, and the component has {columns}")));
                }
                // A row: each column's text in its own cells.
                let row = |maker: &Self, program: &mut Program, texts: &[String], row_ink: i32| -> Result<(), Reply> {
                    let mut x = 0_i32;
                    for (at, one) in props.columns.iter().enumerate() {
                        let text = texts.get(at).map_or("", String::as_str);
                        maker.line(program, x, 0, i32::from(one.width), text, one.align, row_ink)?;
                        x = x.saturating_add(pixels(one.width));
                    }
                    Ok(())
                };
                let mut program = self.papered(width, height, None)?;
                let titles: Vec<String> = props.columns.iter().map(|one| one.title.clone()).collect();
                row(&self, &mut program, &titles, TITLE_INK)?;
                self.root(program);
                // The rows are seen under the headings, through a window
                // of the rows shown.
                let visible = i32::try_from(component.visible()).unwrap_or(i32::MAX);
                let mut body = Want::new(Role::Body, Some(Role::Root)).at(0, CELL);
                body.clip = Some([0, 0, width, pixels(visible)]);
                self.wants.push(body);
                let taken = |_: usize| props.selectable && !props.disabled;
                self.rows(Role::Body, props.rows.len(), &taken, &|maker, index, program| {
                    let Some(texts) = props.rows.get(index) else { return Ok(()) };
                    row(maker, program, texts, maker.ink)
                })?;
                if props.selectable {
                    self.ring(width, pixels(visible.saturating_add(1)));
                }
            }
            Kind::Field => {
                let program = self.papered(width, CELL, Some(CONTROL_PAPER))?;
                self.root(program);
                if let Some(root) = self.wants.first_mut() {
                    root.clip = Some([0, 0, width, CELL]);
                }
                self.takes_pointer();
                // The whole value is drawn, and moved under the field's
                // clip so that the caret is among the cells shown.
                let first = i32::try_from(component.view.caret.saturating_add(1).saturating_sub(usize::from(columns))).unwrap_or(0);
                let placeholder = props.value.is_empty() && !shown.focused;
                let (text, text_ink): (String, i32) = if placeholder {
                    (props.placeholder.clone(), LINK_INK)
                } else if props.masked {
                    (props.value.chars().map(|_| MASK).collect(), ink)
                } else {
                    (props.value.clone(), ink)
                };
                let cells = i32::try_from(text.chars().count()).unwrap_or(i32::MAX).max(room);
                self.text_child(Role::Text, Role::Root, cells, &text, Align::Left, text_ink)?;
                if let Some(written) = self.wants.last_mut() {
                    written.x = pixels(first).saturating_neg();
                }
                if shown.focused {
                    // The caret: a bar before the character it stands at,
                    // shown and hidden by the receiver's own clock.
                    let mut bar = Program::default();
                    bar.fill(0, MARGIN, CARET_WIDTH, CELL.saturating_sub(MARGIN.saturating_mul(2)), TEXT_INK);
                    self.wants.push(Want::hidden(Role::CaretOn).drawn(bar.0));
                    self.wants.push(Want::hidden(Role::CaretOff));
                    let caret_cell = i32::try_from(component.view.caret).unwrap_or(i32::MAX).saturating_sub(first).min(room.saturating_sub(1));
                    let mut caret = Want::new(Role::Caret, Some(Role::Root)).at(pixels(caret_cell), 0);
                    caret.depth = TOPMOST;
                    caret.blinks = true;
                    self.wants.push(caret);
                }
                self.ring(width, CELL);
            }
            Kind::Slider => {
                // The track along the middle of the row, and the thumb,
                // which alone moves.
                let mut program = self.papered(width, CELL, Some(SCREEN_PAPER))?;
                program.fill(0, CELL.saturating_sub(TRACK_HEIGHT).checked_div(2).unwrap_or(0), width, TRACK_HEIGHT, TRACK_INK);
                self.root(program);
                if !props.disabled
                    && let Some(root) = self.wants.first_mut()
                {
                    root.detectable = true;
                    root.part = Some(0);
                }
                let span = i64::from(width.saturating_sub(THUMB_WIDTH).max(0));
                let whole = props.max.saturating_sub(props.min).max(1);
                let along = props.number.saturating_sub(props.min).clamp(0, whole).saturating_mul(span).checked_div(whole).unwrap_or(0);
                let mut thumb_program = Program::default();
                thumb_program.fill(0, 0, THUMB_WIDTH, THUMB_HEIGHT, ink);
                let thumb_y = CELL.saturating_sub(THUMB_HEIGHT).checked_div(2).unwrap_or(0);
                let mut thumb = Want::new(Role::Thumb, Some(Role::Root)).at(i32::try_from(along).unwrap_or(0), thumb_y).drawn(thumb_program.0);
                thumb.depth = OVER;
                self.wants.push(thumb);
                self.ring(width, CELL);
            }
            Kind::Progress => {
                let mut program = self.papered(width, height, None)?;
                let bar_y = if rows >= 2 {
                    self.line(&mut program, 0, 0, room, &props.text, Align::Left, ink)?;
                    CELL
                } else {
                    0
                };
                let top = bar_y.saturating_add(CELL.saturating_sub(BAR_HEIGHT).checked_div(2).unwrap_or(0));
                program.outline(0, top, width, BAR_HEIGHT, TRACK_INK);
                self.root(program);
                // The filling is drawn whole once, and shown as far as
                // the value through its clip rectangle.
                let inside_width = width.saturating_sub(MARGIN.saturating_mul(2)).max(0);
                let inside_height = BAR_HEIGHT.saturating_sub(MARGIN.saturating_mul(2));
                let whole = props.max.max(1);
                let filled = props.number.clamp(0, whole).saturating_mul(i64::from(inside_width)).checked_div(whole).unwrap_or(0);
                let mut filling = Program::default();
                filling.fill(0, 0, inside_width, inside_height, entry(props.ink, ON_INK)?);
                let mut fill = Want::new(Role::Fill, Some(Role::Root)).at(MARGIN, top.saturating_add(MARGIN)).drawn(filling.0);
                fill.depth = OVER;
                fill.clip = Some([0, 0, i32::try_from(filled).unwrap_or(0), inside_height]);
                self.wants.push(fill);
            }
            Kind::Clock => {
                let (hours, minutes, seconds) = shown.time.parts(props.utc);
                let text = if props.seconds { format!("{hours:02}:{minutes:02}:{seconds:02}") } else { format!("{hours:02}:{minutes:02}") };
                let mut program = self.papered(width, height, None)?;
                self.line(&mut program, 0, 0, room, &text, Align::Left, ink)?;
                self.root(program);
            }
        }
        Ok(self.wants)
    }
}

/// What tells one program from another: a hash no script can steer two
/// programs to share, since it never sees one.
fn told(program: &[i32]) -> u64 {
    let mut hasher = DefaultHasher::new();
    program.hash(&mut hasher);
    hasher.finish()
}

/// The page's tables as a drawing of a component began, to put back when
/// the renderer has no room for all of it.
struct Mark {
    drawings: Vec<Drawing>,
    next: [u16; crate::limits::CONTEXTS],
    contents: [usize; crate::limits::CONTEXTS],
    out: usize,
    pending: usize,
    animate: Vec<neoteletext_rules::animate::Operation>,
    bounds: Vec<Bounds>,
    lists_changed: bool,
    nodes: Vec<Node>,
}

/// The components' painter on a ReGIS page: the host, changing the
/// page's drawings by its own ReGIS operations, each held to the bounds
/// a script's own call is.
pub(in crate::host) struct Components<'a> {
    host: &'a mut Host,
    context: ContextId,
}

impl<'a> Components<'a> {
    /// The painter of `context`'s components, while the vocabulary's own
    /// state, of `lent` bytes, is in the caller's hands and not the
    /// host's: it is counted as held all the same.
    pub(in crate::host) fn new(host: &'a mut Host, context: ContextId, lent: usize) -> Self {
        host.regis.ui.lent = lent;
        Self { host, context }
    }

    /// Whether the components are to be drawn again, which this asks
    /// once: a commit was refused and took their last changes with it.
    pub(in crate::host) fn owed(&mut self) -> bool {
        let ui = &mut self.host.regis.ui;
        let owed = core::mem::take(&mut ui.owed);
        ui.tried |= owed;
        owed
    }

    /// Deletes the drawings of every component but those of `handles`:
    /// what a refused commit brought back of components since removed.
    pub(in crate::host) fn keep_only(&mut self, handles: &[i32]) {
        let mut gone: Vec<i32> = self.host.regis.ui.nodes.iter().map(|node| node.component).filter(|component| !handles.contains(component)).collect();
        gone.dedup();
        for component in gone {
            let _ = self.change(component, &[]);
        }
    }

    fn mark(&mut self) -> Mark {
        let regis = &mut self.host.regis;
        // What is staged from here on is added after what is staged
        // already, so that taking it back is cutting it off.
        regis.open_set = None;
        Mark {
            drawings: regis.drawings.clone(),
            next: regis.next,
            contents: regis.contents,
            out: regis.out.len(),
            pending: regis.pending,
            animate: regis.animate.clone(),
            bounds: regis.bounds.clone(),
            lists_changed: regis.lists_changed,
            nodes: regis.ui.nodes.clone(),
        }
    }

    fn rewind(&mut self, mark: Mark) {
        let regis = &mut self.host.regis;
        regis.drawings = mark.drawings;
        regis.next = mark.next;
        regis.contents = mark.contents;
        regis.out.truncate(mark.out);
        regis.open_set = None;
        regis.pending = mark.pending;
        regis.animate = mark.animate;
        regis.bounds = mark.bounds;
        regis.lists_changed = mark.lists_changed;
        regis.ui.nodes = mark.nodes;
    }

    /// Makes the drawings of `component` those of `wants`, or changes
    /// nothing and gives the refusal.
    fn change(&mut self, component: i32, wants: &[Want]) -> Result<(), Reply> {
        let mark = self.mark();
        self.host.regis.ui.painting = true;
        let changed = self.reconcile(component, wants);
        self.host.regis.ui.painting = false;
        if changed.is_err() {
            self.rewind(mark);
        }
        changed
    }

    /// The handle the host's own operations name drawing `number` by; 0
    /// for none.
    fn handle(&self, number: u16) -> i32 {
        if number == 0 { 0 } else { self.host.regis.handle(number) }
    }

    /// Writes what differs between the drawings `component` has and
    /// those it wants: new drawings, changed attributes, changed
    /// contents, and the deletion of what is wanted no longer.
    fn reconcile(&mut self, component: i32, wants: &[Want]) -> Result<(), Reply> {
        let context = self.context;
        let had: HashMap<Role, (u16, u64)> =
            self.host.regis.ui.nodes.iter().filter(|node| node.component == component).map(|node| (node.role, (node.number, node.program))).collect();
        let mut numbers: HashMap<Role, u16> = HashMap::new();
        let mut nodes: Vec<Node> = Vec::with_capacity(wants.len());
        let nothing = told(&[]);
        // A look named before its drawing is made is given when it is.
        let mut later: Vec<(u16, Property, Role)> = Vec::new();
        for want in wants {
            let named = |role: Option<Role>| role.and_then(|role| numbers.get(&role).copied()).unwrap_or(0);
            let (parent, source, hover, pressed) = (named(want.parent), named(want.source), named(want.hover), named(want.pressed));
            let existing = had.get(&want.role).copied().filter(|(number, _)| self.host.regis.get(*number).is_some());
            let (number, program_before) = if let Some((number, program)) = existing {
                self.host.regis_set(context, number, Property::PARENT, self.handle(parent))?;
                self.host.regis_set(context, number, Property::SOURCE, self.handle(source))?;
                self.host.regis_set(context, number, Property::X, want.x)?;
                self.host.regis_set(context, number, Property::Y, want.y)?;
                self.host.regis_set(context, number, Property::DEPTH, want.depth)?;
                self.host.regis_set(context, number, Property::VISIBLE, i32::from(want.visible))?;
                self.host.regis_set(context, number, Property::DETECTABLE, i32::from(want.detectable))?;
                (number, program)
            } else {
                let flags = if want.visible { FLAG_VISIBLE } else { 0 } | if want.detectable { FLAG_DETECTABLE } else { 0 };
                let made = self
                    .host
                    .regis_drawing(context, [self.handle(parent), self.handle(source), DEFAULT_LAYER, want.x, want.y, want.depth, PAINT_RATE_UNSET, flags])?;
                let Reply::Int(handle) = made else { return Err(Reply::Error("a component's drawing was not made".to_owned())) };
                let number = handle.checked_rem(HANDLE_NUMBERS).and_then(|number| u16::try_from(number).ok()).unwrap_or(0);
                (number, nothing)
            };
            for (property, role, named) in [(Property::HOVER, want.hover, hover), (Property::PRESSED, want.pressed, pressed)] {
                match role {
                    Some(role) if named == 0 => later.push((number, property, role)),
                    _ => self.host.regis_set(context, number, property, self.handle(named))?,
                }
            }
            let clip = match want.clip {
                Some(clip) => {
                    let mut held = [0_i16; 4];
                    for (slot, value) in held.iter_mut().zip(clip) {
                        *slot = super::attribute(value.max(0), "a component's clip rectangle")?;
                    }
                    Some(held)
                }
                None => None,
            };
            let now = self.host.regis.get(number);
            let (reclip, relabel, unframed) = (
                now.is_some_and(|drawing| drawing.clip != clip),
                now.is_some_and(|drawing| drawing.label.as_deref().unwrap_or("") != want.label),
                now.is_some_and(|drawing| drawing.frames.is_none()),
            );
            if reclip {
                self.host.regis_clip(number, clip)?;
            }
            if relabel {
                self.host.regis_label(number, &want.label)?;
            }
            if want.blinks && unframed {
                let frames = vec![numbers.get(&Role::CaretOn).copied().unwrap_or(0), numbers.get(&Role::CaretOff).copied().unwrap_or(0)];
                self.host.regis_frames(number, frames)?;
                self.host.regis_animate(number, i32::from(code::CYCLE), [FIRST_FRAME, SECOND_FRAME, CARET_HALF_PERIOD_TICKS, 0])?;
            }
            let program = told(&want.program);
            if program != program_before {
                self.host.regis_define(context, number, &want.program)?;
            }
            numbers.insert(want.role, number);
            nodes.push(Node { component, role: want.role, number, program, part: want.part.map(place) });
        }
        for (number, property, role) in later {
            let named = numbers.get(&role).copied().unwrap_or(0);
            self.host.regis_set(context, number, property, self.handle(named))?;
        }
        // What is wanted no longer goes, each with the drawings below
        // it: one already gone with its parent is not deleted twice.
        let mut unwanted: Vec<u16> = had.iter().filter(|(role, _)| !numbers.contains_key(role)).map(|(_, (number, _))| *number).collect();
        unwanted.sort_unstable();
        // A component erased whole is one deletion, of its root.
        if wants.is_empty()
            && let Some((root, _)) = had.get(&Role::Root)
        {
            unwanted.retain(|number| number != root);
            unwanted.insert(0, *root);
        }
        for number in unwanted {
            if self.host.regis.get(number).is_some() {
                self.host.regis_remove(number)?;
            }
        }
        let held = &mut self.host.regis.ui.nodes;
        held.retain(|node| node.component != component);
        held.extend(nodes);
        Ok(())
    }
}

impl Drop for Components<'_> {
    fn drop(&mut self) {
        self.host.regis.ui.lent = 0;
    }
}

impl Painter for Components<'_> {
    fn text_columns(&self, component: &Component) -> usize {
        // No attribute cell: every cell of a row holds a character.
        usize::from(component.at.columns).max(1)
    }

    fn paint(&mut self, before: Option<&Component>, now: Option<&Shown<'_>>) -> Result<(), Reply> {
        let Some(handle) = now.map(|shown| shown.component.handle).or_else(|| before.map(|component| component.handle)) else { return Ok(()) };
        let own = self.host.context(self.context).map_or_else(String::new, |held| held.open.language.clone());
        let wants = match now {
            Some(shown) => Maker::new(shown, &own)?.make()?,
            None => Vec::new(),
        };
        let changed = self.change(handle, &wants);
        // An erasure the step had no room for is owed: the drawings are
        // deleted when the components are next drawn again.
        if changed.is_err() && now.is_none() {
            self.host.regis.ui.owed = true;
        }
        changed
    }

    fn part(&self, handle: i32) -> Option<(i32, usize)> {
        let number = handle.checked_rem(HANDLE_NUMBERS).and_then(|number| u16::try_from(number).ok()).filter(|number| *number != 0)?;
        self.host.regis.ui.nodes.iter().find(|node| node.number == number).and_then(|node| Some((node.component, usize::from(node.part?))))
    }
}

impl Host {
    /// A step begins on a ReGIS page: when a refused commit took the
    /// components' last changes with it, every component is drawn again
    /// as it now is, and what the refusal brought back of components
    /// since removed is deleted.
    pub(in crate::host) fn regis_ui_begin_step(&mut self) {
        if !self.regis.ui.owed || self.regis_available(ContextId::Page).is_err() {
            return;
        }
        let ui = core::mem::take(&mut self.ui);
        let mut painter = Components::new(self, ContextId::Page, ui.state_bytes());
        if painter.owed() {
            let handles = ui.redraw(&mut painter);
            painter.keep_only(&handles);
        }
        drop(painter);
        self.ui = ui;
    }
}

#[cfg(test)]
mod tests;
