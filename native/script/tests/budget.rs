//! The budgets across the callbacks of a step and across a second
//! (script-api.md section 3.2 rules 2 and 3; the security review's
//! finding S-05, TODO-7271), and what the bounds on the queue of events
//! keep (section 4.7). The review's `timers.js` made every step of the
//! spike take 590 ms, tick after tick, and the page was never stopped.
mod common;

use std::time::Duration;

use common::{Opens as _, all_lines, claim, key, limited, lines, opening, tick, timed, view, with_events};
use neoteletext_script::LinkError;
use neoteletext_script::limits::{CALLBACK_TIME_LIMIT, SECOND_TIME_BUDGET, STEP_TIME_BUDGET, TICKS_PER_SECOND, TIME_CHECK_MARGIN};
use neoteletext_script::ops::KeyBit;
use protocol::script::{ContextId, EventKind, FailReason};

/// How many turns of a script's busy loop take about `target` on this
/// machine now: measured by running the loop at a top level, since a
/// script has no clock to stop itself by.
fn turns_for(target: Duration) -> u64 {
    const PROBE_TURNS: u64 = 20_000;
    let burn = |turns: u64| {
        let mut container = limited();
        let source = format!("(function () {{ let x = 0; for (let j = 0; j < {turns}; j++) {{ x += j; }} }})();");
        timed(|| container.open_with(opening(ContextId::Page), source.as_bytes()).unwrap()).1.running
    };
    let empty = (0..3).map(|_| burn(0)).min().unwrap();
    let probe = (0..3).map(|_| burn(PROBE_TURNS)).min().unwrap().saturating_sub(empty).max(Duration::from_micros(50));
    (u128::from(PROBE_TURNS) * target.as_nanos() / probe.as_nanos()).max(1).try_into().unwrap()
}

/// TODO-7271: with many timers each busy for a few milliseconds every
/// tick, no step runs past its budget by more than the one callback that
/// was running when the budget was used; the callbacks that could not
/// start stay due; and the container is stopped once it has used more
/// than the second's budget, here well within a second of ticks.
#[test]
fn busy_timers_are_held_to_the_step_and_stopped_by_the_second() {
    common::unless_starved(|| {
        // About 4 ms a callback: a third of the step's budget, a twelfth of a callback's limit.
        let turns = turns_for(Duration::from_millis(4));
        let source = format!(
            "let fired = 0; for (let i = 0; i < 8; i++) setInterval(() => {{ fired++; let x = 0; for (let j = 0; j < {turns}; j++) {{ x += j; }} }}, 0);
             nt.addEventListener('tick', () => console.log('tick ' + nt.now() + ' fired ' + fired));"
        );
        let mut container = limited();
        let mut open = opening(ContextId::Page);
        claim(&mut open, &[]);
        container.open_with(open, source.as_bytes()).unwrap();
        let mut log = Vec::new();
        let mut total = Duration::ZERO;
        let mut stopped_at = None;
        for at in 0..TICKS_PER_SECOND {
            let before = container.costs().callbacks;
            let (batch, took) = timed(|| container.step(&tick(u32::try_from(at).unwrap(), at)));
            let batch = batch.unwrap();
            let ran = container.costs().callbacks - before;
            let used = container.costs().last_step;
            total += used;
            // A step starts nothing once its budget is used: it runs over by
            // at most the callback then running.
            let longest = container.costs().longest_callback;
            assert!(used <= STEP_TIME_BUDGET + longest, "tick {at}: the step used {used:?} with a longest callback of {longest:?}");
            assert!(took.running <= STEP_TIME_BUDGET + longest + TIME_CHECK_MARGIN, "tick {at}: the step took {took:?}");
            assert!(longest < CALLBACK_TIME_LIMIT, "a callback took {longest:?}: this machine is too slow now for the test to mean anything");
            // Eight timers of about 4 ms each do not all fit a 12 ms budget.
            if at > 0 {
                assert!(ran < 8, "tick {at}: {ran} callbacks ran in one step");
            }
            log.extend(lines(&batch));
            if view(&batch).states[0].stopped.is_some() {
                assert_eq!(view(&batch).states[0].stopped, Some(FailReason::Time));
                stopped_at = Some(at);
                break;
            }
        }
        let stopped_at = stopped_at.unwrap_or_else(|| panic!("the container used {total:?} in a second of ticks and was not stopped"));
        assert!(log.iter().any(|line| line.starts_with("SECOND_TIME_BUDGET: ")), "{log:?}");
        assert!(
            total > SECOND_TIME_BUDGET && total <= SECOND_TIME_BUDGET + STEP_TIME_BUDGET + CALLBACK_TIME_LIMIT,
            "stopped at tick {stopped_at} after {total:?}"
        );
        // The tick event itself could not start while the timers used the
        // budget: no tick was heard after the first.
        assert!(!log.iter().any(|line| line.starts_with("tick 1 ")), "{log:?}");
        assert!(matches!(container.step(&tick(999, 999)), Err(LinkError::Stopped(FailReason::Time, _))));
    });
}

/// Section 3.2 rule 2 and section 4.7: input a step has no time for stays
/// queued for the next step, in order, with its default taken meanwhile
/// (its verdict says nothing was cancelled); a lost release is delivered
/// first and is never held back; a tick that could not start is counted
/// in the next one's `missed`.
#[test]
fn input_a_step_has_no_time_for_waits_but_lost_releases_never_do() {
    common::unless_starved(|| {
        // The handling of "a" is to use the step's whole budget, by a
        // loop whose turns were counted a moment earlier: twice the budget
        // is asked for, so that it still does when the processor runs the
        // loop up to twice as fast as when it was measured, and stays
        // within a callback's limit when it runs it up to twice as slow.
        const _: () = assert!(STEP_TIME_BUDGET.as_nanos() * 4 <= CALLBACK_TIME_LIMIT.as_nanos());
        let turns = turns_for(STEP_TIME_BUDGET * 2);
        let source = format!(
            "nt.addEventListener('keydown', (e) => {{ console.log('down ' + e.key + ' at ' + nt.now()); e.preventDefault(); if (e.key === 'a') {{ let x = 0; for (let j = 0; j < {turns}; j++) {{ x += j; }} }} }});
             nt.addEventListener('keyup', (e) => console.log('up ' + e.key + ' lost ' + e.lost + ' at ' + nt.now()));
             nt.addEventListener('tick', (e) => console.log('tick ' + e.tick + ' missed ' + e.missed));"
        );
        let mut container = limited();
        let mut open = opening(ContextId::Page);
        claim(&mut open, &["a", "Enter", "ArrowLeft"]);
        container.open_with(open, source.as_bytes()).unwrap();
        assert_eq!(lines(&container.step(&tick(1, 0)).unwrap()), vec!["tick 0 missed 0"]);
        let events = vec![
            key(EventKind::KeyDown, "a", 0),
            key(EventKind::KeyDown, "Enter", 0),
            key(EventKind::KeyDown, "ArrowLeft", KeyBit::REPEAT_OR_LOST.number()),
            key(EventKind::KeyUp, "Enter", KeyBit::REPEAT_OR_LOST.number()),
        ];
        let first = container.step(&with_events(2, 1, events)).unwrap();
        // The lost release came last and is delivered first; "a" uses the
        // step's budget; Enter, the repeat and the tick wait.
        assert_eq!(lines(&first), vec!["up Enter lost true at 1", "down a at 1"]);
        // "a" was cancelled and heard; the release was heard; the two that
        // waited have no verdict, so their defaults are taken.
        assert_eq!(view(&first).verdicts, vec![3, 0, 0, 2]);
        let second = container.step(&tick(3, 2)).unwrap();
        // The repeat that waited is dropped; Enter is heard a step late; the
        // tick says one was missed.
        assert_eq!(lines(&second), vec!["down Enter at 2", "tick 2 missed 1"]);
        assert_eq!(view(&second).verdicts, Vec::<u8>::new());
        assert_eq!(all_lines(&[container.step(&tick(4, 3))]), vec!["tick 3 missed 0"]);
    });
}
