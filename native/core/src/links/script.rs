//! Where a script's `nt.navigate` leads (the script API's specification,
//! `neoteletext-common/docs/script-api.md`, section 3.3; TODO-7294): a
//! target a script wrote is page data like a link's, classified against
//! the channel it is on by the same rules, and followed by the same rule
//! as a link the viewer chose, with one more: a script leaves its channel
//! only on the viewer's own action.

use super::destination::Descriptor;
use super::target::{channel_address, page, subpage_of};
use super::{Channel, Destination, Follow, Links, Refusal, Target};

/// What separates an address's scheme from the rest of it.
const SCHEME_SEPARATOR: &str = "://";
/// What separates a page from its sub-page in a script's target
/// (section 3.3: `"100/0002"`).
const SUBPAGE_SEPARATOR: char = '/';

/// A script's target, parsed: exactly a page as three hexadecimal digits,
/// a page and a sub-page as `NNN/SSSS`, or an `http:` or `https:` address
/// the address field would open. Anything else is refused, never
/// repaired; a script never names a web page for the system's browser, a
/// file or a service.
fn script_target(text: &str) -> Option<Target> {
    if text.contains(SCHEME_SEPARATOR) {
        return channel_address(text).map(Target::Channel);
    }
    match text.split_once(SUBPAGE_SEPARATOR) {
        None => page(text).map(Target::Page),
        Some((number, subpage)) => Some(Target::SubPage(page(number)?, subpage_of(subpage)?)),
    }
}

impl Links {
    /// A script of `channel`'s open page asked to go to `target`
    /// (section 3.3). A page of its own channel is followed at once, as a
    /// link to it is. Another channel is followed only when `activated`:
    /// the viewer's key press or pointer release was being handled by the
    /// script as it asked (HTML's user activation), since a script is no
    /// viewer and a link's dwell and the strip's showing of its
    /// destination cannot be asked of it.
    ///
    /// # Errors
    /// Why nothing is followed.
    pub fn script_navigation(&mut self, target: &str, channel: &Channel<'_>, activated: bool, now: u64) -> Result<Follow, Refusal> {
        let target = script_target(target).ok_or(Refusal::NotALink)?;
        // No descriptor's aliases or services: a script names a page or an
        // address, and its own channel's pages are told from another
        // channel by the one rule a link's are.
        let destination = Descriptor::default().resolve(&target, channel).ok_or(Refusal::NotALink)?;
        if matches!(destination, Destination::Channel(_)) && !activated {
            return Err(Refusal::Unasked);
        }
        self.follow(destination, now)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "a test")]

    use super::*;
    use crate::address;

    fn follow(target: &str, activated: bool) -> Result<Follow, Refusal> {
        let own = address::parse("https://example.test/chan/").unwrap();
        Links::default().script_navigation(target, &Channel { id: 1, address: &own }, activated, 10_000)
    }

    /// Section 3.3: a page of the script's own channel is followed at any
    /// time, by its number, its number and sub-page, or its own address.
    #[test]
    fn a_page_of_the_scripts_own_channel_is_followed_without_the_viewer() {
        assert_eq!(follow("1A0", false), Ok(Follow::Page(0x1A0)));
        assert_eq!(follow("100/0002", false), Ok(Follow::SubPage(0x100, 2)));
        assert_eq!(follow("https://example.test/chan/200", false), Ok(Follow::Page(0x200)));
    }

    /// Section 3.3: another channel only on the viewer's own action, and
    /// the same origin outside the channel's folder is another channel
    /// (DEC-7264: the unit of isolation is the channel).
    #[test]
    fn another_channel_is_followed_only_on_the_viewers_action() {
        for elsewhere in ["https://elsewhere.test/", "https://example.test/other/100", "https://example.test/", "http://example.test/chan/100"] {
            assert_eq!(follow(elsewhere, false), Err(Refusal::Unasked), "{elsewhere}");
            assert!(matches!(follow(elsewhere, true), Ok(Follow::Channel(_))), "{elsewhere}");
        }
    }

    /// Section 3.3: only a page, a page and sub-page, or an `http:` or
    /// `https:` address; nothing a script writes hands an address to the
    /// system's web browser, opens a file, or is repaired into a target.
    #[test]
    fn nothing_else_is_a_scripts_target() {
        let refused = [
            "",
            "10",
            "1000",
            "9FF",
            "100/",
            "100/2",
            "100/3F7F",
            "100:0002",
            "news:100",
            "web:https://example.test/",
            "file:///etc/passwd",
            "ftp://example.test/",
            "javascript://example.test/%0Aalert(1)",
            "https://example.test/chan/ 100",
            "https://exa\u{7}mple.test/",
        ];
        for target in refused {
            assert_eq!(follow(target, true), Err(Refusal::NotALink), "{target:?}");
        }
    }
}
