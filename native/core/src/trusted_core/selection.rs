//! The sub-page chosen on the open page, on its way to being shown, as a
//! state machine (DEC-6358 item 4), written as the engine writes its own
//! (`neoteletext-core`'s `fsm.rs`): an enum of states, of events and of
//! actions, and one transition function with an arm for every pair, none
//! a catch-all. It replaces two flags the core kept apart, a sub-page a
//! page is about to open at and a sub-page selected and not yet drawn,
//! which could both be set, for different pages.
//!
//! The tab's lasting selection is not here: each tab's channel keeps the
//! page and sub-page it told its channel (`Typing`), and each tab's
//! address keeps the sub-page the viewer chose. This machine is the open
//! page's passage from a choice to its being drawn, in the window at
//! hand.
//!
//! | State | Event | Next | Action |
//! |---|---|---|---|
//! | `Settled` | `ChosenBeforeOpen` | `Pending` | `None` |
//! | `Settled` | `ChoiceWithdrawn` | `Settled` | `None` |
//! | `Settled` | `OpenedIt` | `Settled` | `None` |
//! | `Settled` | `OpenedOther` | `Settled` | `None` |
//! | `Settled` | `SelectedUndrawn` | `Awaiting` | `Say` |
//! | `Settled` | `SelectedDrawn` | `Settled` | `None` |
//! | `Settled` | `Drew` | `Settled` | `None` |
//! | `Pending` | `ChosenBeforeOpen` | `Pending` | `None` |
//! | `Pending` | `ChoiceWithdrawn` | `Settled` | `None` |
//! | `Pending` | `OpenedIt` | `Settled` | `Show` |
//! | `Pending` | `OpenedOther` | `Settled` | `None` |
//! | `Pending` | `SelectedUndrawn` | `Awaiting` | `Say` |
//! | `Pending` | `SelectedDrawn` | `Settled` | `None` |
//! | `Pending` | `Drew` | `Pending` | `None` |
//! | `Awaiting` | `ChosenBeforeOpen` | `Pending` | `None` |
//! | `Awaiting` | `ChoiceWithdrawn` | `Awaiting` | `None` |
//! | `Awaiting` | `OpenedIt` | `Settled` | `None` |
//! | `Awaiting` | `OpenedOther` | `Settled` | `Say` |
//! | `Awaiting` | `SelectedUndrawn` | `Awaiting` | `None` |
//! | `Awaiting` | `SelectedDrawn` | `Settled` | `Say` |
//! | `Awaiting` | `Drew` | `Settled` | `Say` |

/// How far the choice of a sub-page has got.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum SelectionState {
    /// Nothing is on its way: the open page shows what it was asked to.
    #[default]
    Settled,
    /// A page is about to open at a sub-page: a link, an address or the
    /// sub-page list named one.
    Pending,
    /// A sub-page of the open page was selected of a live channel and is
    /// not yet drawn: the strip says it is loading (REQ-1505 item 5).
    Awaiting,
}

/// What can happen to the choice.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SelectionEvent {
    /// A sub-page was named for a page that is not open yet.
    ChosenBeforeOpen,
    /// A page is to open with no sub-page named.
    ChoiceWithdrawn,
    /// The page the pending sub-page is of opened.
    OpenedIt,
    /// Another page opened, or none is open.
    OpenedOther,
    /// A sub-page of the open page was selected and is not drawn yet.
    SelectedUndrawn,
    /// A sub-page of the open page was selected and is drawn, or its
    /// channel is served as files, which says nothing of loading.
    SelectedDrawn,
    /// The decoder drew the sub-page awaited.
    Drew,
}

/// What to do about the choice.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SelectionAction {
    /// Nothing.
    None,
    /// Show the pending sub-page on the page that has just opened.
    Show,
    /// Tell the strip, whose notice of loading starts or ends.
    Say,
}

/// The selection machine's table.
#[allow(clippy::match_same_arms, reason = "one arm for each row of the table, as the engine's machines are written")]
pub(crate) const fn selection_transition(state: SelectionState, event: SelectionEvent) -> (SelectionState, SelectionAction) {
    match (state, event) {
        (SelectionState::Settled, SelectionEvent::ChosenBeforeOpen) => (SelectionState::Pending, SelectionAction::None),
        (SelectionState::Settled, SelectionEvent::ChoiceWithdrawn) => (SelectionState::Settled, SelectionAction::None),
        (SelectionState::Settled, SelectionEvent::OpenedIt) => (SelectionState::Settled, SelectionAction::None),
        (SelectionState::Settled, SelectionEvent::OpenedOther) => (SelectionState::Settled, SelectionAction::None),
        (SelectionState::Settled, SelectionEvent::SelectedUndrawn) => (SelectionState::Awaiting, SelectionAction::Say),
        (SelectionState::Settled, SelectionEvent::SelectedDrawn) => (SelectionState::Settled, SelectionAction::None),
        (SelectionState::Settled, SelectionEvent::Drew) => (SelectionState::Settled, SelectionAction::None),
        (SelectionState::Pending, SelectionEvent::ChosenBeforeOpen) => (SelectionState::Pending, SelectionAction::None),
        (SelectionState::Pending, SelectionEvent::ChoiceWithdrawn) => (SelectionState::Settled, SelectionAction::None),
        (SelectionState::Pending, SelectionEvent::OpenedIt) => (SelectionState::Settled, SelectionAction::Show),
        (SelectionState::Pending, SelectionEvent::OpenedOther) => (SelectionState::Settled, SelectionAction::None),
        (SelectionState::Pending, SelectionEvent::SelectedUndrawn) => (SelectionState::Awaiting, SelectionAction::Say),
        (SelectionState::Pending, SelectionEvent::SelectedDrawn) => (SelectionState::Settled, SelectionAction::None),
        (SelectionState::Pending, SelectionEvent::Drew) => (SelectionState::Pending, SelectionAction::None),
        (SelectionState::Awaiting, SelectionEvent::ChosenBeforeOpen) => (SelectionState::Pending, SelectionAction::None),
        (SelectionState::Awaiting, SelectionEvent::ChoiceWithdrawn) => (SelectionState::Awaiting, SelectionAction::None),
        (SelectionState::Awaiting, SelectionEvent::OpenedIt) => (SelectionState::Settled, SelectionAction::None),
        (SelectionState::Awaiting, SelectionEvent::OpenedOther) => (SelectionState::Settled, SelectionAction::Say),
        (SelectionState::Awaiting, SelectionEvent::SelectedUndrawn) => (SelectionState::Awaiting, SelectionAction::None),
        (SelectionState::Awaiting, SelectionEvent::SelectedDrawn) => (SelectionState::Settled, SelectionAction::Say),
        (SelectionState::Awaiting, SelectionEvent::Drew) => (SelectionState::Settled, SelectionAction::Say),
    }
}

#[cfg(test)]
mod tests {
    use super::SelectionAction as A;
    use super::SelectionEvent as E;
    use super::SelectionState as S;
    use super::selection_transition;

    /// Every row of the selection machine's table, as the module's head gives it.
    #[test]
    fn the_selection_machine_follows_its_table() {
        let table = [
            (S::Settled, E::ChosenBeforeOpen, S::Pending, A::None),
            (S::Settled, E::ChoiceWithdrawn, S::Settled, A::None),
            (S::Settled, E::OpenedIt, S::Settled, A::None),
            (S::Settled, E::OpenedOther, S::Settled, A::None),
            (S::Settled, E::SelectedUndrawn, S::Awaiting, A::Say),
            (S::Settled, E::SelectedDrawn, S::Settled, A::None),
            (S::Settled, E::Drew, S::Settled, A::None),
            (S::Pending, E::ChosenBeforeOpen, S::Pending, A::None),
            (S::Pending, E::ChoiceWithdrawn, S::Settled, A::None),
            (S::Pending, E::OpenedIt, S::Settled, A::Show),
            (S::Pending, E::OpenedOther, S::Settled, A::None),
            (S::Pending, E::SelectedUndrawn, S::Awaiting, A::Say),
            (S::Pending, E::SelectedDrawn, S::Settled, A::None),
            (S::Pending, E::Drew, S::Pending, A::None),
            (S::Awaiting, E::ChosenBeforeOpen, S::Pending, A::None),
            (S::Awaiting, E::ChoiceWithdrawn, S::Awaiting, A::None),
            (S::Awaiting, E::OpenedIt, S::Settled, A::None),
            (S::Awaiting, E::OpenedOther, S::Settled, A::Say),
            (S::Awaiting, E::SelectedUndrawn, S::Awaiting, A::None),
            (S::Awaiting, E::SelectedDrawn, S::Settled, A::Say),
            (S::Awaiting, E::Drew, S::Settled, A::Say),
        ];
        assert_eq!(table.len(), 3 * 7, "every state with every event");
        for (state, event, next, action) in table {
            assert_eq!(selection_transition(state, event), (next, action), "{state:?} on {event:?}");
        }
    }
}
