//! Page 300's game: invaders, played with the arrow keys and Space, with
//! no cursor at all.

use std::collections::BTreeMap;

use super::tti::{ALPHA_CYAN, ALPHA_GREEN, ALPHA_RED, ALPHA_WHITE, COLUMNS, DOUBLE_HEIGHT, Page, Row};

/// The invaders' playfield.
pub(super) const FIELD_LEFT: i32 = 1;
pub(super) const FIELD_RIGHT: i32 = 38;
pub(super) const INVADER_TOP: i32 = 5;
pub(super) const SHIP_ROW: i32 = 21;
const INVADER_COLUMNS: i32 = 8;
const INVADER_ROWS: i32 = 3;
const INVADER_SPACING: i32 = 4;
/// The invaders step sideways every few frames, and down at each edge.
const INVADER_PERIOD: u32 = 4;

/// What happened in one frame of the game.
#[derive(Default)]
pub(super) struct Happened {
    /// A shot hit an invader.
    pub(super) hit: bool,
    /// The game ended in this frame: won, or lost.
    pub(super) over: Option<bool>,
}

pub(super) struct Invaders {
    pub(super) alive: Vec<(i32, i32)>,
    pub(super) offset: i32,
    direction: i32,
    pub(super) drop: i32,
    pub(super) ship: i32,
    pub(super) shots: Vec<(i32, i32)>,
    pub(super) left: bool,
    pub(super) right: bool,
    pub(super) score: u32,
    frames: u32,
    pub(super) over: Option<bool>,
}

impl Invaders {
    pub(super) fn new() -> Self {
        let alive = (0..INVADER_ROWS).flat_map(|row| (0..INVADER_COLUMNS).map(move |column| (column * INVADER_SPACING, row * 2))).collect();
        Self { alive, offset: 2, direction: 1, drop: 0, ship: 19, shots: Vec::new(), left: false, right: false, score: 0, frames: 0, over: None }
    }

    /// One frame of the game: what happened in it, for its sounds.
    pub(super) fn tick(&mut self) -> Happened {
        let mut happened = Happened::default();
        if self.over.is_some() {
            return happened;
        }
        self.frames += 1;
        if self.left {
            self.ship = (self.ship - 1).max(FIELD_LEFT);
        }
        if self.right {
            self.ship = (self.ship + 1).min(FIELD_RIGHT - 2);
        }
        for shot in &mut self.shots {
            shot.1 -= 1;
        }
        self.shots.retain(|shot| shot.1 >= INVADER_TOP);
        let (offset, drop) = (self.offset, self.drop);
        let mut hit = Vec::new();
        for (index, shot) in self.shots.iter().enumerate() {
            if let Some(position) = self.alive.iter().position(|&(x, y)| shot.1 == INVADER_TOP + y + drop && (x + offset..x + offset + 3).contains(&shot.0)) {
                self.alive.remove(position);
                hit.push(index);
                self.score += 10;
                happened.hit = true;
            }
        }
        for index in hit.into_iter().rev() {
            self.shots.remove(index);
        }
        if self.frames.is_multiple_of(INVADER_PERIOD) {
            let edge = self.alive.iter().any(|&(x, _)| {
                let next = x + self.offset + self.direction;
                next < FIELD_LEFT || next + 2 > FIELD_RIGHT
            });
            if edge {
                self.direction = -self.direction;
                self.drop += 1;
            } else {
                self.offset += self.direction;
            }
        }
        if self.alive.is_empty() {
            self.over = Some(true);
        } else if self.alive.iter().any(|&(_, y)| INVADER_TOP + y + self.drop >= SHIP_ROW) {
            self.over = Some(false);
        }
        happened.over = self.over;
        happened
    }

    pub(super) fn page(&self, acked: u16) -> Page {
        let mut page = Page::new(0x300, "Invaders");
        page.row(3, Row::new().code(ALPHA_WHITE).text(&format!("Score {:<6}", self.score)).code(ALPHA_CYAN).text("Left Right move, Space fires"));
        let mut rows: BTreeMap<i32, Vec<u8>> = BTreeMap::new();
        let mut put = |row: i32, column: i32, text: &[u8]| {
            let line = rows.entry(row).or_insert_with(|| vec![b' '; COLUMNS]);
            for (at, &byte) in (column..).zip(text) {
                if let Some(cell) = usize::try_from(at).ok().and_then(|at| line.get_mut(at)) {
                    *cell = byte;
                }
            }
        };
        for &(x, y) in &self.alive {
            put(INVADER_TOP + y + self.drop, x + self.offset, b"<o>");
        }
        for &(x, y) in &self.shots {
            put(y, x, b"!");
        }
        put(SHIP_ROW, self.ship, b"-A-");
        for (row, line) in rows {
            let colour = if row == SHIP_ROW {
                ALPHA_CYAN
            } else if line.contains(&b'!') && !line.contains(&b'o') {
                ALPHA_WHITE
            } else {
                ALPHA_GREEN
            };
            let mut cells = line;
            cells[0] = colour;
            if let Ok(row) = u8::try_from(row) {
                page.row(row, Row::from_codes(cells));
            }
        }
        if let Some(won) = self.over {
            page.row(12, Row::new().at(10).code(if won { ALPHA_GREEN } else { ALPHA_RED }).code(DOUBLE_HEIGHT).text(if won { "YOU WIN" } else { "GAME OVER" }));
            page.row(14, Row::new().at(10).code(ALPHA_WHITE).text("Enter plays again"));
        }
        page.set_acked(acked);
        page
    }
}
