//! The component vocabulary's host calls (script-api.md sections 8 and
//! 10.2, operations 60 to 67; work package U1): each call is given to
//! [`crate::ui::Ui`] with the drawing of the page's renderer.
//!
//! **Where a renderer plugs in.** [`painter`] chooses the
//! [`Painter`](crate::ui::Painter) of the renderer of the page shown.
//! Teletext's is here; work packages U2 and U3 add ReGIS's and NAPLPS's
//! to that one function, each an implementation of the trait on its own
//! state. Nothing else of this file, of `ui.rs` or of the prelude's part
//! knows which renderer draws.

use protocol::script::ContextId;

use super::{Host, Ints, Payload, Reply, no_payload, text_of, type_error};
use crate::limits::{MAX_KEY_NAME_BYTES, MAX_LABEL_BYTES, MAX_TEXT_EVENT_BYTES, TICKS_PER_SECOND};
use crate::ops::{EventKind, KeyBit, Op, PointerAction};
use crate::teletext::Slot;
use crate::ui::limits::{GRID_COLUMNS, GRID_FIRST_ROW, GRID_LAST_ROW, MAX_UI_OPTION_BYTES};
use crate::ui::options::Area;
use crate::ui::{Kind, Painter, Time, What, keys_json};

/// Milliseconds in a second.
const SECOND_MS: u64 = 1000;

/// The drawing of components for `context` on the page shown: its
/// renderer's own. A renderer with no page state for the context has
/// none, and the call is refused.
fn painter(teletext: &mut Slot, context: ContextId) -> Result<&mut dyn Painter, Reply> {
    teletext.painter(context).ok_or_else(|| type_error("components are drawn on the page shown, by the page context of a page whose renderer draws them"))
}

/// The numbers the prelude's component part shares with the host, written
/// in front of the prelude as the table `UI` (section 10.1: defined once,
/// in Rust): each kind by its name in capitals, each event, and the grid.
#[must_use]
pub fn ui_prelude_numbers() -> Vec<(&'static str, i64)> {
    const KINDS: [&str; 13] = ["FRAME", "TEXT", "PARAGRAPH", "BAND", "LINK", "BUTTON", "MENU", "TABLE", "FIELD", "TOGGLE", "SLIDER", "PROGRESS", "CLOCK"];
    const _: () = assert!(KINDS.len() == Kind::ALL.len());
    let mut numbers: Vec<(&'static str, i64)> = KINDS.iter().copied().zip(0_i64..).collect();
    numbers.extend([
        ("MOVE", What::Move as i64),
        ("ACTIVATE", What::Activate as i64),
        ("CHANGE", What::Change as i64),
        ("PAGE", What::Page as i64),
        ("SUBMIT", What::Submit as i64),
        ("CANCEL", What::Cancel as i64),
        ("GRID_COLUMNS", i64::from(GRID_COLUMNS)),
        ("GRID_FIRST_ROW", i64::from(GRID_FIRST_ROW)),
        ("GRID_LAST_ROW", i64::from(GRID_LAST_ROW)),
    ]);
    numbers
}

/// A call's options: the JSON text of its payload.
fn options(payload: Payload<'_>) -> Result<&[u8], Reply> {
    match payload {
        Payload::Text(bytes) => Ok(bytes),
        _ => Err(type_error("a component's options are JSON text")),
    }
}

impl Host {
    /// The time as clocks show it.
    fn ui_time(&self) -> Time {
        Time { epoch_seconds: self.clock_now_ms().checked_div(SECOND_MS).unwrap_or(0), zone_minutes: self.zone_minutes }
    }

    /// A step begins: clocks are drawn for the second it is in.
    pub(super) fn ui_begin_step(&mut self) {
        if self.ui.is_empty() {
            return;
        }
        let time = self.ui_time();
        self.ui.set_time(self.teletext.painter(ContextId::Page), time);
    }

    /// The tick at which a clock of `context` next changes: the first of
    /// the next second.
    pub(super) fn ui_wake(&self, context: ContextId) -> Option<u64> {
        if !self.ui.has_clock(context) {
            return None;
        }
        let left = SECOND_MS.saturating_sub(self.clock_now_ms().checked_rem(SECOND_MS).unwrap_or(0));
        let ticks = left.saturating_mul(TICKS_PER_SECOND).div_ceil(SECOND_MS).max(1);
        Some(self.tick.saturating_add(ticks))
    }

    /// The scope `scope` of `context` ends: its components are removed.
    pub(super) fn ui_scope_end(&mut self, context: ContextId, scope: i32) -> Vec<i32> {
        self.ui.remove_scope(self.teletext.painter(context), context, scope)
    }

    /// One of the component vocabulary's operations.
    pub(super) fn ui_call(&mut self, context: ContextId, op: Op, ints: &Ints<'_>, payload: Payload<'_>) -> Result<Reply, Reply> {
        match op {
            Op::UI_KEYS => {
                let [kind] = ints.exactly()?;
                no_payload(op, payload)?;
                keys_json(kind).map(Reply::Text)
            }
            Op::UI_CREATE => {
                let [kind, row, column, columns, rows] = ints.exactly()?;
                let options = options(payload)?;
                let kind = Kind::of(kind).ok_or_else(|| type_error("no such kind of component"))?;
                let at = Area::new(i64::from(row), i64::from(column), i64::from(columns), i64::from(rows))?;
                // What a component makes the host keep is its options,
                // and a label may wait to leave.
                self.room(MAX_UI_OPTION_BYTES.saturating_mul(2), MAX_LABEL_BYTES)?;
                let scope = self.context(context).and_then(|held| held.scope_stack.last().copied()).unwrap_or(0);
                let time = self.ui_time();
                self.ui.set_time(None, time);
                self.ui.create(painter(&mut self.teletext, context)?, context, scope, kind, at, options).map(Reply::Int)
            }
            Op::UI_SET => {
                let [handle] = ints.exactly()?;
                let options = options(payload)?;
                self.room(MAX_UI_OPTION_BYTES.saturating_mul(2), MAX_LABEL_BYTES)?;
                self.ui.set(painter(&mut self.teletext, context)?, context, handle, options).map(|()| Reply::Undefined)
            }
            Op::UI_REMOVE => {
                let [handle] = ints.exactly()?;
                no_payload(op, payload)?;
                self.ui.remove(painter(&mut self.teletext, context)?, context, handle)?;
                if let Some(held) = self.context_mut(context) {
                    held.listens.retain(|(listened, _)| *listened != handle);
                }
                Ok(Reply::Undefined)
            }
            Op::UI_FOCUS => {
                let [handle] = ints.exactly()?;
                no_payload(op, payload)?;
                self.ui.focus(painter(&mut self.teletext, context)?, context, handle).map(|()| Reply::Undefined)
            }
            Op::UI_VALUE => {
                let [handle] = ints.exactly()?;
                no_payload(op, payload)?;
                self.ui.value(context, handle).map(Reply::Text)
            }
            Op::UI_OFFER => {
                let [kind, first, second] = ints.exactly()?;
                // No component: nothing is offered, and nothing refused.
                let Ok(painter) = painter(&mut self.teletext, context) else { return Ok(Reply::Undefined) };
                let offered = match EventKind::from_number(kind) {
                    Some(EventKind::KEY_DOWN) => {
                        let name = text_of(op, payload)?;
                        if name.len() > MAX_KEY_NAME_BYTES {
                            return Err(type_error("no key has so long a name"));
                        }
                        self.ui.offer_key(painter, context, name, first & KeyBit::SHIFT.number() != 0)
                    }
                    Some(EventKind::TEXT) => {
                        let text = text_of(op, payload)?;
                        if text.len() > MAX_TEXT_EVENT_BYTES {
                            return Err(type_error("no text event is so long"));
                        }
                        self.ui.offer_text(context, text)
                    }
                    Some(EventKind::POINTER) => {
                        no_payload(op, payload)?;
                        self.ui.offer_pointer(painter, context, first, second == PointerAction::DOWN.number(), second == PointerAction::UP.number())
                    }
                    _ => return Err(type_error("a component is offered a key, text or the pointer")),
                };
                Ok(offered.map_or(Reply::Undefined, Reply::Text))
            }
            _ => {
                let [handle, go] = ints.exactly()?;
                no_payload(op, payload)?;
                if !(0..=1).contains(&go) {
                    return Err(type_error("a verdict is to go on (1) or not (0)"));
                }
                let done = self.ui.settle(painter(&mut self.teletext, context)?, context, handle, go == 1)?;
                Ok(Reply::Int(i32::from(done)))
            }
        }
    }
}
