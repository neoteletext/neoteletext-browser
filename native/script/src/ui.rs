//! The component vocabulary `nt.ui` (work package U1 of script-api.md;
//! section 8, operations 60 to 67 of section 10.2; DEC-7205): the part
//! that is the same on every renderer, and the opening each renderer's
//! own drawing plugs into.
//!
//! **What is shared** is what a viewer and a program rely on: the
//! thirteen kinds and their options ([`options`]), a component's view
//! state (the highlighted item, the part of a paragraph shown, a field's
//! text and caret), which component has the focus, what each key and
//! each press of the pointer means to each kind, and the events that
//! says. It is a retained tree of one level: a component is made once,
//! changed by its options, and reports events.
//!
//! **What is not shared** is every pixel and cell. A renderer draws a
//! component in its own terms, through [`Painter`]: teletext on the page
//! plane with attribute cells, the overlay and controls
//! (`teletext/ui.rs`); ReGIS and NAPLPS with their own drawings and
//! objects (work packages U2 and U3). Nothing here knows a cell's code,
//! a triplet, a drawing or a macro, and a renderer's drawing is refused
//! by that renderer's own bounds. All a painter must do is in the
//! trait's three calls.
//!
//! **Events and their defaults.** A key or a press is offered to a
//! component only after the script's own listeners for it have let it
//! through ([`Ui::offer_key`], [`Ui::offer_text`], [`Ui::offer_pointer`]).
//! What the component would do is then held as an intent and given to the
//! script as the component's event; the script's verdict comes back
//! ([`Ui::settle`]) and only then is the intent carried out, or dropped.
//! So every default action of a component can be cancelled or replaced,
//! as section 4.1 asks, and nothing a component does happens behind a
//! script's back.

pub mod limits;
pub mod options;

use std::fmt::Write as _;

use protocol::script::ContextId;

use self::limits::{COMPONENT_HANDLE_BASE, MAX_COMPONENTS, MAX_UI_NODES, MAX_UI_STATE_BYTES};
use self::options::{Area, Props, Value, quote};
use crate::host::Reply;
use crate::link::Context as _;

/// A `RangeError` naming `bound`.
fn range(bound: &str, detail: &str) -> Reply {
    Reply::Range(format!("{bound}: {detail}"))
}

/// A `TypeError`.
fn type_error(detail: &str) -> Reply {
    Reply::Type(detail.to_owned())
}

/// A kind of component: the first set of thirteen (section 8).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    /// A page's furniture: a title, a hint, the coloured prompts.
    Frame,
    /// One text.
    Text,
    /// A text over several rows, shown a part at a time.
    Paragraph,
    /// A band of colour with a text.
    Band,
    /// A text that navigates.
    Link,
    /// A text that is activated.
    Button,
    /// Items, one highlighted.
    Menu,
    /// Rows in columns under headings.
    Table,
    /// A line of text the viewer types.
    Field,
    /// A text that is on or off.
    Toggle,
    /// A value moved between two ends.
    Slider,
    /// How far something has got.
    Progress,
    /// The time, drawn by the host each second.
    Clock,
}

impl Kind {
    /// Every kind, in the order its number as a host call gives it.
    pub const ALL: [Self; 13] = [
        Self::Frame,
        Self::Text,
        Self::Paragraph,
        Self::Band,
        Self::Link,
        Self::Button,
        Self::Menu,
        Self::Table,
        Self::Field,
        Self::Toggle,
        Self::Slider,
        Self::Progress,
        Self::Clock,
    ];

    /// The kind numbered `number` in [`Kind::ALL`].
    #[must_use]
    pub fn of(number: i32) -> Option<Self> {
        usize::try_from(number).ok().and_then(|at| Self::ALL.get(at)).copied()
    }

    /// Its name: the call of `nt.ui` that makes one.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Frame => "frame",
            Self::Text => "text",
            Self::Paragraph => "paragraph",
            Self::Band => "band",
            Self::Link => "link",
            Self::Button => "button",
            Self::Menu => "menu",
            Self::Table => "table",
            Self::Field => "field",
            Self::Toggle => "toggle",
            Self::Slider => "slider",
            Self::Progress => "progress",
            Self::Clock => "clock",
        }
    }

    /// The keys the kind uses, by their W3C names, for a page's manifest
    /// to claim (`nt.ui.keys`). A field also needs the manifest's `text`.
    #[must_use]
    pub const fn keys(self) -> &'static [&'static str] {
        match self {
            Self::Frame | Self::Text | Self::Band | Self::Progress | Self::Clock => &[],
            Self::Paragraph => &[key::SPACE, key::PAGE_DOWN, key::PAGE_UP],
            Self::Link => &[key::ENTER],
            Self::Button | Self::Toggle => &[key::ENTER, key::SPACE],
            Self::Menu => &[key::UP, key::DOWN, key::HOME, key::END, key::ENTER],
            Self::Table => &[key::UP, key::DOWN, key::HOME, key::END, key::ENTER, key::PAGE_UP, key::PAGE_DOWN],
            Self::Field => &[key::ENTER, key::ESCAPE, key::BACKSPACE, key::LEFT, key::RIGHT],
            Self::Slider => &[key::LEFT, key::RIGHT],
        }
    }
}

/// The names of the keys components use (the W3C UI Events key values).
pub mod key {
    /// Moves the highlight up.
    pub const UP: &str = "ArrowUp";
    /// Moves the highlight down.
    pub const DOWN: &str = "ArrowDown";
    /// Moves a caret or a slider left.
    pub const LEFT: &str = "ArrowLeft";
    /// Moves a caret or a slider right.
    pub const RIGHT: &str = "ArrowRight";
    /// The first item.
    pub const HOME: &str = "Home";
    /// The last item.
    pub const END: &str = "End";
    /// Activates, or submits.
    pub const ENTER: &str = "Enter";
    /// Activates, or shows the next part.
    pub const SPACE: &str = " ";
    /// The part, or the rows, before.
    pub const PAGE_UP: &str = "PageUp";
    /// The part, or the rows, after.
    pub const PAGE_DOWN: &str = "PageDown";
    /// Cancels a field.
    pub const ESCAPE: &str = "Escape";
    /// Removes the character before a field's caret.
    pub const BACKSPACE: &str = "Backspace";
    /// Moves the focus, with Shift the other way.
    pub const TAB: &str = "Tab";
}

/// What a component's event says (section 8's table), by the number the
/// prelude is given it as.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(i32)]
pub enum What {
    /// The highlight would move.
    Move = 1,
    /// It was activated.
    Activate = 2,
    /// Its value would change.
    Change = 3,
    /// Another part would be shown.
    Page = 4,
    /// A field was submitted.
    Submit = 5,
    /// A field was cancelled.
    Cancel = 6,
}

/// What carrying out an intent changes.
#[derive(Clone, PartialEq, Eq, Debug)]
enum Effect {
    /// Nothing of the component.
    None,
    /// The highlighted item.
    Select(usize),
    /// The first line of a paragraph shown.
    Offset(usize),
    /// A field's value and its caret.
    Edit(String, usize),
    /// A slider's value.
    Number(i64),
    /// A toggle's state.
    On(bool),
}

/// What a component would do with a key or a press: its event, and the
/// change that is the event's default action.
#[derive(Clone, PartialEq, Eq, Debug)]
struct Intent {
    what: What,
    value: Value,
    /// Where the default action navigates, when it does.
    target: Option<String>,
    effect: Effect,
}

/// What a component made of what it was offered.
enum Offered {
    /// Nothing: the key is not one it uses now.
    Unused,
    /// It used it, with no event to report: a caret moved.
    Used,
    /// It would do this.
    Event(Intent),
}

/// A component's view state: the host's, never the script's to hold.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct View {
    /// The first item, or line, shown.
    pub top: usize,
    /// A field's caret, as characters before it.
    pub caret: usize,
}

/// One component.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Component {
    /// Its handle.
    pub handle: i32,
    /// Its kind.
    pub kind: Kind,
    /// The context that made it.
    pub context: ContextId,
    /// The scope it belongs to, or 0.
    pub scope: i32,
    /// Where it lies on the grid.
    pub at: Area,
    /// Its options.
    pub props: Props,
    /// Its view state.
    pub view: View,
}

/// The time a clock shows.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Time {
    /// Seconds since the epoch.
    pub epoch_seconds: u64,
    /// The viewer's zone, in minutes east of UTC.
    pub zone_minutes: i16,
}

/// Seconds in a minute, minutes in an hour, hours in a day.
const SIXTY: u64 = 60;
const HOURS: u64 = 24;

impl Time {
    /// The hour, minute and second, in UTC or the viewer's zone.
    #[must_use]
    pub fn parts(&self, utc: bool) -> (u8, u8, u8) {
        let zone = if utc { 0 } else { i64::from(self.zone_minutes).saturating_mul(60) };
        let day = i64::try_from(SIXTY.saturating_mul(SIXTY).saturating_mul(HOURS)).unwrap_or(i64::MAX);
        let local = i64::try_from(self.epoch_seconds).unwrap_or(i64::MAX).saturating_add(zone).checked_rem_euclid(day).unwrap_or(0);
        let seconds = u64::try_from(local).unwrap_or(0);
        let part = |value: u64| u8::try_from(value).unwrap_or(0);
        (
            part(seconds.checked_div(SIXTY.saturating_mul(SIXTY)).unwrap_or(0)),
            part(seconds.checked_div(SIXTY).unwrap_or(0).checked_rem(SIXTY).unwrap_or(0)),
            part(seconds.checked_rem(SIXTY).unwrap_or(0)),
        )
    }
}

/// A component as a renderer is to draw it.
#[derive(Clone, Copy, Debug)]
pub struct Shown<'a> {
    /// The component, with its options and view state.
    pub component: &'a Component,
    /// Whether it has its context's focus.
    pub focused: bool,
    /// The time, for a clock.
    pub time: Time,
}

/// A renderer's drawing of components: all a renderer gives the
/// vocabulary. Each renderer implements it on its own model and by its
/// own bounds; the vocabulary asks nothing else of it.
pub trait Painter {
    /// The cells of text one row of `component` holds as this renderer
    /// draws it, so that a paragraph is broken where its lines end.
    fn text_columns(&self, component: &Component) -> usize;

    /// Draws `now` in place of `before`: a new component has no `before`,
    /// and a removed one no `now`. All or nothing: when the renderer has
    /// no room for the drawing (its triplets, its drawings, its objects)
    /// it changes nothing and gives the refusal, which names its bound.
    ///
    /// # Errors
    /// The `RangeError` or `TypeError` of the renderer's own bound.
    fn paint(&mut self, before: Option<&Component>, now: Option<&Shown<'_>>) -> Result<(), Reply>;

    /// The component a thing of the renderer belongs to, and which of the
    /// component's parts it is (an item of a menu, a row of a table; 0
    /// where a component has one part): what the pointer hit, by the
    /// handle the decoder names it with.
    fn part(&self, handle: i32) -> Option<(i32, usize)>;
}

impl Component {
    /// Whether the focus can rest on it: it takes keys, and is enabled.
    #[must_use]
    pub fn focusable(&self) -> bool {
        let takes_keys = match self.kind {
            Kind::Table => self.props.selectable,
            kind => !kind.keys().is_empty(),
        };
        takes_keys && !self.props.disabled
    }

    /// The items a menu or a table has.
    #[must_use]
    pub fn count(&self) -> usize {
        match self.kind {
            Kind::Menu => self.props.items.len(),
            Kind::Table => self.props.rows.len(),
            _ => 0,
        }
    }

    /// The items, or lines, shown at once: the option `shown`, within
    /// the rows the area has (a table's first row is its headings, and a
    /// paragraph that does not fit keeps its last row for its prompt).
    #[must_use]
    pub fn visible(&self) -> usize {
        let rows = usize::from(self.at.rows);
        let room = match self.kind {
            Kind::Table => rows.saturating_sub(1),
            _ => rows,
        };
        if self.props.shown == 0 { room } else { self.props.shown.min(room) }.max(1)
    }

    /// Whether the highlight may rest on item `index`.
    fn enabled(&self, index: usize) -> bool {
        match self.kind {
            Kind::Menu => self.props.items.get(index).is_some_and(|item| !item.disabled),
            _ => index < self.count(),
        }
    }

    /// The enabled item nearest `from` going `by` at a time, or none.
    fn step(&self, from: usize, by: isize) -> Option<usize> {
        let mut at = from;
        loop {
            at = at.checked_add_signed(by).filter(|next| *next < self.count())?;
            if self.enabled(at) {
                return Some(at);
            }
        }
    }

    /// The first enabled item from `from` going `by` at a time, `from`
    /// itself included.
    fn settle_on(&self, from: usize, by: isize) -> Option<usize> {
        if self.enabled(from) { Some(from) } else { self.step(from, by) }
    }

    /// A paragraph's lines in `columns` cells: those given, or its text
    /// broken at spaces. A word longer than a line is cut across lines.
    #[must_use]
    pub fn lines(&self, columns: usize) -> Vec<String> {
        if let Some(lines) = &self.props.lines {
            return lines.clone();
        }
        let columns = columns.max(1);
        let mut lines = Vec::new();
        let mut line = String::new();
        let mut length = 0_usize;
        for word in self.props.text.split(' ').filter(|word| !word.is_empty()) {
            let mut word: Vec<char> = word.chars().collect();
            if length > 0 && length.saturating_add(1).saturating_add(word.len()) > columns {
                lines.push(core::mem::take(&mut line));
                length = 0;
            }
            while word.len() > columns {
                let rest = word.split_off(columns);
                lines.push(word.into_iter().collect());
                word = rest;
            }
            if length > 0 {
                line.push(' ');
                length = length.saturating_add(1);
            }
            length = length.saturating_add(word.len());
            line.extend(word);
        }
        if !line.is_empty() || lines.is_empty() {
            lines.push(line);
        }
        lines
    }

    /// The lines a paragraph shows at once when it has `total`: every row
    /// when they all fit, and one fewer, for the prompt, when not.
    #[must_use]
    pub fn page_lines(&self, total: usize) -> usize {
        let rows = usize::from(self.at.rows);
        if total <= rows { rows } else { rows.saturating_sub(1).max(1) }
    }

    /// The event's value for the item `index`: the item's own, or its
    /// place.
    fn item_value(&self, index: usize) -> Value {
        let place = Value::Number(i64::try_from(index).unwrap_or(0));
        match self.kind {
            Kind::Menu => self.props.items.get(index).and_then(|item| item.value.clone()).unwrap_or(place),
            _ => place,
        }
    }

    /// Activating the item `index`.
    fn activate_item(&self, index: usize) -> Offered {
        if !self.enabled(index) {
            return Offered::Unused;
        }
        let target = self.props.items.get(index).and_then(|item| item.target.clone());
        Offered::Event(Intent { what: What::Activate, value: self.item_value(index), target, effect: Effect::Select(index) })
    }

    /// Moving the highlight to `index`, when there is such an item.
    fn move_to(&self, index: Option<usize>) -> Offered {
        match index {
            Some(index) if index != self.props.selected => {
                Offered::Event(Intent { what: What::Move, value: self.item_value(index), target: None, effect: Effect::Select(index) })
            }
            // The key is the component's though the highlight is already
            // there: it is not the browser's to act on.
            _ => Offered::Used,
        }
    }

    /// What the key `name` means to the component. `lines` is a
    /// paragraph's lines as its renderer breaks them.
    fn key(&self, name: &str, lines: usize) -> Offered {
        if !self.kind.keys().contains(&name) || !self.focusable() {
            return Offered::Unused;
        }
        let props = &self.props;
        let simple = |what: What, value: Value, effect: Effect| Offered::Event(Intent { what, value, target: None, effect });
        match self.kind {
            Kind::Paragraph => {
                let page = self.page_lines(lines);
                let top = self.view.top;
                let next = if name == key::PAGE_UP {
                    top.saturating_sub(page)
                } else if top.saturating_add(page) < lines {
                    top.saturating_add(page)
                } else {
                    top
                };
                if next == top {
                    return Offered::Used;
                }
                simple(What::Page, Value::Number(i64::try_from(next).unwrap_or(0)), Effect::Offset(next))
            }
            Kind::Link => Offered::Event(Intent {
                what: What::Activate,
                value: Value::Text(props.target.clone()),
                target: Some(props.target.clone()).filter(|target| !target.is_empty()),
                effect: Effect::None,
            }),
            Kind::Button => simple(What::Activate, Value::Null, Effect::None),
            Kind::Toggle => simple(What::Change, Value::Bool(!props.on), Effect::On(!props.on)),
            Kind::Menu | Kind::Table => {
                let last = self.count().saturating_sub(1);
                let jump = isize::try_from(self.visible()).unwrap_or(1);
                match name {
                    key::UP => self.move_to(self.step(props.selected, -1)),
                    key::DOWN => self.move_to(self.step(props.selected, 1)),
                    key::HOME => self.move_to(self.settle_on(0, 1)),
                    key::END => self.move_to(self.settle_on(last, -1)),
                    key::PAGE_UP => self.move_to(self.settle_on(props.selected.saturating_add_signed(jump.saturating_neg()), 1)),
                    key::PAGE_DOWN => self.move_to(self.settle_on(props.selected.saturating_add_signed(jump).min(last), -1)),
                    _ => self.activate_item(props.selected),
                }
            }
            Kind::Slider => {
                let by = if name == key::LEFT { props.step.saturating_neg() } else { props.step };
                let next = props.number.saturating_add(by).clamp(props.min, props.max);
                if next == props.number {
                    return Offered::Used;
                }
                simple(What::Change, Value::Number(next), Effect::Number(next))
            }
            Kind::Field => {
                let value = Value::Text(props.value.clone());
                match name {
                    key::ENTER => simple(What::Submit, value, Effect::None),
                    key::ESCAPE => simple(What::Cancel, value, Effect::None),
                    key::BACKSPACE => {
                        let Some(before) = self.view.caret.checked_sub(1) else { return Offered::Used };
                        let edited: String = props.value.chars().enumerate().filter(|(at, _)| *at != before).map(|(_, character)| character).collect();
                        simple(What::Change, Value::Text(edited.clone()), Effect::Edit(edited, before))
                    }
                    // The caret's moves are the caller's to make at once:
                    // they report nothing.
                    _ => Offered::Used,
                }
            }
            Kind::Frame | Kind::Text | Kind::Band | Kind::Progress | Kind::Clock => Offered::Unused,
        }
    }

    /// What typed `text` means to a field: the characters it has room
    /// for, put in at the caret.
    fn typed(&self, text: &str) -> Offered {
        if self.kind != Kind::Field || !self.focusable() {
            return Offered::Unused;
        }
        let held = self.props.value.chars().count();
        let room = self.props.longest.saturating_sub(held);
        let typed: Vec<char> = text.chars().filter(|character| !character.is_control()).take(room).collect();
        if typed.is_empty() {
            return Offered::Used;
        }
        let caret = self.view.caret.min(held);
        let mut edited: Vec<char> = self.props.value.chars().collect();
        let after = edited.split_off(caret);
        let moved = caret.saturating_add(typed.len());
        edited.extend(typed);
        edited.extend(after);
        let edited: String = edited.into_iter().collect();
        Offered::Event(Intent { what: What::Change, value: Value::Text(edited.clone()), target: None, effect: Effect::Edit(edited, moved) })
    }

    /// What a release of the pointer on part `part` means.
    fn pressed(&self, part: usize, lines: usize) -> Offered {
        if self.props.disabled {
            return Offered::Unused;
        }
        match self.kind {
            Kind::Link | Kind::Button | Kind::Toggle => self.key(key::ENTER, lines),
            Kind::Paragraph => self.key(key::SPACE, lines),
            Kind::Menu => self.activate_item(part),
            Kind::Table if self.props.selectable => self.activate_item(part),
            // A press on a field or a slider gives it the focus, which
            // the caller has done.
            Kind::Field | Kind::Slider => Offered::Used,
            _ => Offered::Unused,
        }
    }

    /// Keeps the highlight among the items shown.
    fn follow(&mut self) {
        let visible = self.visible();
        let selected = self.props.selected;
        if selected < self.view.top {
            self.view.top = selected;
        } else if selected >= self.view.top.saturating_add(visible) {
            self.view.top = selected.saturating_add(1).saturating_sub(visible);
        }
        self.view.top = self.view.top.min(self.count().saturating_sub(visible));
    }

    /// Carries out `effect`.
    fn apply(&mut self, effect: &Effect) {
        match effect {
            Effect::None => {}
            Effect::Select(index) => {
                self.props.selected = *index;
                self.follow();
            }
            Effect::Offset(top) => self.view.top = *top,
            Effect::Edit(value, caret) => {
                self.props.value.clone_from(value);
                self.view.caret = *caret;
            }
            Effect::Number(number) => self.props.number = *number,
            Effect::On(on) => self.props.on = *on,
        }
    }

    /// The component's value, as `component.value` gives it: a menu's
    /// highlighted item's value, a table's highlighted row, a field's
    /// text, a toggle's state, a slider's or a progress bar's number, the
    /// first line a paragraph shows; null for the rest.
    #[must_use]
    pub fn value(&self) -> Value {
        match self.kind {
            Kind::Menu | Kind::Table if self.count() > 0 => self.item_value(self.props.selected),
            Kind::Field => Value::Text(self.props.value.clone()),
            Kind::Toggle => Value::Bool(self.props.on),
            Kind::Slider | Kind::Progress => Value::Number(self.props.number),
            Kind::Paragraph => Value::Number(i64::try_from(self.view.top).unwrap_or(0)),
            _ => Value::Null,
        }
    }
}

/// The components of a container's host.
#[derive(Default)]
pub struct Ui {
    /// The components, in the order they were made: the focus's order.
    list: Vec<Component>,
    /// The focused component of each context.
    focus: [Option<i32>; 2],
    /// Components made so far: the next handle's place above the base.
    made: i32,
    /// What a component would do, waiting for the script's verdict.
    pending: Option<(i32, Intent)>,
    /// The time clocks were last drawn for.
    time: Time,
}

impl Ui {
    /// The place among the components of the one `handle` names, when it
    /// is `context`'s (section 10.1 rule 4).
    fn place(&self, context: ContextId, handle: i32) -> Result<usize, Reply> {
        self.list.iter().position(|held| held.handle == handle && held.context == context).ok_or_else(|| type_error("no such thing"))
    }

    /// Whether `handle` names a component of `context`.
    #[must_use]
    pub fn owns(&self, context: ContextId, handle: i32) -> bool {
        self.place(context, handle).is_ok()
    }

    /// The focused component of `context`, or 0.
    #[must_use]
    pub fn focused(&self, context: ContextId) -> i32 {
        self.focus.get(context.index()).copied().flatten().unwrap_or(0)
    }

    /// Whether any component is held.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.list.is_empty()
    }

    /// The bytes held for every component.
    #[must_use]
    pub fn state_bytes(&self) -> usize {
        self.list.iter().fold(0_usize, |sum, held| sum.saturating_add(size_of::<Component>()).saturating_add(held.props.bytes()))
    }

    /// The items of every component.
    fn nodes(&self) -> usize {
        self.list.iter().fold(0_usize, |sum, held| sum.saturating_add(held.props.nodes()))
    }

    /// The components' entries of the budget report (section 3.7), each
    /// written after a comma.
    pub fn budget(&self, out: &mut String) {
        let _ = write!(
            out,
            ",\"components\":{{\"used\":{},\"limit\":{MAX_COMPONENTS}}},\"uiNodes\":{{\"used\":{},\"limit\":{MAX_UI_NODES}}}",
            self.list.len(),
            self.nodes()
        );
    }

    /// The component at `place` as a renderer is to draw it.
    fn shown(&self, place: usize) -> Option<Shown<'_>> {
        let component = self.list.get(place)?;
        Some(Shown { component, focused: self.focused(component.context) == component.handle, time: self.time })
    }

    /// Holds a component's bytes and items, with the others', to the
    /// vocabulary's bounds.
    fn room(&self, replacing: Option<usize>, props: &Props) -> Result<(), Reply> {
        let (was_bytes, was_nodes) = replacing.and_then(|place| self.list.get(place)).map_or((0, 0), |held| (held.props.bytes(), held.props.nodes()));
        if self.state_bytes().saturating_sub(was_bytes).saturating_add(props.bytes()) > MAX_UI_STATE_BYTES {
            return Err(range("MAX_UI_STATE_BYTES", &format!("the components' options are at most {MAX_UI_STATE_BYTES} bytes together")));
        }
        if self.nodes().saturating_sub(was_nodes).saturating_add(props.nodes()) > MAX_UI_NODES {
            return Err(range("MAX_UI_NODES", &format!("the components hold at most {MAX_UI_NODES} items together")));
        }
        Ok(())
    }

    /// `nt.ui.<kind>(options)` (operation `UI_CREATE`): the component is
    /// made and drawn, or nothing is. The first component of a context
    /// the focus can rest on takes it.
    ///
    /// # Errors
    /// The `RangeError` or `TypeError` the script is thrown: the
    /// vocabulary's own, or the renderer's for a drawing it has no room
    /// for.
    pub fn create(&mut self, painter: &mut dyn Painter, context: ContextId, scope: i32, kind: Kind, at: Area, options: &[u8]) -> Result<i32, Reply> {
        if self.list.len() >= MAX_COMPONENTS {
            return Err(range("MAX_COMPONENTS", &format!("a container has at most {MAX_COMPONENTS} components")));
        }
        let mut props = Props::default();
        let at = props.take(kind, &options::read(options)?)?.unwrap_or(at);
        self.room(None, &props)?;
        let handle = COMPONENT_HANDLE_BASE.checked_add(self.made).ok_or_else(|| type_error("no component handle is left"))?;
        let mut component = Component { handle, kind, context, scope, at, props, view: View::default() };
        component.view.caret = component.props.value.chars().count();
        component.follow();
        let takes_focus = component.focusable() && self.focused(context) == 0;
        painter.paint(None, Some(&Shown { component: &component, focused: takes_focus, time: self.time }))?;
        self.made = self.made.saturating_add(1);
        if takes_focus && let Some(slot) = self.focus.get_mut(context.index()) {
            *slot = Some(handle);
        }
        self.list.push(component);
        Ok(handle)
    }

    /// `component.set(options)` (operation `UI_SET`): the options given
    /// replace those it had, and it is drawn again, or nothing changes.
    ///
    /// # Errors
    /// As [`Ui::create`].
    pub fn set(&mut self, painter: &mut dyn Painter, context: ContextId, handle: i32, options: &[u8]) -> Result<(), Reply> {
        let place = self.place(context, handle)?;
        let Some(before) = self.list.get(place) else { return Err(type_error("no such thing")) };
        let mut after = before.clone();
        if let Some(at) = after.props.take(after.kind, &options::read(options)?)? {
            after.at = at;
        }
        self.room(Some(place), &after.props)?;
        after.view.caret = after.view.caret.min(after.props.value.chars().count());
        after.follow();
        // A component that can no longer hold the focus gives it up.
        let focused = self.focused(context) == handle && after.focusable();
        painter.paint(Some(before), Some(&Shown { component: &after, focused, time: self.time }))?;
        if !focused
            && self.focused(context) == handle
            && let Some(slot) = self.focus.get_mut(context.index())
        {
            *slot = None;
        }
        self.pending = self.pending.take().filter(|(waiting, _)| *waiting != handle);
        if let Some(slot) = self.list.get_mut(place) {
            *slot = after;
        }
        Ok(())
    }

    /// Forgets the components `gone` chooses, erasing each; their handles
    /// are given. An erasure draws nothing, so no renderer refuses one.
    fn forget(&mut self, painter: Option<&mut dyn Painter>, gone: impl Fn(&Component) -> bool) -> Vec<i32> {
        let (removed, kept): (Vec<Component>, Vec<Component>) = core::mem::take(&mut self.list).into_iter().partition(|held| gone(held));
        self.list = kept;
        if let Some(painter) = painter {
            for component in &removed {
                let _ = painter.paint(Some(component), None);
            }
        }
        let handles: Vec<i32> = removed.iter().map(|component| component.handle).collect();
        for slot in &mut self.focus {
            *slot = slot.filter(|focused| !handles.contains(focused));
        }
        self.pending = self.pending.take().filter(|(waiting, _)| !handles.contains(waiting));
        handles
    }

    /// `component.remove()` (operation `UI_REMOVE`).
    ///
    /// # Errors
    /// A `TypeError` for a handle that names no component of `context`.
    pub fn remove(&mut self, painter: &mut dyn Painter, context: ContextId, handle: i32) -> Result<(), Reply> {
        self.place(context, handle)?;
        self.forget(Some(painter), |held| held.handle == handle);
        Ok(())
    }

    /// The scope `scope` of `context` ends: its components are removed,
    /// and erased when their renderer's state is still there.
    pub fn remove_scope(&mut self, painter: Option<&mut dyn Painter>, context: ContextId, scope: i32) -> Vec<i32> {
        self.forget(painter, |held| held.context == context && held.scope == scope)
    }

    /// `context` ends: its components go with it, and with the page they
    /// were drawn on.
    pub fn remove_context(&mut self, context: ContextId) {
        self.forget(None, |held| held.context == context);
    }

    /// Moves the focus of `context` to the component at `to` (or to
    /// none), drawing the one that had it and the one that takes it. A
    /// renderer that has no room to draw the change leaves a component
    /// drawn as it was; the focus moves all the same, since it is where
    /// keys go.
    fn move_focus(&mut self, painter: &mut dyn Painter, context: ContextId, to: Option<i32>) {
        let from = self.focus.get(context.index()).copied().flatten();
        if from == to {
            return;
        }
        if let Some(slot) = self.focus.get_mut(context.index()) {
            *slot = to;
        }
        for handle in [from, to].into_iter().flatten() {
            if let Ok(place) = self.place(context, handle)
                && let (Some(before), Some(shown)) = (self.list.get(place), self.shown(place))
            {
                let _ = painter.paint(Some(before), Some(&shown));
            }
        }
    }

    /// `component.focus()` (operation `UI_FOCUS`).
    ///
    /// # Errors
    /// A `TypeError` for a handle that names no component of `context`,
    /// and for a component the focus cannot rest on.
    pub fn focus(&mut self, painter: &mut dyn Painter, context: ContextId, handle: i32) -> Result<(), Reply> {
        let place = self.place(context, handle)?;
        if !self.list.get(place).is_some_and(Component::focusable) {
            return Err(type_error("the focus rests only on an enabled component that takes keys"));
        }
        self.move_focus(painter, context, Some(handle));
        Ok(())
    }

    /// `component.value` (operation `UI_VALUE`), as JSON text.
    ///
    /// # Errors
    /// A `TypeError` for a handle that names no component of `context`.
    pub fn value(&self, context: ContextId, handle: i32) -> Result<String, Reply> {
        let place = self.place(context, handle)?;
        let mut out = String::new();
        if let Some(component) = self.list.get(place) {
            component.value().write(&mut out);
        }
        Ok(out)
    }

    /// What an offer came to, as the JSON text the prelude is given: the
    /// component's handle, and when it has an event to report, which
    /// event, its value and where its default navigates. The intent then
    /// waits for [`Ui::settle`].
    fn answer(&mut self, handle: i32, offered: Offered) -> Option<String> {
        let mut out = format!("{{\"h\":{handle}");
        match offered {
            Offered::Unused => return None,
            Offered::Used => {}
            Offered::Event(intent) => {
                let _ = write!(out, ",\"what\":{},\"value\":", intent.what as i32);
                intent.value.write(&mut out);
                if let Some(target) = &intent.target {
                    out.push_str(",\"target\":");
                    quote(&mut out, target);
                }
                self.pending = Some((handle, intent));
            }
        }
        out.push('}');
        Some(out)
    }

    /// The component after the focused one in the order they were made,
    /// or before it, that the focus can rest on; around the ends.
    fn neighbour(&self, context: ContextId, backwards: bool) -> Option<i32> {
        let own: Vec<&Component> = self.list.iter().filter(|held| held.context == context && held.focusable()).collect();
        let at = own.iter().position(|held| held.handle == self.focused(context));
        let count = own.len();
        let next = match (at, backwards) {
            (None, false) => 0,
            (None, true) => count.checked_sub(1)?,
            (Some(at), false) => at.saturating_add(1).checked_rem(count)?,
            (Some(at), true) => at.checked_sub(1).unwrap_or(count.saturating_sub(1)),
        };
        own.get(next).map(|held| held.handle)
    }

    /// A paragraph's lines, as `painter` breaks them.
    fn lines(painter: &dyn Painter, component: &Component) -> usize {
        if component.kind == Kind::Paragraph { component.lines(painter.text_columns(component)).len() } else { 0 }
    }

    /// Offers the key `name` to the focused component of `context`, once
    /// the script's listeners for `keydown` have let it through
    /// (operation `UI_OFFER`). None when no component uses the key, which
    /// is then the browser's as before. Tab, with Shift the other way,
    /// moves the focus; a field's caret moves at once.
    pub fn offer_key(&mut self, painter: &mut dyn Painter, context: ContextId, name: &str, shift: bool) -> Option<String> {
        if name == key::TAB {
            let to = self.neighbour(context, shift)?;
            self.move_focus(painter, context, Some(to));
            return self.answer(to, Offered::Used);
        }
        let handle = self.focused(context);
        let place = self.place(context, handle).ok()?;
        let component = self.list.get(place)?;
        let offered = component.key(name, Self::lines(painter, component));
        if component.kind == Kind::Field && matches!(offered, Offered::Used) && (name == key::LEFT || name == key::RIGHT) {
            let length = component.props.value.chars().count();
            let caret = if name == key::LEFT { component.view.caret.saturating_sub(1) } else { component.view.caret.saturating_add(1).min(length) };
            let mut moved = component.clone();
            moved.view.caret = caret;
            if painter.paint(Some(component), Some(&Shown { component: &moved, focused: true, time: self.time })).is_ok()
                && let Some(slot) = self.list.get_mut(place)
            {
                *slot = moved;
            }
        }
        self.answer(handle, offered)
    }

    /// Offers typed text to the focused component of `context`.
    pub fn offer_text(&mut self, context: ContextId, text: &str) -> Option<String> {
        let handle = self.focused(context);
        let place = self.place(context, handle).ok()?;
        let offered = self.list.get(place)?.typed(text);
        self.answer(handle, offered)
    }

    /// Says which component of `context` the renderer's thing `thing`
    /// belongs to, for a pointer event's target, and for a release what
    /// the component would do. A press gives the component the focus when
    /// it can rest there.
    pub fn offer_pointer(&mut self, painter: &mut dyn Painter, context: ContextId, thing: i32, pressed: bool, released: bool) -> Option<String> {
        let (handle, part) = painter.part(thing)?;
        let place = self.place(context, handle).ok()?;
        let component = self.list.get(place)?;
        if !pressed && !released {
            return self.answer(handle, Offered::Used);
        }
        if pressed && component.focusable() {
            self.move_focus(painter, context, Some(handle));
        }
        let component = self.list.get(place)?;
        let offered = if released { component.pressed(part, Self::lines(painter, component)) } else { Offered::Used };
        // A press a component makes nothing of is still on the component.
        let offered = if matches!(offered, Offered::Unused) { Offered::Used } else { offered };
        self.answer(handle, offered)
    }

    /// The script's verdict on the event of the component `handle`
    /// (operation `UI_SETTLE`): with `go` the intent that waited is
    /// carried out and drawn, otherwise dropped. False when the renderer
    /// had no room to draw the change, which is then not made.
    ///
    /// # Errors
    /// A `TypeError` for a handle that names no component of `context`.
    pub fn settle(&mut self, painter: &mut dyn Painter, context: ContextId, handle: i32, go: bool) -> Result<bool, Reply> {
        let place = self.place(context, handle)?;
        let Some((_, intent)) = self.pending.take().filter(|(waiting, _)| *waiting == handle) else { return Ok(false) };
        if !go || intent.effect == Effect::None {
            return Ok(go);
        }
        let Some(before) = self.list.get(place) else { return Ok(false) };
        let mut after = before.clone();
        after.apply(&intent.effect);
        let focused = self.focused(context) == handle;
        if painter.paint(Some(before), Some(&Shown { component: &after, focused, time: self.time })).is_err() {
            return Ok(false);
        }
        if let Some(slot) = self.list.get_mut(place) {
            *slot = after;
        }
        Ok(true)
    }

    /// Tells the components the time: a clock whose text it changes is
    /// drawn again (section 8: "drawn by the host each second").
    pub fn set_time(&mut self, painter: Option<&mut dyn Painter>, time: Time) {
        let before = core::mem::replace(&mut self.time, time);
        let Some(painter) = painter.filter(|_| before != time) else { return };
        for place in 0..self.list.len() {
            if let Some(shown) = self.shown(place).filter(|shown| shown.component.kind == Kind::Clock) {
                let _ = painter.paint(Some(shown.component), Some(&shown));
            }
        }
    }

    /// Draws every component again as it now is, and gives the handles
    /// of the components there are: for a renderer that lost what it had
    /// drawn of them (ReGIS, whose refused commit takes a step's changes
    /// with it). A component the renderer has no room for stays as it
    /// was drawn.
    pub fn redraw(&self, painter: &mut dyn Painter) -> Vec<i32> {
        for place in 0..self.list.len() {
            if let Some(shown) = self.shown(place) {
                let _ = painter.paint(Some(shown.component), Some(&shown));
            }
        }
        self.list.iter().map(|held| held.handle).collect()
    }

    /// Whether `context` has any component.
    #[must_use]
    pub fn has_any(&self, context: ContextId) -> bool {
        self.list.iter().any(|held| held.context == context)
    }

    /// Whether a clock is held, so that the host is woken each second.
    #[must_use]
    pub fn has_clock(&self, context: ContextId) -> bool {
        self.list.iter().any(|held| held.kind == Kind::Clock && held.context == context)
    }
}

/// The names of the keys the kind numbered `kind` uses, as JSON text
/// (operation `UI_KEYS`).
///
/// # Errors
/// A `TypeError` for a number that is no kind's.
pub fn keys_json(kind: i32) -> Result<String, Reply> {
    let kind = Kind::of(kind).ok_or_else(|| type_error("no such kind of component"))?;
    let mut out = String::from("[");
    for (at, name) in kind.keys().iter().enumerate() {
        if at > 0 {
            out.push(',');
        }
        quote(&mut out, name);
    }
    out.push(']');
    Ok(out)
}

#[cfg(test)]
mod tests;
