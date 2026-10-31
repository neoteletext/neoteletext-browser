//! The trusted strip's question (`hyperlinks.md` section 12, requirements
//! 17.7 and 17.9): what following a `web:` link asks, and the answers. A
//! link to another channel asks nothing (REQ-1670).

use protocol::links::Prompt;

use super::target::HTTPS;
use super::{Destination, Follow, Links, Refusal};

/// The strip's question accepts no answer in its first half second
/// (`hyperlinks.md` section 12).
pub const PROMPT_DELAY_MILLISECONDS: u64 = 500;
/// At most one web page is handed to the system's web browser every two
/// seconds (requirement 17.9).
pub const HANDOFF_INTERVAL_MILLISECONDS: u64 = 2_000;

/// A question the trusted strip is asking.
pub(super) struct Asking {
    prompt: Prompt,
    asked: u64,
    address: String,
}

impl Links {
    /// A page of this channel or another channel opens at once (REQ-1670);
    /// a web page is asked about in the trusted strip first, and only while
    /// the window has the keyboard focus.
    pub(super) fn follow(&mut self, destination: Destination, now: u64) -> Result<Follow, Refusal> {
        let name = destination.name();
        let address = match destination {
            Destination::Page(page) => return Ok(Follow::Page(page)),
            Destination::SubPage(page, subpage) => return Ok(Follow::SubPage(page, subpage)),
            Destination::Channel(address) => return Ok(Follow::Channel(address)),
            Destination::Web(address) => address,
        };
        if self.unfocused {
            return Err(Refusal::Unfocused);
        }
        self.next_prompt = self.next_prompt.wrapping_add(1);
        let prompt = Prompt { id: self.next_prompt, destination: name, secure: address.starts_with(HTTPS) };
        // A new question replaces the one before; questions never stack.
        self.asking = Some(Asking { prompt: prompt.clone(), asked: now, address });
        Ok(Follow::Ask(prompt))
    }

    /// The strip's question now, if any.
    #[must_use]
    pub fn prompt(&self) -> Option<Prompt> {
        self.asking.as_ref().map(|asking| asking.prompt.clone())
    }

    /// Yes to the strip's question `id`. Refused within half a second of the
    /// question, for another question, without the keyboard focus, or within
    /// two seconds of the last web page handed over.
    ///
    /// # Errors
    /// Why nothing is followed.
    pub fn confirm(&mut self, id: u32, now: u64) -> Result<Follow, Refusal> {
        let asking = self.asking.as_ref().filter(|asking| asking.prompt.id == id).ok_or(Refusal::NoPrompt)?;
        if now.saturating_sub(asking.asked) < PROMPT_DELAY_MILLISECONDS {
            return Err(Refusal::TooSoon);
        }
        if self.unfocused {
            return Err(Refusal::Unfocused);
        }
        if self.last_handoff.is_some_and(|last| now.saturating_sub(last) < HANDOFF_INTERVAL_MILLISECONDS) {
            return Err(Refusal::TooOften);
        }
        let asking = self.asking.take().ok_or(Refusal::NoPrompt)?;
        self.last_handoff = Some(now);
        Ok(Follow::HandOff(asking.address))
    }

    /// No to the strip's question `id`; returns whether it was the question
    /// asked. Declining has no other effect.
    pub fn cancel(&mut self, id: u32) -> bool {
        let asked = self.asking.as_ref().is_some_and(|asking| asking.prompt.id == id);
        if asked {
            self.asking = None;
        }
        asked
    }

    /// Forgets the strip's question, as when the tab or its channel changes.
    pub fn forget_prompt(&mut self) -> bool {
        self.asking.take().is_some()
    }
}
