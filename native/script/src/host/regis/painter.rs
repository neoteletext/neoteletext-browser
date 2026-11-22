//! A painter program as a drawing's contents (script-api.md sections 6.3
//! and 10.2): the whole numbers a script's `RegisPainter` calls became,
//! checked one by one and written as ReGIS by the host's own writer, so
//! that the contents are always one well-formed definition. The first
//! fault refuses the whole program.
//!
//! Work package R1 writes the calls a drawing needs to have any contents
//! at all: positions, vectors, polylines, polygons, rectangles, the ink
//! and filling. Work package R2 adds the rest (the other arms of
//! [`Encoder::call`]): pixel vectors, circles, arcs, curves, writing
//! modes, patterns, planes, shading, text and macrograph calls.
//!
//! A drawing's contents start in a fresh drawing context with [0,0] at
//! the drawing's own origin (`regis-v2.md`, "Rules for contents", rule
//! 1), so the encoder always knows where the position is. It needs to:
//! in ReGIS a signed number in brackets is relative, so a place with a
//! negative coordinate can only be written as a move from where the
//! position is.

use std::io::Write as _;

use super::super::{Reply, range, type_error};
use crate::limits::regis::{MAX_COORDINATE, MAX_FILL_VERTICES, MAX_POINTS, MIN_COORDINATE, REGIS_COLOURS, REGIS_MAX_CONTENTS_BYTES};
use crate::ops::RegisPaint;

/// A painter program being read: its whole numbers, front first.
pub(in crate::host) struct Words<'a>(pub(in crate::host) &'a [i32]);

impl<'a> Words<'a> {
    /// The next whole number.
    pub(in crate::host) fn next(&mut self) -> Result<i32, Reply> {
        let (first, rest) = self.0.split_first().ok_or_else(|| type_error("the painter program ends inside a call"))?;
        self.0 = rest;
        Ok(*first)
    }

    /// The next `count` whole numbers, as a program of their own.
    pub(in crate::host) fn take(&mut self, count: usize) -> Result<Words<'a>, Reply> {
        let (taken, rest) = self.0.split_at_checked(count).ok_or_else(|| type_error("the painter program ends inside a call"))?;
        self.0 = rest;
        Ok(Words(taken))
    }

    /// Whether nothing is left.
    pub(in crate::host) const fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// `value` as a coordinate inside a drawing.
pub(in crate::host) fn coordinate(value: i32) -> Result<i32, Reply> {
    if (MIN_COORDINATE..=MAX_COORDINATE).contains(&value) {
        Ok(value)
    } else {
        Err(range("MAX_COORDINATE", "a coordinate inside a drawing is -32,768 to 32,767"))
    }
}

/// Writes a drawing's contents from a painter program.
pub(in crate::host) struct Encoder {
    /// The contents so far.
    pub(in crate::host) out: Vec<u8>,
    /// Where the position is, from the drawing's origin.
    pub(in crate::host) x: i32,
    /// See `x`.
    pub(in crate::host) y: i32,
    /// Whether the calls being read are inside a `fill`.
    pub(in crate::host) filling: bool,
    /// The vertices the `fill` being read has traced.
    pub(in crate::host) fill_vertices: usize,
}

impl Encoder {
    /// An encoder at the start of a drawing's contents.
    pub(in crate::host) const fn new() -> Self {
        Self { out: Vec::new(), x: 0, y: 0, filling: false, fill_vertices: 0 }
    }

    /// Writes text; writing to a vector cannot fail.
    fn text(&mut self, text: core::fmt::Arguments<'_>) {
        let _ = self.out.write_fmt(text);
    }

    /// Writes the place `[x, y]` in brackets and takes the position
    /// there: as it stands when neither coordinate is negative, otherwise
    /// as a move from where the position is, since a signed number in
    /// brackets is relative (the manual's chapter 1, "Relative
    /// Positioning").
    pub(in crate::host) fn place(&mut self, x: i32, y: i32) {
        if x >= 0 && y >= 0 {
            self.text(format_args!("[{x},{y}]"));
        } else {
            self.text(format_args!("[{:+},{:+}]", x.saturating_sub(self.x), y.saturating_sub(self.y)));
        }
        self.x = x;
        self.y = y;
    }

    /// Counts a vertex of the outline a `fill` traces.
    pub(in crate::host) fn vertices(&mut self, count: usize) -> Result<(), Reply> {
        if self.filling {
            self.fill_vertices = self.fill_vertices.saturating_add(count);
            if self.fill_vertices > MAX_FILL_VERTICES {
                return Err(range("MAX_FILL_VERTICES", "a fill traces at most 4,096 vertices"));
            }
        }
        Ok(())
    }

    /// The points of a `polyline` or `polygon`: their count, then each.
    fn points(words: &mut Words<'_>) -> Result<Vec<(i32, i32)>, Reply> {
        let count =
            usize::try_from(words.next()?).ok().filter(|count| *count >= 1).ok_or_else(|| type_error("a polyline or polygon has at least one point"))?;
        if count > MAX_POINTS {
            return Err(range("MAX_POINTS", "a polyline or polygon has at most 1,024 points"));
        }
        (0..count).map(|_| Ok((coordinate(words.next()?)?, coordinate(words.next()?)?))).collect()
    }

    /// Reads and writes one call.
    #[expect(clippy::too_many_lines, reason = "one arm for each call of section 6.3's table")]
    fn call(&mut self, words: &mut Words<'_>) -> Result<(), Reply> {
        let number = words.next()?;
        let call = RegisPaint::from_number(number).ok_or_else(|| type_error("the painter program holds a call no painter has"))?;
        match call {
            RegisPaint::MOVE_TO => {
                let (x, y) = (coordinate(words.next()?)?, coordinate(words.next()?)?);
                self.out.push(b'P');
                self.place(x, y);
            }
            RegisPaint::MOVE_BY | RegisPaint::LINE_BY => {
                let (dx, dy) = (words.next()?, words.next()?);
                let (x, y) = (coordinate(self.x.saturating_add(dx))?, coordinate(self.y.saturating_add(dy))?);
                if call == RegisPaint::LINE_BY {
                    self.vertices(1)?;
                }
                self.out.push(if call == RegisPaint::MOVE_BY { b'P' } else { b'V' });
                self.text(format_args!("[{:+},{:+}]", x.saturating_sub(self.x), y.saturating_sub(self.y)));
                self.x = x;
                self.y = y;
            }
            RegisPaint::LINE_TO => {
                let (x, y) = (coordinate(words.next()?)?, coordinate(words.next()?)?);
                self.vertices(1)?;
                self.out.push(b'V');
                self.place(x, y);
            }
            RegisPaint::DOT => {
                self.vertices(1)?;
                self.out.extend_from_slice(b"V[]");
            }
            RegisPaint::POLYLINE | RegisPaint::POLYGON => {
                let points = Self::points(words)?;
                self.vertices(points.len())?;
                let mut points = points.into_iter();
                let Some((first_x, first_y)) = points.next() else { return Ok(()) };
                self.out.push(b'P');
                self.place(first_x, first_y);
                // A bounded sequence closes back to its first point
                // (chapter 4, "Bounded Sequence"); an open one ends where
                // its last vector ends. One point alone is a dot.
                self.out.extend_from_slice(if call == RegisPaint::POLYGON { b"V(B)" } else { b"V" });
                let mut drawn = false;
                for (x, y) in points {
                    self.place(x, y);
                    drawn = true;
                }
                if !drawn {
                    self.out.extend_from_slice(b"[]");
                }
                if call == RegisPaint::POLYGON {
                    self.out.extend_from_slice(b"(E)");
                    self.x = first_x;
                    self.y = first_y;
                }
            }
            RegisPaint::RECT => {
                let (x, y) = (coordinate(words.next()?)?, coordinate(words.next()?)?);
                let (width, height) = (words.next()?, words.next()?);
                if width < 1 || height < 1 {
                    return Err(type_error("a rectangle's width and height are at least 1"));
                }
                // A rectangle of `width` pixels reaches from its left
                // column to the one `width - 1` further on.
                let (across, down) = (width.saturating_sub(1), height.saturating_sub(1));
                coordinate(x.saturating_add(across))?;
                coordinate(y.saturating_add(down))?;
                self.vertices(4)?;
                self.out.push(b'P');
                self.place(x, y);
                self.text(format_args!("V[+{across}][,+{down}][-{across}][,-{down}]"));
            }
            RegisPaint::FILL => {
                if self.filling {
                    return Err(type_error("a fill is not traced inside another fill"));
                }
                let length = usize::try_from(words.next()?).map_err(|_| type_error("a fill's length is not negative"))?;
                let mut body = words.take(length)?;
                self.filling = true;
                self.fill_vertices = 0;
                self.out.extend_from_slice(b"F(");
                while !body.is_empty() {
                    self.call(&mut body)?;
                }
                self.out.push(b')');
                self.filling = false;
            }
            RegisPaint::INK => {
                let entry = words.next()?;
                if self.filling {
                    return Err(type_error("the ink is chosen before a fill, not inside it"));
                }
                if usize::try_from(entry).map_or(true, |entry| entry >= REGIS_COLOURS) {
                    return Err(range("REGIS_COLOURS", "an ink is a colour-map entry, 0 to 15"));
                }
                self.text(format_args!("W(I{entry})"));
            }
            RegisPaint::VECTOR
            | RegisPaint::CIRCLE
            | RegisPaint::CIRCLE_THROUGH
            | RegisPaint::ARC
            | RegisPaint::CURVE
            | RegisPaint::MODE
            | RegisPaint::NEGATIVE
            | RegisPaint::PATTERN
            | RegisPaint::PLANES
            | RegisPaint::SHADE
            | RegisPaint::TEXT
            | RegisPaint::CALL => return Err(type_error("this painter call is not built yet (work package R2)")),
        }
        if self.out.len() > REGIS_MAX_CONTENTS_BYTES {
            return Err(range("REGIS_MAX_CONTENTS_BYTES", "a drawing's contents are at most 65,536 bytes"));
        }
        Ok(())
    }
}

/// The contents the painter program `words` writes, or the error that
/// refuses it whole.
pub(in crate::host) fn encode(words: &[i32]) -> Result<Vec<u8>, Reply> {
    let mut words = Words(words);
    let mut encoder = Encoder::new();
    while !words.is_empty() {
        encoder.call(&mut words)?;
    }
    Ok(encoder.out)
}
