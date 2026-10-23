//! The trusted strip's questions (`hyperlinks.md` section 12, requirements
//! 8.6, 17.7 and 17.9): what following a link asks, and the answers.

use protocol::links::{Prompt, PromptKind};

use super::target::HTTPS;
use super::{Destination, Follow, Links, Refusal};

/// The strip's questions accept no answer in their first half second
/// (`hyperlinks.md` section 12).
pub const PROMPT_DELAY_MILLISECONDS: u64 = 500;
/// At most one web page is handed to the system's web browser every two
/// seconds (requirement 17.9).
pub const HANDOFF_INTERVAL_MILLISECONDS: u64 = 2_000;

/// A question the trusted strip is asking.
pub(super) struct Asking {
    prompt: Prompt,
    asked: u64,
    destination: Destination,
}

impl Links {
    /// A page of this channel opens at once; anything else is asked about
    /// in the trusted strip first, and a web page only while the window has
    /// the keyboard focus.
    pub(super) fn follow(&mut self, destination: Destination, now: u64) -> Result<Follow, Refusal> {
        let (kind, secure) = match &destination {
            Destination::Page(page) => return Ok(Follow::Page(*page)),
            Destination::Channel(address) => (PromptKind::Leave, address.encrypted),
            Destination::Web(address) => {
                if self.unfocused {
                    return Err(Refusal::Unfocused);
                }
                (PromptKind::Web, address.starts_with(HTTPS))
            }
        };
        self.next_prompt = self.next_prompt.wrapping_add(1);
        let prompt = Prompt { id: self.next_prompt, kind, destination: destination.name(), secure };
        // A new question replaces the one before; questions never stack.
        self.asking = Some(Asking { prompt: prompt.clone(), asked: now, destination });
        Ok(Follow::Ask(prompt))
    }

    /// The strip's question now, if any.
    #[must_use]
    pub fn prompt(&self) -> Option<Prompt> {
        self.asking.as_ref().map(|asking| asking.prompt.clone())
    }

    /// Yes to the strip's question `id`. Refused within half a second of the
    /// question, for another question, and, for a web page, without the
    /// keyboard focus or within two seconds of the last web page handed over.
    ///
    /// # Errors
    /// Why nothing is followed.
    pub fn confirm(&mut self, id: u32, now: u64) -> Result<Follow, Refusal> {
        let asking = self.asking.as_ref().filter(|asking| asking.prompt.id == id).ok_or(Refusal::NoPrompt)?;
        if now.saturating_sub(asking.asked) < PROMPT_DELAY_MILLISECONDS {
            return Err(Refusal::TooSoon);
        }
        if let Destination::Web(_) = asking.destination {
            if self.unfocused {
                return Err(Refusal::Unfocused);
            }
            if self.last_handoff.is_some_and(|last| now.saturating_sub(last) < HANDOFF_INTERVAL_MILLISECONDS) {
                return Err(Refusal::TooOften);
            }
        }
        let asking = self.asking.take().ok_or(Refusal::NoPrompt)?;
        Ok(match asking.destination {
            Destination::Page(page) => Follow::Page(page),
            Destination::Channel(address) => Follow::Leave(address),
            Destination::Web(address) => {
                self.last_handoff = Some(now);
                Follow::HandOff(address)
            }
        })
    }

    /// No to the strip's question `id`; returns whether it was the question
    /// asked. Declining has no other effect (requirement 8.6).
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
