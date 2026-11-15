//! The two state machines of the sub-page list (DEC-6358 item 4), each as
//! the engine writes its own (`neoteletext-core`'s `fsm.rs`): an enum of
//! states, of events and of actions, and one transition function with an
//! arm for every state and event, none of them a catch-all, so that an
//! event nobody planned for cannot be met without a decision about it.
//! They replace the flags the list kept (`asked`, `landing.asked`), which
//! an unplanned order of messages could leave set for good.
//!
//! **The rows asked for** ([`rows_transition`]): the list asks the trusted
//! core for the rows it shows, one request at a time, each with a number
//! its answer carries back.
//!
//! | State | Event | Next | Action |
//! |---|---|---|---|
//! | `Idle` | `NeedWaitedFor` | `Waiting` | `Ask` |
//! | `Idle` | `NeedOther` | `Waiting` | `Ask` |
//! | `Idle` | `AnswerOurs` | `Idle` | `Drop` |
//! | `Idle` | `AnswerOther` | `Idle` | `Drop` |
//! | `Idle` | `Invalidated` | `Idle` | `None` |
//! | `Idle` | `TimedOut` | `Idle` | `None` |
//! | `Waiting` | `NeedWaitedFor` | `Waiting` | `None` |
//! | `Waiting` | `NeedOther` | `Waiting` | `Ask` |
//! | `Waiting` | `AnswerOurs` | `Idle` | `Take` |
//! | `Waiting` | `AnswerOther` | `Waiting` | `Drop` |
//! | `Waiting` | `Invalidated` | `Idle` | `None` |
//! | `Waiting` | `TimedOut` | `Waiting` | `Ask` |
//!
//! **The move waiting to be shown** ([`move_transition`]): a key or the
//! wheel moved the list to a place whose sub-page is to be shown, which
//! can be asked for only once its row is held.
//!
//! | State | Event | Next | Action |
//! |---|---|---|---|
//! | `Rest` | `Moved` | `AwaitingRow` | `None` |
//! | `Rest` | `RowHeld` | `Rest` | `None` |
//! | `Rest` | `Shown` | `Rest` | `None` |
//! | `Rest` | `Left` | `Rest` | `None` |
//! | `Rest` | `TimedOut` | `Rest` | `None` |
//! | `AwaitingRow` | `Moved` | `AwaitingRow` | `None` |
//! | `AwaitingRow` | `RowHeld` | `AwaitingShow` | `Open` |
//! | `AwaitingRow` | `Shown` | `Rest` | `None` |
//! | `AwaitingRow` | `Left` | `Rest` | `None` |
//! | `AwaitingRow` | `TimedOut` | `AwaitingRow` | `None` |
//! | `AwaitingShow` | `Moved` | `AwaitingRow` | `None` |
//! | `AwaitingShow` | `RowHeld` | `AwaitingShow` | `None` |
//! | `AwaitingShow` | `Shown` | `Rest` | `None` |
//! | `AwaitingShow` | `Left` | `Rest` | `None` |
//! | `AwaitingShow` | `TimedOut` | `Rest` | `None` |

/// How long a request for rows waits for its answer before it is made
/// again: the answer is the trusted core's own, or its decoder's, on this
/// computer, so a second is far longer than either takes, and short enough
/// that a lost answer is not noticed as untitled rows (DEC-6358, fault 4).
pub(super) const ROWS_TIMEOUT_MILLISECONDS: u64 = 1_000;
/// How long a move waits for its sub-page to be shown before it is given
/// up, so that the list follows what is shown again: a live channel's
/// answer crosses the network, so longer than the rows' wait.
pub(super) const MOVE_TIMEOUT_MILLISECONDS: u64 = 4_000;

/// Whether a request for rows is in flight.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum RowsState {
    /// No request is waiting for its answer.
    #[default]
    Idle,
    /// A request was made and not yet answered.
    Waiting,
}

/// What can happen to the rows asked for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum RowsEvent {
    /// The list needs the rows the request in flight asked for.
    NeedWaitedFor,
    /// The list needs rows no request in flight asked for.
    NeedOther,
    /// Rows arrived with the number of the request in flight, for the
    /// channel shown.
    AnswerOurs,
    /// Rows arrived with another number or for another channel.
    AnswerOther,
    /// The page's count changed or another channel is shown: what was
    /// asked for is no longer what is wanted.
    Invalidated,
    /// The request in flight has waited [`ROWS_TIMEOUT_MILLISECONDS`].
    TimedOut,
}

/// What to do about the rows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum RowsAction {
    /// Nothing.
    None,
    /// Make a request, with a new number.
    Ask,
    /// Keep the rows that arrived.
    Take,
    /// Leave the rows that arrived: they answer nothing waited for.
    Drop,
}

/// The rows machine's table.
#[allow(clippy::match_same_arms, reason = "one arm for each row of the table, as the engine's machines are written")]
pub(super) const fn rows_transition(state: RowsState, event: RowsEvent) -> (RowsState, RowsAction) {
    match (state, event) {
        (RowsState::Idle, RowsEvent::NeedWaitedFor) => (RowsState::Waiting, RowsAction::Ask),
        (RowsState::Idle, RowsEvent::NeedOther) => (RowsState::Waiting, RowsAction::Ask),
        (RowsState::Idle, RowsEvent::AnswerOurs) => (RowsState::Idle, RowsAction::Drop),
        (RowsState::Idle, RowsEvent::AnswerOther) => (RowsState::Idle, RowsAction::Drop),
        (RowsState::Idle, RowsEvent::Invalidated) => (RowsState::Idle, RowsAction::None),
        (RowsState::Idle, RowsEvent::TimedOut) => (RowsState::Idle, RowsAction::None),
        (RowsState::Waiting, RowsEvent::NeedWaitedFor) => (RowsState::Waiting, RowsAction::None),
        (RowsState::Waiting, RowsEvent::NeedOther) => (RowsState::Waiting, RowsAction::Ask),
        (RowsState::Waiting, RowsEvent::AnswerOurs) => (RowsState::Idle, RowsAction::Take),
        (RowsState::Waiting, RowsEvent::AnswerOther) => (RowsState::Waiting, RowsAction::Drop),
        (RowsState::Waiting, RowsEvent::Invalidated) => (RowsState::Idle, RowsAction::None),
        (RowsState::Waiting, RowsEvent::TimedOut) => (RowsState::Waiting, RowsAction::Ask),
    }
}

/// How far a move of the list has got.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum MoveState {
    /// No move is waiting.
    #[default]
    Rest,
    /// The list moved to a place whose row is not held yet.
    AwaitingRow,
    /// Its sub-page was asked for and is not yet the one shown.
    AwaitingShow,
}

/// What can happen to a move.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum MoveEvent {
    /// A key or the wheel moved the list to a place.
    Moved,
    /// The row of the place moved to is held.
    RowHeld,
    /// The sub-page at the place moved to is the one shown.
    Shown,
    /// Another page or another channel is listed.
    Left,
    /// The move has waited [`MOVE_TIMEOUT_MILLISECONDS`] to be shown.
    TimedOut,
}

/// What to do about a move.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum MoveAction {
    /// Nothing.
    None,
    /// Ask for the sub-page at the place moved to.
    Open,
}

/// The move machine's table.
#[allow(clippy::match_same_arms, reason = "one arm for each row of the table, as the engine's machines are written")]
pub(super) const fn move_transition(state: MoveState, event: MoveEvent) -> (MoveState, MoveAction) {
    match (state, event) {
        (MoveState::Rest, MoveEvent::Moved) => (MoveState::AwaitingRow, MoveAction::None),
        (MoveState::Rest, MoveEvent::RowHeld) => (MoveState::Rest, MoveAction::None),
        (MoveState::Rest, MoveEvent::Shown) => (MoveState::Rest, MoveAction::None),
        (MoveState::Rest, MoveEvent::Left) => (MoveState::Rest, MoveAction::None),
        (MoveState::Rest, MoveEvent::TimedOut) => (MoveState::Rest, MoveAction::None),
        (MoveState::AwaitingRow, MoveEvent::Moved) => (MoveState::AwaitingRow, MoveAction::None),
        (MoveState::AwaitingRow, MoveEvent::RowHeld) => (MoveState::AwaitingShow, MoveAction::Open),
        (MoveState::AwaitingRow, MoveEvent::Shown) => (MoveState::Rest, MoveAction::None),
        (MoveState::AwaitingRow, MoveEvent::Left) => (MoveState::Rest, MoveAction::None),
        (MoveState::AwaitingRow, MoveEvent::TimedOut) => (MoveState::AwaitingRow, MoveAction::None),
        (MoveState::AwaitingShow, MoveEvent::Moved) => (MoveState::AwaitingRow, MoveAction::None),
        (MoveState::AwaitingShow, MoveEvent::RowHeld) => (MoveState::AwaitingShow, MoveAction::None),
        (MoveState::AwaitingShow, MoveEvent::Shown) => (MoveState::Rest, MoveAction::None),
        (MoveState::AwaitingShow, MoveEvent::Left) => (MoveState::Rest, MoveAction::None),
        (MoveState::AwaitingShow, MoveEvent::TimedOut) => (MoveState::Rest, MoveAction::None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every row of the rows machine's table, as the module's head gives it.
    #[test]
    fn the_rows_machine_follows_its_table() {
        use RowsAction as A;
        use RowsEvent as E;
        use RowsState as S;
        let table = [
            (S::Idle, E::NeedWaitedFor, S::Waiting, A::Ask),
            (S::Idle, E::NeedOther, S::Waiting, A::Ask),
            (S::Idle, E::AnswerOurs, S::Idle, A::Drop),
            (S::Idle, E::AnswerOther, S::Idle, A::Drop),
            (S::Idle, E::Invalidated, S::Idle, A::None),
            (S::Idle, E::TimedOut, S::Idle, A::None),
            (S::Waiting, E::NeedWaitedFor, S::Waiting, A::None),
            (S::Waiting, E::NeedOther, S::Waiting, A::Ask),
            (S::Waiting, E::AnswerOurs, S::Idle, A::Take),
            (S::Waiting, E::AnswerOther, S::Waiting, A::Drop),
            (S::Waiting, E::Invalidated, S::Idle, A::None),
            (S::Waiting, E::TimedOut, S::Waiting, A::Ask),
        ];
        assert_eq!(table.len(), 2 * 6, "every state with every event");
        for (state, event, next, action) in table {
            assert_eq!(rows_transition(state, event), (next, action), "{state:?} on {event:?}");
        }
    }

    /// Every row of the move machine's table.
    #[test]
    fn the_move_machine_follows_its_table() {
        use MoveAction as A;
        use MoveEvent as E;
        use MoveState as S;
        let table = [
            (S::Rest, E::Moved, S::AwaitingRow, A::None),
            (S::Rest, E::RowHeld, S::Rest, A::None),
            (S::Rest, E::Shown, S::Rest, A::None),
            (S::Rest, E::Left, S::Rest, A::None),
            (S::Rest, E::TimedOut, S::Rest, A::None),
            (S::AwaitingRow, E::Moved, S::AwaitingRow, A::None),
            (S::AwaitingRow, E::RowHeld, S::AwaitingShow, A::Open),
            (S::AwaitingRow, E::Shown, S::Rest, A::None),
            (S::AwaitingRow, E::Left, S::Rest, A::None),
            (S::AwaitingRow, E::TimedOut, S::AwaitingRow, A::None),
            (S::AwaitingShow, E::Moved, S::AwaitingRow, A::None),
            (S::AwaitingShow, E::RowHeld, S::AwaitingShow, A::None),
            (S::AwaitingShow, E::Shown, S::Rest, A::None),
            (S::AwaitingShow, E::Left, S::Rest, A::None),
            (S::AwaitingShow, E::TimedOut, S::Rest, A::None),
        ];
        assert_eq!(table.len(), 3 * 5, "every state with every event");
        for (state, event, next, action) in table {
            assert_eq!(move_transition(state, event), (next, action), "{state:?} on {event:?}");
        }
    }
}
