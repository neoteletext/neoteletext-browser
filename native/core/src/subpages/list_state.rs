//! The life of one page's list of sub-pages in the trusted core, as a
//! state machine (DEC-6358 item 4), written as the engine writes its own
//! (`neoteletext-core`'s `fsm.rs`): an enum of states and of events, and
//! one transition function with an arm for every pair, none a catch-all.
//! It replaces the separate sets the core kept for "a list answered" and
//! "the page has no list", which nothing held in step.
//!
//! What the channel's index declares of a page's count is not a state of
//! its list: it is kept beside the machine, since an index can speak of a
//! page in any state of its list.
//!
//! | State | Event | Next |
//! |---|---|---|
//! | `Unknown` | `Asked` | `Fetching` |
//! | `Unknown` | `Arrived` | `Held` |
//! | `Unknown` | `Absent` | `Missing` |
//! | `Unknown` | `PartFailed` | `Unknown` |
//! | `Unknown` | `Evicted` | `Unknown` |
//! | `Unknown` | `Reopened` | `Unknown` |
//! | `Fetching` | `Asked` | `Fetching` |
//! | `Fetching` | `Arrived` | `Held` |
//! | `Fetching` | `Absent` | `Missing` |
//! | `Fetching` | `PartFailed` | `Failed` |
//! | `Fetching` | `Evicted` | `Fetching` |
//! | `Fetching` | `Reopened` | `Fetching` |
//! | `Held` | `Asked` | `Held` |
//! | `Held` | `Arrived` | `Held` |
//! | `Held` | `Absent` | `Missing` |
//! | `Held` | `PartFailed` | `Held` |
//! | `Held` | `Evicted` | `Forgotten` |
//! | `Held` | `Reopened` | `Held` |
//! | `Forgotten` | `Asked` | `Fetching` |
//! | `Forgotten` | `Arrived` | `Held` |
//! | `Forgotten` | `Absent` | `Missing` |
//! | `Forgotten` | `PartFailed` | `Forgotten` |
//! | `Forgotten` | `Evicted` | `Forgotten` |
//! | `Forgotten` | `Reopened` | `Forgotten` |
//! | `Missing` | `Asked` | `Missing` |
//! | `Missing` | `Arrived` | `Held` |
//! | `Missing` | `Absent` | `Missing` |
//! | `Missing` | `PartFailed` | `Missing` |
//! | `Missing` | `Evicted` | `Missing` |
//! | `Missing` | `Reopened` | `Missing` |
//! | `Failed` | `Asked` | `Fetching` |
//! | `Failed` | `Arrived` | `Held` |
//! | `Failed` | `Absent` | `Missing` |
//! | `Failed` | `PartFailed` | `Failed` |
//! | `Failed` | `Evicted` | `Failed` |
//! | `Failed` | `Reopened` | `Unknown` |

/// Where a page's list of sub-pages stands.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ListState {
    /// Nothing is known of a list for the page.
    #[default]
    Unknown,
    /// A part of the list was asked for and none is held.
    Fetching,
    /// At least a part of the list is held.
    Held,
    /// The list was held and forgotten to keep within the budget of rows;
    /// its count is kept.
    Forgotten,
    /// The channel has no list for the page: never asked for again, and
    /// its sub-pages are what its decoder holds.
    Missing,
    /// The list was asked for and could not be fetched; asked for again
    /// when its page next opens.
    Failed,
}

/// What can happen to a page's list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ListEvent {
    /// A part of it was asked of the fetcher.
    Asked,
    /// A part of it arrived.
    Arrived,
    /// Its first part does not exist: the page has no list.
    Absent,
    /// A part of it could not be fetched.
    PartFailed,
    /// It was forgotten to keep within the budget.
    Evicted,
    /// Its page opened again.
    Reopened,
}

/// The list machine's table.
#[must_use]
#[allow(clippy::match_same_arms, reason = "one arm for each row of the table, as the engine's machines are written")]
pub const fn list_transition(state: ListState, event: ListEvent) -> ListState {
    match (state, event) {
        (ListState::Unknown, ListEvent::Asked) => ListState::Fetching,
        (ListState::Unknown, ListEvent::Arrived) => ListState::Held,
        (ListState::Unknown, ListEvent::Absent) => ListState::Missing,
        (ListState::Unknown, ListEvent::PartFailed) => ListState::Unknown,
        (ListState::Unknown, ListEvent::Evicted) => ListState::Unknown,
        (ListState::Unknown, ListEvent::Reopened) => ListState::Unknown,
        (ListState::Fetching, ListEvent::Asked) => ListState::Fetching,
        (ListState::Fetching, ListEvent::Arrived) => ListState::Held,
        (ListState::Fetching, ListEvent::Absent) => ListState::Missing,
        (ListState::Fetching, ListEvent::PartFailed) => ListState::Failed,
        (ListState::Fetching, ListEvent::Evicted) => ListState::Fetching,
        (ListState::Fetching, ListEvent::Reopened) => ListState::Fetching,
        (ListState::Held, ListEvent::Asked) => ListState::Held,
        (ListState::Held, ListEvent::Arrived) => ListState::Held,
        (ListState::Held, ListEvent::Absent) => ListState::Missing,
        (ListState::Held, ListEvent::PartFailed) => ListState::Held,
        (ListState::Held, ListEvent::Evicted) => ListState::Forgotten,
        (ListState::Held, ListEvent::Reopened) => ListState::Held,
        (ListState::Forgotten, ListEvent::Asked) => ListState::Fetching,
        (ListState::Forgotten, ListEvent::Arrived) => ListState::Held,
        (ListState::Forgotten, ListEvent::Absent) => ListState::Missing,
        (ListState::Forgotten, ListEvent::PartFailed) => ListState::Forgotten,
        (ListState::Forgotten, ListEvent::Evicted) => ListState::Forgotten,
        (ListState::Forgotten, ListEvent::Reopened) => ListState::Forgotten,
        (ListState::Missing, ListEvent::Asked) => ListState::Missing,
        (ListState::Missing, ListEvent::Arrived) => ListState::Held,
        (ListState::Missing, ListEvent::Absent) => ListState::Missing,
        (ListState::Missing, ListEvent::PartFailed) => ListState::Missing,
        (ListState::Missing, ListEvent::Evicted) => ListState::Missing,
        (ListState::Missing, ListEvent::Reopened) => ListState::Missing,
        (ListState::Failed, ListEvent::Asked) => ListState::Fetching,
        (ListState::Failed, ListEvent::Arrived) => ListState::Held,
        (ListState::Failed, ListEvent::Absent) => ListState::Missing,
        (ListState::Failed, ListEvent::PartFailed) => ListState::Failed,
        (ListState::Failed, ListEvent::Evicted) => ListState::Failed,
        (ListState::Failed, ListEvent::Reopened) => ListState::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::ListEvent as E;
    use super::ListState as S;
    use super::list_transition;

    /// Every row of the list machine's table, as the module's head gives it.
    #[test]
    fn the_list_machine_follows_its_table() {
        let table = [
            (S::Unknown, E::Asked, S::Fetching),
            (S::Unknown, E::Arrived, S::Held),
            (S::Unknown, E::Absent, S::Missing),
            (S::Unknown, E::PartFailed, S::Unknown),
            (S::Unknown, E::Evicted, S::Unknown),
            (S::Unknown, E::Reopened, S::Unknown),
            (S::Fetching, E::Asked, S::Fetching),
            (S::Fetching, E::Arrived, S::Held),
            (S::Fetching, E::Absent, S::Missing),
            (S::Fetching, E::PartFailed, S::Failed),
            (S::Fetching, E::Evicted, S::Fetching),
            (S::Fetching, E::Reopened, S::Fetching),
            (S::Held, E::Asked, S::Held),
            (S::Held, E::Arrived, S::Held),
            (S::Held, E::Absent, S::Missing),
            (S::Held, E::PartFailed, S::Held),
            (S::Held, E::Evicted, S::Forgotten),
            (S::Held, E::Reopened, S::Held),
            (S::Forgotten, E::Asked, S::Fetching),
            (S::Forgotten, E::Arrived, S::Held),
            (S::Forgotten, E::Absent, S::Missing),
            (S::Forgotten, E::PartFailed, S::Forgotten),
            (S::Forgotten, E::Evicted, S::Forgotten),
            (S::Forgotten, E::Reopened, S::Forgotten),
            (S::Missing, E::Asked, S::Missing),
            (S::Missing, E::Arrived, S::Held),
            (S::Missing, E::Absent, S::Missing),
            (S::Missing, E::PartFailed, S::Missing),
            (S::Missing, E::Evicted, S::Missing),
            (S::Missing, E::Reopened, S::Missing),
            (S::Failed, E::Asked, S::Fetching),
            (S::Failed, E::Arrived, S::Held),
            (S::Failed, E::Absent, S::Missing),
            (S::Failed, E::PartFailed, S::Failed),
            (S::Failed, E::Evicted, S::Failed),
            (S::Failed, E::Reopened, S::Unknown),
        ];
        assert_eq!(table.len(), 6 * 6, "every state with every event");
        for (state, event, next) in table {
            assert_eq!(list_transition(state, event), next, "{state:?} on {event:?}");
        }
    }
}
