//! A component's options (script-api.md section 8): read from the strict,
//! bounded JSON a script's call carries, and held as the component's
//! properties. A script's options are outside input: the text is held to
//! [`MAX_UI_OPTION_BYTES`] before it is read, the engine's own pull reader
//! (`neoteletext_web::json`, RFC 8259) checks the whole grammar and
//! bounds the nesting, the values are counted against
//! [`MAX_UI_OPTION_VALUES`], and a member the kind does not have, a
//! member given twice, a wrong type or a value out of range refuses the
//! call whole, by name.

use neoteletext_web::json::{JsonReader, JsonStr, Token, integer};

use super::limits::{
    GRID_COLUMNS, GRID_FIRST_ROW, GRID_LAST_ROW, MAX_FIELD_CHARS, MAX_PROMPTS, MAX_TABLE_COLUMNS, MAX_UI_ID_BYTES, MAX_UI_ITEMS, MAX_UI_NUMBER,
    MAX_UI_OPTION_BYTES, MAX_UI_OPTION_VALUES,
};
use super::{Kind, range, type_error};
use crate::host::Reply;
use crate::limits::{MAX_LABEL_BYTES, MAX_URL_BYTES};

/// A JSON value of a script's options.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Value {
    /// `null`.
    Null,
    /// `true` or `false`.
    Bool(bool),
    /// A whole number.
    Number(i64),
    /// A string.
    Text(String),
    /// An array.
    List(Vec<Self>),
    /// An object: its members in the order written, no name twice.
    Object(Vec<(String, Self)>),
}

impl Value {
    /// The value as JSON text.
    pub fn write(&self, out: &mut String) {
        use core::fmt::Write as _;
        match self {
            Self::Null => out.push_str("null"),
            Self::Bool(flag) => out.push_str(if *flag { "true" } else { "false" }),
            Self::Number(number) => {
                let _ = write!(out, "{number}");
            }
            Self::Text(text) => quote(out, text),
            Self::List(items) => {
                out.push('[');
                for (at, item) in items.iter().enumerate() {
                    if at > 0 {
                        out.push(',');
                    }
                    item.write(out);
                }
                out.push(']');
            }
            Self::Object(members) => {
                out.push('{');
                for (at, (name, value)) in members.iter().enumerate() {
                    if at > 0 {
                        out.push(',');
                    }
                    quote(out, name);
                    out.push(':');
                    value.write(out);
                }
                out.push('}');
            }
        }
    }
}

/// `text` as a JSON string: the two characters JSON must escape and every
/// control character escaped.
pub fn quote(out: &mut String, text: &str) {
    use core::fmt::Write as _;
    out.push('"');
    for character in text.chars() {
        match character {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            control if control.is_control() => {
                let _ = write!(out, "\\u{:04x}", u32::from(control));
            }
            other => out.push(other),
        }
    }
    out.push('"');
}

fn decoded(text: JsonStr<'_>) -> Result<String, &'static str> {
    let mut buffer = vec![0_u8; text.raw_len()];
    let length = text.decode_into(&mut buffer).map_err(|_| "a string that is not well formed")?;
    buffer.truncate(length);
    String::from_utf8(buffer).map_err(|_| "a string that is not well formed")
}

/// The value that begins with `first`. The engine's reader bounds the
/// nesting, and so this function's own.
fn value(reader: &mut JsonReader<'_>, first: Token<'_>, room: &mut usize) -> Result<Value, &'static str> {
    const NOT_JSON: &str = "text that is not JSON";
    *room = room.checked_sub(1).ok_or("more values than a component's options hold")?;
    match first {
        Token::Null => Ok(Value::Null),
        Token::Bool(flag) => Ok(Value::Bool(flag)),
        Token::Number(number) => integer(number).map(Value::Number).ok_or("a number that is not a whole number"),
        Token::Str(text) => decoded(text).map(Value::Text),
        Token::BeginArray => {
            let mut items = Vec::new();
            loop {
                match reader.next_token().map_err(|_| NOT_JSON)? {
                    Some(Token::EndArray) => return Ok(Value::List(items)),
                    Some(token) => items.push(value(reader, token, room)?),
                    None => return Err(NOT_JSON),
                }
            }
        }
        Token::BeginObject => {
            let mut members: Vec<(String, Value)> = Vec::new();
            loop {
                match reader.next_token().map_err(|_| NOT_JSON)? {
                    Some(Token::EndObject) => return Ok(Value::Object(members)),
                    Some(Token::Key(key)) => {
                        let name = decoded(key)?;
                        if members.iter().any(|(held, _)| *held == name) {
                            return Err("a member given twice");
                        }
                        let token = reader.next_token().map_err(|_| NOT_JSON)?.ok_or(NOT_JSON)?;
                        members.push((name, value(reader, token, room)?));
                    }
                    _ => return Err(NOT_JSON),
                }
            }
        }
        Token::EndArray | Token::EndObject | Token::Key(_) => Err(NOT_JSON),
    }
}

/// Reads a call's options: one JSON object of at most
/// [`MAX_UI_OPTION_BYTES`] bytes and [`MAX_UI_OPTION_VALUES`] values, as
/// its members.
///
/// # Errors
/// `RangeError MAX_UI_OPTION_BYTES` for a longer text, and a `TypeError`
/// saying what was wrong for anything else.
pub fn read(bytes: &[u8]) -> Result<Vec<(String, Value)>, Reply> {
    if bytes.len() > MAX_UI_OPTION_BYTES {
        return Err(range("MAX_UI_OPTION_BYTES", &format!("a component's options are at most {MAX_UI_OPTION_BYTES} bytes of JSON, not {}", bytes.len())));
    }
    let wrong = |what: &str| type_error(&format!("a component's options hold {what}"));
    let mut reader = JsonReader::new(bytes).map_err(|_| wrong("text that is not UTF-8"))?;
    let first = reader.next_token().map_err(|_| wrong("text that is not JSON"))?.ok_or_else(|| wrong("nothing"))?;
    let mut room = MAX_UI_OPTION_VALUES;
    let read = value(&mut reader, first, &mut room).map_err(wrong)?;
    if !matches!(reader.next_token(), Ok(None)) {
        return Err(wrong("text that is not JSON"));
    }
    match read {
        Value::Object(members) => Ok(members),
        _ => Err(type_error("a component's options are an object")),
    }
}

/// Where a component lies on the grid of 40 columns by rows 1 to 23.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Area {
    /// Its top row, 1 to 23.
    pub row: u8,
    /// Its first column, 0 to 39.
    pub column: u8,
    /// Cells across, at least 1.
    pub columns: u8,
    /// Rows down, at least 1.
    pub rows: u8,
}

impl Area {
    /// The area of four whole numbers, held to the grid.
    ///
    /// # Errors
    /// `RangeError UI_GRID_ROWS` or `UI_GRID_COLUMNS`.
    pub fn new(row: i64, column: i64, columns: i64, rows: i64) -> Result<Self, Reply> {
        let (first, last, across) = (i64::from(GRID_FIRST_ROW), i64::from(GRID_LAST_ROW), i64::from(GRID_COLUMNS));
        if !(first..=last).contains(&row) || rows < 1 || rows > last.saturating_sub(row).saturating_add(1) {
            return Err(range("UI_GRID_ROWS", &format!("a component lies on rows {first} to {last}; {rows} rows from row {row} do not")));
        }
        if !(0..across).contains(&column) || columns < 1 || columns > across.saturating_sub(column) {
            return Err(range(
                "UI_GRID_COLUMNS",
                &format!("a component lies on columns 0 to {}; {columns} columns from column {column} do not", across.saturating_sub(1)),
            ));
        }
        let byte = |value: i64| u8::try_from(value).unwrap_or(u8::MAX);
        Ok(Self { row: byte(row), column: byte(column), columns: byte(columns), rows: byte(rows) })
    }
}

/// How a text lies in its columns.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Align {
    /// From the first column.
    Left,
    /// In the middle.
    Center,
    /// To the last column.
    Right,
}

/// What is done with a text longer than its columns.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Fit {
    /// The call is refused.
    Refuse,
    /// The text is drawn smaller where the renderer can, and cut where it
    /// cannot.
    Shrink,
    /// The text is cut.
    Cut,
}

/// One item of a menu.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Item {
    /// What it shows.
    pub text: String,
    /// What its events carry: a string or a whole number; its place when
    /// it names none.
    pub value: Option<Value>,
    /// Where activating it navigates, as `nt.navigate` takes it.
    pub target: Option<String>,
    /// Whether the highlight passes over it.
    pub disabled: bool,
}

/// One column of a table.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Column {
    /// Its heading.
    pub title: String,
    /// Its cells across.
    pub width: u8,
    /// How its texts lie.
    pub align: Align,
}

/// A colour a script names for a component: an entry of the renderer, as
/// a whole number, which the renderer's own drawing holds to its range.
/// The eight names every renderer has are their numbers, 0 to 7.
pub type Colour = u8;

/// A component's properties: every option of section 8's table, each
/// with the value it has when a script names none. A kind reads only its
/// own.
#[derive(Clone, PartialEq, Eq, Debug)]
#[expect(clippy::struct_excessive_bools, reason = "each is one option of section 8's table, on or off")]
pub struct Props {
    /// For the script.
    pub id: String,
    /// Spoken by a screen reader.
    pub label: String,
    /// Takes no key, pointer or focus.
    pub disabled: bool,
    /// The colour of its text, when the script names one.
    pub ink: Option<Colour>,
    /// The colour behind it, when the script names one.
    pub paper: Option<Colour>,
    /// The language its text is in, a tag of the engine's list; empty
    /// for the page's.
    pub language: String,
    /// A frame's title.
    pub title: String,
    /// A frame's hint.
    pub hint: String,
    /// A frame's coloured prompts, at most [`MAX_PROMPTS`].
    pub prompts: Vec<String>,
    /// A frame with no rule.
    pub bare: bool,
    /// The text of a text, paragraph, band, link, button, toggle or
    /// progress.
    pub text: String,
    /// A text's alignment.
    pub align: Align,
    /// A text's height, 1 or 2.
    pub height: u8,
    /// What a text does when it is longer than its columns.
    pub fit: Fit,
    /// A paragraph's lines, given already broken; none to break `text`.
    pub lines: Option<Vec<String>>,
    /// A paragraph's prompt when it does not fit.
    pub more: String,
    /// A link's target.
    pub target: String,
    /// A menu's items.
    pub items: Vec<Item>,
    /// A menu's or a table's highlighted item.
    pub selected: usize,
    /// A menu's or a table's rows visible; 0 for as many as its area has.
    pub shown: usize,
    /// A table's columns.
    pub columns: Vec<Column>,
    /// A table's rows.
    pub rows: Vec<Vec<String>>,
    /// Whether a table's rows take the highlight.
    pub selectable: bool,
    /// A field's value.
    pub value: String,
    /// The longest a field's value may be, in characters.
    pub longest: usize,
    /// What an empty field shows.
    pub placeholder: String,
    /// Whether a field shows its characters.
    pub masked: bool,
    /// A toggle's state.
    pub on: bool,
    /// A slider's least value.
    pub min: i64,
    /// A slider's or a progress bar's greatest value.
    pub max: i64,
    /// A slider's step.
    pub step: i64,
    /// A slider's or a progress bar's value.
    pub number: i64,
    /// Whether a clock shows seconds.
    pub seconds: bool,
    /// Whether a clock shows UTC and not the viewer's zone.
    pub utc: bool,
}

/// The default of `longest`.
const DEFAULT_LONGEST: usize = 32;
/// The default of a slider's and a progress bar's `max`.
const DEFAULT_MAX: i64 = 100;

impl Default for Props {
    fn default() -> Self {
        Self {
            id: String::new(),
            label: String::new(),
            disabled: false,
            ink: None,
            paper: None,
            language: String::new(),
            title: String::new(),
            hint: String::new(),
            prompts: Vec::new(),
            bare: false,
            text: String::new(),
            align: Align::Left,
            height: 1,
            fit: Fit::Refuse,
            lines: None,
            more: "More".to_owned(),
            target: String::new(),
            items: Vec::new(),
            selected: 0,
            shown: 0,
            columns: Vec::new(),
            rows: Vec::new(),
            selectable: false,
            value: String::new(),
            longest: DEFAULT_LONGEST,
            placeholder: String::new(),
            masked: false,
            on: false,
            min: 0,
            max: DEFAULT_MAX,
            step: 1,
            number: 0,
            seconds: false,
            utc: false,
        }
    }
}

/// The options each kind has beside the common ones (section 8's table).
const fn own_options(kind: Kind) -> &'static [&'static str] {
    match kind {
        Kind::Frame => &["title", "hint", "prompts", "bare"],
        Kind::Text => &["text", "align", "height", "fit"],
        Kind::Paragraph => &["text", "lines", "more"],
        Kind::Band | Kind::Button => &["text"],
        Kind::Link => &["text", "target"],
        Kind::Menu => &["items", "selected", "shown"],
        Kind::Table => &["columns", "rows", "selectable", "selected", "shown"],
        Kind::Field => &["value", "longest", "placeholder", "masked"],
        Kind::Toggle => &["text", "on"],
        Kind::Slider => &["min", "max", "step", "value"],
        Kind::Progress => &["value", "max", "text"],
        Kind::Clock => &["format", "zone"],
    }
}

/// The options every kind has.
const COMMON_OPTIONS: [&str; 7] = ["id", "label", "disabled", "ink", "paper", "language", "at"];
/// The eight colours every renderer names, by their numbers 0 to 7.
const COLOUR_NAMES: [&str; 8] = ["black", "red", "green", "yellow", "blue", "magenta", "cyan", "white"];

fn text_of(name: &str, value: &Value, most: usize, bound: &str) -> Result<String, Reply> {
    let Value::Text(text) = value else { return Err(type_error(&format!("{name} is a string"))) };
    if text.len() > most {
        return Err(range(bound, &format!("{name} is at most {most} bytes of UTF-8, not {}", text.len())));
    }
    if text.chars().any(char::is_control) {
        return Err(type_error(&format!("{name} holds no control character")));
    }
    Ok(text.clone())
}

fn flag_of(name: &str, value: &Value) -> Result<bool, Reply> {
    match value {
        Value::Bool(flag) => Ok(*flag),
        _ => Err(type_error(&format!("{name} is true or false"))),
    }
}

fn number_of(name: &str, value: &Value, low: i64, high: i64, bound: &str) -> Result<i64, Reply> {
    let Value::Number(number) = value else { return Err(type_error(&format!("{name} is a whole number"))) };
    if !(low..=high).contains(number) {
        return Err(range(bound, &format!("{name} is {low} to {high}, not {number}")));
    }
    Ok(*number)
}

fn count_of(name: &str, value: &Value, high: usize, bound: &str) -> Result<usize, Reply> {
    let high = i64::try_from(high).unwrap_or(i64::MAX);
    number_of(name, value, 0, high, bound).map(|number| usize::try_from(number).unwrap_or(0))
}

fn list_of<'a>(name: &str, value: &'a Value, most: usize, bound: &str) -> Result<&'a [Value], Reply> {
    let Value::List(items) = value else { return Err(type_error(&format!("{name} is an array"))) };
    if items.len() > most {
        return Err(range(bound, &format!("{name} holds at most {most}, not {}", items.len())));
    }
    Ok(items)
}

fn colour_of(name: &str, value: &Value) -> Result<Option<Colour>, Reply> {
    match value {
        Value::Null => Ok(None),
        Value::Text(colour) => COLOUR_NAMES
            .iter()
            .position(|held| held == colour)
            .map(|at| Some(u8::try_from(at).unwrap_or(0)))
            .ok_or_else(|| type_error(&format!("{name} is an entry of the renderer's colours or one of {}", COLOUR_NAMES.join(", ")))),
        other => number_of(name, other, 0, i64::from(u8::MAX), "UI_COLOURS").map(|entry| Some(u8::try_from(entry).unwrap_or(0))),
    }
}

fn align_of(name: &str, value: &Value) -> Result<Align, Reply> {
    match value {
        Value::Text(align) if align == "left" => Ok(Align::Left),
        Value::Text(align) if align == "center" => Ok(Align::Center),
        Value::Text(align) if align == "right" => Ok(Align::Right),
        _ => Err(type_error(&format!("{name} is \"left\", \"center\" or \"right\""))),
    }
}

/// The members of an object, each of which `known` names.
fn members_of<'a>(what: &str, value: &'a Value, known: &[&str]) -> Result<&'a [(String, Value)], Reply> {
    let Value::Object(members) = value else { return Err(type_error(&format!("{what} is an object"))) };
    match members.iter().find(|(name, _)| !known.contains(&name.as_str())) {
        Some((name, _)) => Err(type_error(&format!("{what} has no member {name:?}"))),
        None => Ok(members),
    }
}

fn item_of(value: &Value) -> Result<Item, Reply> {
    let mut item = Item { text: String::new(), value: None, target: None, disabled: false };
    for (name, value) in members_of("a menu's item", value, &["text", "value", "target", "disabled"])? {
        match name.as_str() {
            "text" => item.text = text_of("an item's text", value, MAX_UI_OPTION_BYTES, "MAX_UI_OPTION_BYTES")?,
            "value" => match value {
                Value::Text(_) | Value::Number(_) => item.value = Some(value.clone()),
                _ => return Err(type_error("an item's value is a string or a whole number")),
            },
            "target" => item.target = Some(text_of("an item's target", value, MAX_URL_BYTES, "MAX_URL_BYTES")?),
            _ => item.disabled = flag_of("an item's disabled", value)?,
        }
    }
    Ok(item)
}

fn column_of(value: &Value) -> Result<Column, Reply> {
    let mut column = Column { title: String::new(), width: 0, align: Align::Left };
    for (name, value) in members_of("a table's column", value, &["title", "width", "align"])? {
        match name.as_str() {
            "title" => column.title = text_of("a column's title", value, MAX_UI_OPTION_BYTES, "MAX_UI_OPTION_BYTES")?,
            "width" => column.width = u8::try_from(number_of("a column's width", value, 1, i64::from(GRID_COLUMNS), "UI_GRID_COLUMNS")?).unwrap_or(1),
            _ => column.align = align_of("a column's align", value)?,
        }
    }
    if column.width == 0 {
        return Err(type_error("a table's column has a width"));
    }
    Ok(column)
}

impl Props {
    /// Takes the options `members` of a component of `kind`, each
    /// checked, and gives the area they name when they name one. A
    /// refusal may leave some options taken: the caller works on a copy.
    ///
    /// # Errors
    /// The `RangeError` or `TypeError` the script is thrown.
    pub fn take(&mut self, kind: Kind, members: &[(String, Value)]) -> Result<Option<Area>, Reply> {
        let mut area = None;
        let long = |name: &str, value: &Value| text_of(name, value, MAX_UI_OPTION_BYTES, "MAX_UI_OPTION_BYTES");
        for (name, value) in members {
            let name = name.as_str();
            if !COMMON_OPTIONS.contains(&name) && !own_options(kind).contains(&name) {
                return Err(type_error(&format!("a {} has no option {name:?}", kind.name())));
            }
            match name {
                "id" => self.id = text_of(name, value, MAX_UI_ID_BYTES, "MAX_UI_ID_BYTES")?,
                "label" => self.label = text_of(name, value, MAX_LABEL_BYTES, "MAX_LABEL_BYTES")?,
                "disabled" => self.disabled = flag_of(name, value)?,
                "ink" => self.ink = colour_of(name, value)?,
                "paper" => self.paper = colour_of(name, value)?,
                "language" => self.language = text_of(name, value, MAX_UI_ID_BYTES, "MAX_UI_ID_BYTES")?,
                "at" => {
                    let mut numbers = [None; 4];
                    for (member, value) in members_of("at", value, &["row", "column", "columns", "rows"])? {
                        let slot = ["row", "column", "columns", "rows"].iter().position(|held| held == member).and_then(|at| numbers.get_mut(at));
                        if let (Some(slot), Value::Number(number)) = (slot, value) {
                            *slot = Some(*number);
                        } else {
                            return Err(type_error("at is { row, column, columns, rows }, each a whole number"));
                        }
                    }
                    // One row, when the script names none.
                    let [Some(row), Some(column), Some(columns), rows] = numbers else {
                        return Err(type_error("at is { row, column, columns, rows }, each a whole number"));
                    };
                    area = Some(Area::new(row, column, columns, rows.unwrap_or(1))?);
                }
                "title" => self.title = long(name, value)?,
                "hint" => self.hint = long(name, value)?,
                "prompts" => {
                    self.prompts =
                        list_of(name, value, MAX_PROMPTS, "MAX_UI_PROMPTS")?.iter().map(|prompt| long("a prompt", prompt)).collect::<Result<_, _>>()?;
                }
                "bare" => self.bare = flag_of(name, value)?,
                "text" => self.text = long(name, value)?,
                "align" => self.align = align_of(name, value)?,
                "height" => self.height = u8::try_from(number_of(name, value, 1, 2, "UI_TEXT_HEIGHT")?).unwrap_or(1),
                "fit" => {
                    self.fit = match value {
                        Value::Text(fit) if fit == "refuse" => Fit::Refuse,
                        Value::Text(fit) if fit == "shrink" => Fit::Shrink,
                        Value::Text(fit) if fit == "cut" => Fit::Cut,
                        _ => return Err(type_error("fit is \"refuse\", \"shrink\" or \"cut\"")),
                    };
                }
                "lines" => {
                    self.lines = match value {
                        Value::Null => None,
                        _ => Some(list_of(name, value, MAX_UI_ITEMS, "MAX_UI_ITEMS")?.iter().map(|line| long("a line", line)).collect::<Result<_, _>>()?),
                    };
                }
                "more" => self.more = long(name, value)?,
                "target" => self.target = text_of(name, value, MAX_URL_BYTES, "MAX_URL_BYTES")?,
                "items" => self.items = list_of(name, value, MAX_UI_ITEMS, "MAX_UI_ITEMS")?.iter().map(item_of).collect::<Result<_, _>>()?,
                "selected" => self.selected = count_of(name, value, MAX_UI_ITEMS, "MAX_UI_ITEMS")?,
                "shown" => self.shown = count_of(name, value, MAX_UI_ITEMS, "MAX_UI_ITEMS")?,
                "columns" => self.columns = list_of(name, value, MAX_TABLE_COLUMNS, "MAX_UI_TABLE_COLUMNS")?.iter().map(column_of).collect::<Result<_, _>>()?,
                "rows" => {
                    self.rows = list_of(name, value, MAX_UI_ITEMS, "MAX_UI_ITEMS")?
                        .iter()
                        .map(|row| list_of("a row", row, MAX_TABLE_COLUMNS, "MAX_UI_TABLE_COLUMNS")?.iter().map(|cell| long("a row's cell", cell)).collect())
                        .collect::<Result<_, _>>()?;
                }
                "selectable" => self.selectable = flag_of(name, value)?,
                "value" if kind == Kind::Field => self.value = long(name, value)?,
                "value" => self.number = number_of(name, value, MAX_UI_NUMBER.saturating_neg(), MAX_UI_NUMBER, "MAX_UI_NUMBER")?,
                "longest" => self.longest = count_of(name, value, MAX_FIELD_CHARS, "MAX_UI_FIELD_CHARS")?.max(1),
                "placeholder" => self.placeholder = long(name, value)?,
                "masked" => self.masked = flag_of(name, value)?,
                "on" => self.on = flag_of(name, value)?,
                "min" => self.min = number_of(name, value, MAX_UI_NUMBER.saturating_neg(), MAX_UI_NUMBER, "MAX_UI_NUMBER")?,
                "max" => self.max = number_of(name, value, MAX_UI_NUMBER.saturating_neg(), MAX_UI_NUMBER, "MAX_UI_NUMBER")?,
                "step" => self.step = number_of(name, value, 1, MAX_UI_NUMBER, "MAX_UI_NUMBER")?,
                "format" => {
                    self.seconds = match value {
                        Value::Text(format) if format == "HH:MM" => false,
                        Value::Text(format) if format == "HH:MM:SS" => true,
                        _ => return Err(type_error("format is \"HH:MM\" or \"HH:MM:SS\"")),
                    };
                }
                _ => {
                    self.utc = match value {
                        Value::Text(zone) if zone == "local" => false,
                        Value::Text(zone) if zone == "utc" => true,
                        _ => return Err(type_error("zone is \"local\" or \"utc\"")),
                    };
                }
            }
        }
        self.settle(kind)?;
        Ok(area)
    }

    /// Holds the options to one another once all are taken: a value
    /// inside its range, the highlight on an item there is, a field's
    /// value within its longest, a table's rows within its columns.
    fn settle(&mut self, kind: Kind) -> Result<(), Reply> {
        match kind {
            Kind::Slider | Kind::Progress => {
                if kind == Kind::Progress {
                    self.min = 0;
                }
                if self.max <= self.min {
                    return Err(range("MAX_UI_NUMBER", &format!("max, {}, is greater than min, {}", self.max, self.min)));
                }
                self.number = self.number.clamp(self.min, self.max);
            }
            Kind::Menu => self.selected = self.selected.min(self.items.len().saturating_sub(1)),
            Kind::Table => {
                self.selected = self.selected.min(self.rows.len().saturating_sub(1));
                if self.rows.iter().any(|row| row.len() > self.columns.len()) {
                    return Err(type_error("a table's row has no more cells than the table has columns"));
                }
            }
            Kind::Field if self.value.chars().count() > self.longest => {
                return Err(range("MAX_UI_FIELD_CHARS", &format!("the field's value is at most its longest, {} characters", self.longest)));
            }
            _ => {}
        }
        Ok(())
    }

    /// The bytes the properties hold beside their own size: what a
    /// script's options make the host keep.
    #[must_use]
    pub fn bytes(&self) -> usize {
        let texts = [&self.id, &self.label, &self.language, &self.title, &self.hint, &self.text, &self.more, &self.target, &self.value, &self.placeholder];
        let mut bytes = texts.iter().fold(0_usize, |sum, text| sum.saturating_add(text.len()));
        let lines = self.prompts.iter().chain(self.lines.iter().flatten()).chain(self.rows.iter().flatten());
        bytes = lines.fold(bytes, |sum, line| sum.saturating_add(line.len()).saturating_add(size_of::<String>()));
        bytes = bytes.saturating_add(self.rows.len().saturating_mul(size_of::<Vec<String>>()));
        bytes = self.columns.iter().fold(bytes, |sum, column| sum.saturating_add(column.title.len()).saturating_add(size_of::<Column>()));
        self.items.iter().fold(bytes, |sum, item| {
            let value = match &item.value {
                Some(Value::Text(text)) => text.len(),
                _ => 0,
            };
            sum.saturating_add(item.text.len())
                .saturating_add(value)
                .saturating_add(item.target.as_ref().map_or(0, String::len))
                .saturating_add(size_of::<Item>())
        })
    }

    /// The items the properties hold: the budget report's `uiNodes`.
    #[must_use]
    pub fn nodes(&self) -> usize {
        self.items.len().saturating_add(self.rows.len()).saturating_add(self.lines.as_ref().map_or(0, Vec::len))
    }
}
