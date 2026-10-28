//! A page as TTI, the engine's page file format: the spacing attributes
//! the pages use (EN 300 706 Table 26), a row of codes, and the page with
//! its cursor record, input modes and links.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use neoteletext_input::caps::{CURSOR_CAP, PAGE_CAP};
use neoteletext_input::cursor::NO_CURSOR;

// EN 300 706 Table 26: the spacing attributes used here.
pub(super) const ALPHA_RED: u8 = 0x01;
pub(super) const ALPHA_GREEN: u8 = 0x02;
pub(super) const ALPHA_YELLOW: u8 = 0x03;
const ALPHA_BLUE: u8 = 0x04;
pub(super) const ALPHA_MAGENTA: u8 = 0x05;
pub(super) const ALPHA_CYAN: u8 = 0x06;
pub(super) const ALPHA_WHITE: u8 = 0x07;
pub(super) const DOUBLE_HEIGHT: u8 = 0x0D;
pub(super) const MOSAIC_RED: u8 = 0x11;
pub(super) const MOSAIC_BLUE: u8 = 0x14;
pub(super) const MOSAIC_CYAN: u8 = 0x16;
pub(super) const MOSAIC_WHITE: u8 = 0x17;
const NEW_BACKGROUND: u8 = 0x1D;
pub(super) const BLACK_BACKGROUND: u8 = 0x1C;
/// TTI writes a code below 0x20 as Escape and the code plus 0x40.
const ESCAPE: char = '\x1b';
const ESCAPE_OFFSET: u8 = 0x40;
/// The page status word in TTI's `PS` form: 0x8000 a page to show, and
/// 0x4000 C4, erase page (the engine's `TTI_PS_C4`).
const PAGE_STATUS: u16 = 0x8000 | 0x4000;
/// `#` in the English national option subset (EN 300 706 Table 36).
const ENGLISH_HASH: u8 = 0x5F;
pub(super) const COLUMNS: usize = 40;

/// A row of a page: 7-bit codes, at most 40.
#[derive(Clone, Default)]
pub(super) struct Row(Vec<u8>);

impl Row {
    pub(super) fn new() -> Self {
        Self(Vec::new())
    }

    /// A row of these codes.
    pub(super) fn from_codes(codes: Vec<u8>) -> Self {
        Self(codes)
    }

    pub(super) fn code(mut self, code: u8) -> Self {
        self.0.push(code);
        self
    }

    /// Text in the English national option subset (EN 300 706 Table 36),
    /// the page's default: there `#` is at 0x5F, where ASCII has `_`, and
    /// 0x23 is `£`.
    pub(super) fn text(mut self, text: &str) -> Self {
        self.0.extend(text.chars().map(|character| match character {
            '#' => ENGLISH_HASH,
            character if character.is_ascii() && !character.is_ascii_control() => character as u8,
            _ => b'?',
        }));
        self
    }

    pub(super) fn at(mut self, column: usize) -> Self {
        self.0.resize(column.max(self.0.len()), b' ');
        self
    }
}

/// What the channel uses of what a receiver offers (INPUT.md section 6.2):
/// pages and the cursor.
pub(super) const USED: u32 = PAGE_CAP | CURSOR_CAP;

/// A page as TTI: its rows and its cursor record as a `CU` line.
pub(super) struct Page {
    number: u16,
    rows: BTreeMap<u8, Row>,
    cursor: Option<(u8, u8)>,
    /// The input count acted on (INPUT.md section 7.1).
    acked: u16,
    /// `LT` lines: a run of text that is a link.
    links: Vec<String>,
}

impl Page {
    pub(super) fn new(number: u16, title: &str) -> Self {
        let mut page = Self { number, rows: BTreeMap::new(), cursor: None, acked: 0, links: Vec::new() };
        page.row(0, Row::new().at(8).code(ALPHA_YELLOW).text(&format!("NEOTELETEXT TERMINAL {number:03X}")));
        // Row 1 is double height, so row 2 is its lower half.
        page.row(1, Row::new().code(ALPHA_BLUE).code(NEW_BACKGROUND).code(ALPHA_YELLOW).code(DOUBLE_HEIGHT).text(&title.to_uppercase()));
        page
    }

    /// The cursor record: its row and column.
    pub(super) fn set_cursor(&mut self, cursor: (u8, u8)) {
        self.cursor = Some(cursor);
    }

    /// The input count acted on.
    pub(super) fn set_acked(&mut self, acked: u16) {
        self.acked = acked;
    }

    /// An `LT` line, a run of text that is a link.
    pub(super) fn link(&mut self, line: String) {
        self.links.push(line);
    }

    pub(super) fn row(&mut self, number: u8, row: Row) {
        self.rows.insert(number, row);
    }

    pub(super) fn tti(&self) -> String {
        // C4, erase page (EN 300 706 Table 2): each version replaces the
        // last whole, so a row left out is blank, not the row before.
        let mut out = format!("PN,{:03X}00\r\nSC,0000\r\nPS,{PAGE_STATUS:04X}\r\n", self.number);
        for (number, row) in &self.rows {
            let _ = write!(out, "OL,{number},");
            for &code in row.0.iter().take(COLUMNS) {
                if code < 0x20 {
                    out.push(ESCAPE);
                    out.push(char::from(code + ESCAPE_OFFSET));
                } else {
                    out.push(char::from(code));
                }
            }
            out.push_str("\r\n");
        }
        for link in &self.links {
            let _ = write!(out, "{link}\r\n");
        }
        // The engine's REQ-INT-006: the cursor record as a line, with no
        // echo width and no flags, since the browser echoes nothing; a page
        // with no cursor still acknowledges the keys, for the round trip.
        let (row, column) = self.cursor.unwrap_or((NO_CURSOR, 0));
        let _ = write!(out, "CU,{row},{column},0,0,{}\r\n", self.acked);
        out
    }
}
