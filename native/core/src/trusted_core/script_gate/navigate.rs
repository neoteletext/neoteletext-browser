//! A script's navigation, and the tab's own, at the gate (the script
//! API's specification sections 3.3 and 4.2; TODO-7294): a script's
//! `nt.navigate` goes through the checks a link the viewer chose goes
//! through and is carried out the same way, inside its channel unless the
//! viewer's own action is being handled, at a bounded rate; and a
//! channel's scripts are told `navigate` before the tab goes somewhere by
//! the browser's own doing, which no script can cancel.

use std::time::{Duration, Instant};

use protocol::console::Level;
use protocol::decoder::ToDecoder;
use protocol::decoder::script::{NavigationCause, ScriptOrder};
use protocol::rules::PageSub;

use super::super::Core;
use super::super::scripts::ScriptState;
use super::ACTIVATION_WINDOW;

/// The most navigations a channel's scripts may ask in any second
/// (sections 3.3 and 10.5's `MAX_NAVIGATIONS_PER_SECOND`). The decoder
/// holds the same bound over its script host, counted in ticks
/// (`decoder::scripts::limits`, which this program does not link); a
/// test holds the two equal.
pub(super) const MAX_NAVIGATIONS_PER_SECOND: usize = 2;

/// The second navigations are counted over.
pub(super) const NAVIGATION_SECOND: Duration = Duration::from_secs(1);

/// A page and sub-code as a script names them (section 3.3): `NNN`, or
/// `NNN/SSSS` when the sub-code names one sub-page.
fn target_of(page: PageSub) -> String {
    if neoteletext_core::limits::is_sub_code(page.sub) && page.sub != neoteletext_core::limits::SUBCODE_ANY && page.sub != 0 {
        format!("{:03X}/{:04X}", page.page, page.sub)
    } else {
        format!("{:03X}", page.page)
    }
}

impl Core {
    /// A script asked the browser to go to `target` (section 3.3), as its
    /// channel's decoder reports: untrusted, as everything a decoder says
    /// is. It is followed only for the open page of the active tab, while
    /// the channel's scripts run and the window has the keyboard, at most
    /// [`MAX_NAVIGATIONS_PER_SECOND`] times a second whether followed or
    /// not, and to another channel only on the viewer's own action
    /// (`Links::script_navigation`).
    pub(in super::super) fn script_navigate(&mut self, channel: u32, page: PageSub, target: &str) -> Result<(), String> {
        let now = Instant::now();
        let open =
            self.active_channel() == Some(channel) && self.script_state(channel) == ScriptState::Running && self.typing.rules_open(channel) == Some(page);
        if !open || self.typing.keys_away() {
            return Ok(());
        }
        let asked = self.script_gate.navigations.entry(channel).or_default();
        while asked.front().is_some_and(|at| now.saturating_duration_since(*at) >= NAVIGATION_SECOND) {
            asked.pop_front();
        }
        if asked.len() >= MAX_NAVIGATIONS_PER_SECOND {
            self.log(Level::Warning, Some(channel), "a script's navigation was not followed: MAX_NAVIGATIONS_PER_SECOND");
            return Ok(());
        }
        asked.push_back(now);
        // One action of the viewer's lets a script leave once.
        let activated = self.script_gate.activated.remove(&channel).is_some_and(|at| now.saturating_duration_since(at) <= ACTIVATION_WINDOW);
        let at = self.now();
        let Some(followed) = self.with_channel(channel, |links, on| links.script_navigation(target, on, activated, at)) else { return Ok(()) };
        if followed.is_ok() {
            // The script that asked heard its own `navigate` before it
            // asked (section 4.2): the page's leaving is not told again.
            self.script_gate.navigating_itself = Some(channel);
        }
        let done = self.follow(followed, "a script's navigation");
        self.script_gate.navigating_itself = None;
        done
    }

    /// The decoder of `channel` is about to be told that another page of
    /// it is the open one: its scripts hear `navigate` first, with the
    /// cause the browser (section 4.2: "the strip, Back, the browser's own
    /// page stepping"), which no script can cancel. Told for every change
    /// of the open page the scripts did not ask for themselves; a page's
    /// own link and a claimed key's default are told so too, and so are
    /// not cancelable (TODO-7294's owed work says why).
    pub(in super::super) fn script_page_leaving(&mut self, channel: u32, next: PageSub) {
        let leaving = self.script_claims(channel).is_some_and(|claims| claims.running && claims.page != next);
        if !leaving || self.script_state(channel) != ScriptState::Running || self.script_gate.navigating_itself == Some(channel) {
            return;
        }
        self.tell(channel, ToDecoder::Script(ScriptOrder::Navigating { cause: NavigationCause::Browser, target: target_of(next) }));
    }
}
