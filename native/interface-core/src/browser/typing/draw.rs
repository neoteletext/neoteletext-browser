//! Typing drawn: the channel's cursor over the open page, and the strip's
//! badge saying where keys go (REQ-INT-131, REQ-INT-134).

use alloc::string::String;
use alloc::vec::Vec;
use alloc::{format, vec};

use neoteletext_input::cursor::BLOCK;

use super::{BAR_SHARE, BLINK_MILLISECONDS, BLOCK_LINE_SHARE};
use crate::browser::Browser;
use crate::canvas::{COLUMNS, Canvas, PageKey, ROWS};
use crate::geometry::Rect;
use crate::text::left_centred;
use crate::theme::{Colour, FIELD_INK, INSECURE, ROW_STATUS, TYPING};

/// The strip's badge: its padding and its corner.
const BADGE_PADDING: f64 = 10.0;
const BADGE_RADIUS: f64 = 4.0;
const BADGE_INSET: f64 = 6.0;
/// A line of the badge on lines of its own: its 12-point text and half as
/// much again between lines.
const BADGE_LINE: f64 = 18.0;
/// Between the parts of the badge on one line.
const SEPARATOR: &str = " \u{00B7} ";
/// The round trip is shown in milliseconds to a tenth: a ping over a near
/// path takes a fraction of one (`RETURN-CHANNEL.md` section 2.1, 0.19 ms
/// over loopback).
const MICROSECONDS_PER_MILLISECOND: u32 = 1_000;
const MICROSECONDS_PER_TENTH: u32 = 100;

impl Browser {
    /// Draws the cursor over the open page, in an amber no page can draw
    /// (REQ-INT-134).
    pub(in crate::browser) fn draw_terminal<C: Canvas + ?Sized>(&self, canvas: &mut C, now: u64) {
        let Some(target) = self.typing_target() else { return };
        // Where the open page is drawn: zoomed, or on the wall.
        let shown = self.wall.shown().find(|(key, _)| *key == target).map(|(_, rect)| rect);
        let (Some(cursor), Some(page)) = (self.typing.typing.and_then(|typing| typing.cursor), shown) else { return };
        let (cell_width, cell_height) = (page.width() / f64::from(COLUMNS), page.height() / f64::from(ROWS));
        canvas.push_clip(self.wall.area());
        let visible = (now.saturating_sub(self.typing.moved) / BLINK_MILLISECONDS).is_multiple_of(2);
        if visible {
            let x = page.x0 + cell_width * f64::from(cursor.column);
            let y = page.y0 + cell_height * f64::from(cursor.row);
            let rect = Rect::new(x, y, x + cell_width, y + cell_height);
            if cursor.flags & BLOCK != 0 {
                let line = cell_width * BLOCK_LINE_SHARE;
                crate::text::border(canvas, rect, line, TYPING);
            } else {
                canvas.fill(Rect::new(rect.x0, rect.y0, rect.x0 + cell_width * BAR_SHARE, rect.y1), 0.0, TYPING);
            }
        }
        canvas.pop_clip();
    }

    /// What the typing badge says while keys go to a channel, or that the
    /// channel did not answer: none otherwise.
    pub(in crate::browser) fn typing_badge(&self) -> Option<TypingBadge> {
        let typing = self.typing.typing?;
        let refused = typing.refused && self.opened() == Some(PageKey { channel: typing.channel, page: typing.page });
        if self.typing_target().is_none() && !refused {
            return None;
        }
        let words = if refused { format!("{} did not answer: keys stay here", self.strip.identity) } else { format!("Keys go to {}", self.strip.identity) };
        let mut parts = vec![(words, FIELD_INK)];
        if let Some(round_trip) = typing.round_trip_us.filter(|_| !refused) {
            let (milliseconds, tenths) = (round_trip / MICROSECONDS_PER_MILLISECOND, round_trip % MICROSECONDS_PER_MILLISECOND / MICROSECONDS_PER_TENTH);
            parts.push((format!("{milliseconds}.{tenths} ms"), FIELD_INK));
        }
        if !typing.secure && !refused {
            parts.push((String::from("Not secure"), INSECURE));
        }
        Some(TypingBadge { refused, parts })
    }

    /// Draws "Keys go to …" at the right of the address field while keys
    /// go to a channel, with "Not secure" on a plain connection and the
    /// round trip (REQ-INT-131), or that the channel did not answer.
    /// Returns the field's text area left.
    pub(in crate::browser) fn draw_typing_badge<C: Canvas + ?Sized>(&self, canvas: &mut C, area: Rect) -> Rect {
        let Some(badge) = self.typing_badge() else { return area };
        let left = badge.draw_line(canvas, area);
        Rect::new(area.x0, area.y0, (left - BADGE_PADDING).max(area.x0), area.y1)
    }
}

/// The typing badge's words, in the parts a line separates with a dot and
/// a narrow margin puts on lines of their own, each with its ink.
pub(in crate::browser) struct TypingBadge {
    /// The channel did not answer: the badge is the warning's colour.
    refused: bool,
    /// Where keys go, then the round trip and "Not secure" when they apply.
    parts: Vec<(String, Colour)>,
}

impl TypingBadge {
    /// The parts as one line draws them, each but the first after a dot.
    fn runs(&self) -> impl Iterator<Item = (String, Colour)> + '_ {
        self.parts.iter().enumerate().map(|(place, (text, ink))| (if place == 0 { text.clone() } else { format!("{SEPARATOR}{text}") }, *ink))
    }

    /// How wide the badge is on one line, its padding included.
    pub(in crate::browser) fn line_width<C: Canvas + ?Sized>(&self, canvas: &mut C) -> f64 {
        self.runs().map(|(text, _)| canvas.text_width(&text, ROW_STATUS)).sum::<f64>() + 2.0 * BADGE_PADDING
    }

    fn fill(&self) -> Colour {
        if self.refused { INSECURE } else { TYPING }
    }

    /// Draws the badge on one line at the right of `area`; returns its
    /// left edge.
    pub(in crate::browser) fn draw_line<C: Canvas + ?Sized>(&self, canvas: &mut C, area: Rect) -> f64 {
        let width = self.line_width(canvas);
        let badge = Rect::new((area.x1 - width).max(area.x0), area.y0 + BADGE_INSET, area.x1, area.y1 - BADGE_INSET);
        canvas.fill(badge, BADGE_RADIUS, self.fill());
        let mut x = badge.x0 + BADGE_PADDING;
        for (text, ink) in self.runs() {
            left_centred(canvas, Rect::new(x, badge.y0, badge.x1, badge.y1), 0.0, &text, ROW_STATUS, ink);
            x += canvas.text_width(&text, ROW_STATUS);
        }
        badge.x0
    }

    /// Draws the badge at the top right of `area`, a margin too narrow for
    /// it on one line: each part on lines of its own, broken between words
    /// and, in a host too long for a line, after a dot or else anywhere, so
    /// that every character of the host shows (`full-screen.md` section 3).
    /// Draws nothing, and returns false, when `area` is narrower than its
    /// padding and one character, or too short for its lines.
    pub(in crate::browser) fn draw_lines<C: Canvas + ?Sized>(&self, canvas: &mut C, area: Rect) -> bool {
        let room = area.width() - 2.0 * BADGE_PADDING;
        let mut lines = Vec::new();
        for (text, ink) in &self.parts {
            let Some(wrapped) = wrap(canvas, text, room) else { return false };
            lines.extend(wrapped.into_iter().map(|line| (line, *ink)));
        }
        let widest = lines.iter().map(|(line, _)| canvas.text_width(line, ROW_STATUS)).fold(0.0, f64::max);
        #[allow(clippy::cast_precision_loss, reason = "a few lines")]
        let height = lines.len() as f64 * BADGE_LINE + 2.0 * BADGE_INSET;
        if height > area.height() {
            return false;
        }
        let badge = Rect::new(area.x1 - widest - 2.0 * BADGE_PADDING, area.y0, area.x1, area.y0 + height);
        canvas.fill(badge, BADGE_RADIUS, self.fill());
        let mut y = badge.y0 + BADGE_INSET;
        for (line, ink) in &lines {
            left_centred(canvas, Rect::new(badge.x0 + BADGE_PADDING, y, badge.x1, y + BADGE_LINE), 0.0, line, ROW_STATUS, *ink);
            y += BADGE_LINE;
        }
        true
    }
}

/// `text` in lines no wider than `room`: words as they fit, and a word
/// wider than `room` broken after its last dot that fits, else after its
/// last character that fits. None when `room` holds no character.
fn wrap<C: Canvas + ?Sized>(canvas: &mut C, text: &str, room: f64) -> Option<Vec<String>> {
    let mut lines: Vec<String> = Vec::new();
    let mut line = String::new();
    for word in text.split(' ') {
        let joined = if line.is_empty() { String::from(word) } else { format!("{line} {word}") };
        if canvas.text_width(&joined, ROW_STATUS) <= room {
            line = joined;
            continue;
        }
        if !line.is_empty() {
            lines.push(core::mem::take(&mut line));
        }
        let mut rest = word;
        while canvas.text_width(rest, ROW_STATUS) > room {
            let fits = rest
                .char_indices()
                .map(|(at, character)| at.saturating_add(character.len_utf8()))
                .take_while(|&end| rest.get(..end).is_some_and(|head| canvas.text_width(head, ROW_STATUS) <= room))
                .last()?;
            let (head, _) = rest.split_at_checked(fits)?;
            let end = head.rfind('.').map_or(fits, |dot| dot.saturating_add('.'.len_utf8()));
            let (head, tail) = rest.split_at_checked(end)?;
            lines.push(String::from(head));
            rest = tail;
        }
        line = String::from(rest);
    }
    if !line.is_empty() {
        lines.push(line);
    }
    Some(lines)
}
