//! A NAPLPS painter program (script-api.md section 10.2, after the
//! operations' table): the calls a script made on a `NaplpsPainter`, as
//! whole numbers. Each call is its number then its whole numbers; a
//! string is its length then its code points; a list of points is its
//! count then its pairs. The numbers are one for each call of section
//! 7.3's table, in the table's order from 1, written in front of the
//! prelude by [`table`] so that the two halves cannot disagree. A number
//! is never reused.
//!
//! A program is outside input: every word is checked as it is read, and
//! the whole program is refused on the first fault.

use super::Fault;

/// Defines the calls: each name and number once.
macro_rules! calls {
    ($($(#[$doc:meta])* $name:ident = $number:literal,)*) => {
        /// A call of a `NaplpsPainter` (section 7.3).
        #[derive(Clone, Copy, PartialEq, Eq, Debug)]
        #[repr(i32)]
        #[allow(non_camel_case_types, reason = "each name is the prelude's JavaScript constant, written as JavaScript writes constants")]
        pub enum Paint {
            $(
                $(#[$doc])*
                $name = $number,
            )*
        }

        impl Paint {
            /// Every call, with its name.
            pub const ALL: &[(&str, Paint)] = &[$((stringify!($name), Paint::$name),)*];

            /// The call numbered `number`.
            #[must_use]
            pub fn from_number(number: i32) -> Option<Self> {
                Self::ALL.iter().find(|(_, one)| *one as i32 == number).map(|(_, one)| *one)
            }

            /// Its number.
            #[must_use]
            pub const fn number(self) -> i32 {
                self as i32
            }
        }
    };
}

calls! {
    /// `moveTo(x, y)`.
    MOVE_TO = 1,
    /// `moveBy(dx, dy)`.
    MOVE_BY = 2,
    /// `pointAt(x, y)`.
    POINT_AT = 3,
    /// `pointBy(dx, dy)`.
    POINT_BY = 4,
    /// `lineTo(x, y)`.
    LINE_TO = 5,
    /// `lineBy(dx, dy)`.
    LINE_BY = 6,
    /// `polyline(points)`: a count, then that many pairs.
    POLYLINE = 7,
    /// `polylineBy(steps)`: a count, then that many pairs.
    POLYLINE_BY = 8,
    /// `rect(x, y, width, height)`.
    RECT = 9,
    /// `fillRect(x, y, width, height)`.
    FILL_RECT = 10,
    /// `rectBy(width, height)`.
    RECT_BY = 11,
    /// `fillRectBy(width, height)`.
    FILL_RECT_BY = 12,
    /// `polygon(points)`.
    POLYGON = 13,
    /// `fillPolygon(points)`.
    FILL_POLYGON = 14,
    /// `polygonBy(steps)`.
    POLYGON_BY = 15,
    /// `fillPolygonBy(steps)`.
    FILL_POLYGON_BY = 16,
    /// `arc(mx, my, ex, ey)`.
    ARC = 17,
    /// `fillArc(mx, my, ex, ey)`.
    FILL_ARC = 18,
    /// `circle(cx, cy, radius)`.
    CIRCLE = 19,
    /// `disc(cx, cy, radius)`.
    DISC = 20,
    /// `trace(dx, dy, moves)`: the step, then the moves as a string.
    TRACE = 21,
    /// `fillTrace(dx, dy, moves)`.
    FILL_TRACE = 22,
    /// `image(x, y, width, height, pixels)`: then a count and that many colour-map entries.
    IMAGE = 23,
    /// `ink(entry)`.
    INK = 24,
    /// `inkOn(entry, background)`.
    INK_ON = 25,
    /// `pen(width, height)`.
    PEN = 26,
    /// `stroke(s)`: a [`super::writer::Line`]'s number.
    STROKE = 27,
    /// `fillStyle(s)`: 0 to 3 a named pattern, or 4 then a texture's handle.
    FILL_STYLE = 28,
    /// `highlight(on)`.
    HIGHLIGHT = 29,
    /// `mask(width, height)`.
    MASK = 30,
    /// `text(row, column, string, options)`: row, column, size, the language as a string, the text as a string.
    TEXT = 31,
    /// `textAt(x, y, string, options)`: x, y, width, height, rotation, path, spacing, row spacing, underline, reverse, the language, the text.
    TEXT_AT = 32,
    /// `glyph(drcs)`: a DRCS character's handle.
    GLYPH = 33,
    /// `stamp(symbol, x, y)`: a symbol's handle, x, y.
    STAMP = 34,
    /// `wait(tenths)`.
    WAIT = 35,
}

/// The calls as the table the prelude is given: `const NP = {...}`.
pub fn table() -> impl Iterator<Item = (&'static str, i64)> {
    Paint::ALL.iter().map(|(name, call)| (*name, i64::from(call.number())))
}

/// Reads a program's words in order.
pub struct Words<'a> {
    rest: &'a [i32],
}

impl<'a> Words<'a> {
    /// A reader of `words`.
    #[must_use]
    pub const fn new(words: &'a [i32]) -> Self {
        Self { rest: words }
    }

    /// Whether every word was read.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.rest.is_empty()
    }

    /// The next word.
    ///
    /// # Errors
    ///
    /// A `TypeError` when the program ends here.
    pub fn word(&mut self) -> Result<i32, Fault> {
        let (first, rest) = self.rest.split_first().ok_or_else(short)?;
        self.rest = rest;
        Ok(*first)
    }

    /// The next `N` words.
    ///
    /// # Errors
    ///
    /// A `TypeError` when the program ends before them.
    pub fn words<const N: usize>(&mut self) -> Result<[i32; N], Fault> {
        let (taken, rest) = self.rest.split_at_checked(N).ok_or_else(short)?;
        self.rest = rest;
        <[i32; N]>::try_from(taken).map_err(|_| short())
    }

    /// A count of at most `most`, which `bound` names.
    ///
    /// # Errors
    ///
    /// A `RangeError` naming `bound` for a count past `most`; a
    /// `TypeError` for a negative one or a program that ends here.
    pub fn count(&mut self, most: usize, bound: &'static str, what: &str) -> Result<usize, Fault> {
        let count = self.word()?;
        match usize::try_from(count) {
            Ok(count) if count <= most => Ok(count),
            Ok(_) => Err(Fault::Range(format!("{bound}: {what} has at most {most}"))),
            Err(_) => Err(Fault::Type(format!("{what} has a count that is not negative"))),
        }
    }

    /// A list of pairs: a count of at most `most`, then the pairs. The
    /// count is checked against the words there are before anything is
    /// allocated.
    ///
    /// # Errors
    ///
    /// As [`Words::count`], and a `TypeError` when the program ends
    /// before the pairs do.
    pub fn pairs(&mut self, most: usize, bound: &'static str, what: &str) -> Result<Vec<(i32, i32)>, Fault> {
        let count = self.count(most, bound, what)?;
        let (taken, rest) = self.rest.split_at_checked(count.saturating_mul(2)).ok_or_else(short)?;
        self.rest = rest;
        Ok(taken.as_chunks::<2>().0.iter().map(|[x, y]| (*x, *y)).collect())
    }

    /// A list of single words: a count of at most `most`, then the words.
    ///
    /// # Errors
    ///
    /// As [`Words::count`], and a `TypeError` when the program ends
    /// before the words do.
    pub fn list(&mut self, most: usize, bound: &'static str, what: &str) -> Result<&'a [i32], Fault> {
        let count = self.count(most, bound, what)?;
        let (taken, rest) = self.rest.split_at_checked(count).ok_or_else(short)?;
        self.rest = rest;
        Ok(taken)
    }

    /// A string: its length in code points, at most `most`, then each
    /// code point, which must be a Unicode scalar value.
    ///
    /// # Errors
    ///
    /// As [`Words::list`], and a `TypeError` for a word that is no
    /// character.
    pub fn string(&mut self, most: usize, bound: &'static str, what: &str) -> Result<String, Fault> {
        let points = self.list(most, bound, what)?;
        points
            .iter()
            .map(|point| u32::try_from(*point).ok().and_then(char::from_u32).ok_or_else(|| Fault::Type(format!("{what} holds a number that is no character"))))
            .collect()
    }
}

fn short() -> Fault {
    Fault::Type("the painter program ends inside a call".to_owned())
}
