//! Pointing at a link and following one (`hyperlinks.md` sections 10 and
//! 11, requirements 17.6 and 17.7): where the link under the pointer
//! leads, as the strip shows it, and the rules by which a click, a typed
//! number or a start-page bookmark follows.

use interface_core::canvas::PageKey;
use protocol::links::{Pointing, Spot};

use super::{DWELL_MILLISECONDS, Destination, Follow, Link, Links, Refusal, Target};

impl Links {
    /// The link at a cell of an image the trusted core sent.
    fn link_at(&self, spot: &Spot) -> Result<&Link, Refusal> {
        let image = self.pages.get(&spot.key).and_then(|state| state.images.iter().find(|image| image.version == spot.version)).ok_or(Refusal::Stale)?;
        image.links.iter().find(|link| link.run.contains(spot.column, spot.row)).ok_or(Refusal::NotALink)
    }

    /// The pointer or the keyboard's focus is on a cell, or on none: the
    /// strip shows where the link there leads (requirement 17.6). Returns
    /// what the strip shows.
    pub fn point(&mut self, spot: Option<Spot>) -> Option<Pointing> {
        self.pointed = spot.and_then(|spot| Some((spot, self.link_at(&spot).ok()?.destination.clone())));
        self.pointing()
    }

    /// The links changed: the strip's destination follows them. Returns
    /// what the strip shows when that changed.
    pub fn repoint(&mut self) -> Option<Option<Pointing>> {
        let before = self.pointed.as_ref().map(|(_, destination)| destination.clone());
        let spot = self.pointed.as_ref().map(|(spot, _)| *spot);
        self.point(spot);
        let after = self.pointed.as_ref().map(|(_, destination)| destination.clone());
        (before != after).then(|| self.pointing())
    }

    fn pointing(&self) -> Option<Pointing> {
        self.pointed.as_ref().map(|(_, destination)| Pointing { kind: destination.kind(), text: destination.describe() })
    }

    /// Asks the strip's question for `destination`, as a link to it would:
    /// what a start-page bookmark opened in the page area leads to
    /// (requirement 4.5, finding F-67).
    ///
    /// # Errors
    /// Why it is not asked.
    pub fn ask(&mut self, destination: Destination, now: u64) -> Result<Follow, Refusal> {
        self.follow(destination, now)
    }

    /// A click on a cell. It follows a link only when the click is on the
    /// open page of the active channel, in an image the trusted core sent
    /// lately, on a link whose destination the strip is showing, and which
    /// has led there for at least half a second (requirement 17.7).
    ///
    /// # Errors
    /// Why nothing is followed.
    pub fn click(&mut self, spot: Spot, active: Option<u32>, now: u64) -> Result<Follow, Refusal> {
        if self.opened != Some(spot.key) || active != Some(spot.key.channel) {
            return Err(Refusal::NotOpen);
        }
        let link = self.link_at(&spot)?;
        if self.pointed.as_ref().is_none_or(|(_, shown)| *shown != link.destination) {
            return Err(Refusal::Unshown);
        }
        if now.saturating_sub(link.since) < DWELL_MILLISECONDS {
            return Err(Refusal::Changed);
        }
        let destination = link.destination.clone();
        self.follow(destination, now)
    }

    /// A page number typed while `opened` was open, which the page or the
    /// channel links elsewhere. A typed number never leads to a web page.
    ///
    /// # Errors
    /// Why nothing is followed.
    pub fn typed(&mut self, number: u16, opened: Option<(PageKey, u32)>, active: Option<u32>, now: u64) -> Result<Follow, Refusal> {
        let channel = active.ok_or(Refusal::NotOpen)?;
        let page_link = match opened {
            Some((key, version)) => {
                if self.opened != Some(key) || key.channel != channel {
                    return Err(Refusal::NotOpen);
                }
                let image = self.pages.get(&key).and_then(|state| state.images.iter().find(|image| image.version == version)).ok_or(Refusal::Stale)?;
                image.numbers.iter().find(|(linked, _)| *linked == number).map(|(_, destination)| destination.clone())
            }
            None => None,
        };
        let destination = if let Some(destination) = page_link {
            destination
        } else {
            {
                let descriptor = self.descriptors.get(&channel).ok_or(Refusal::NotALink)?;
                let target = descriptor.alias(number).ok_or(Refusal::NotALink)?;
                // An alias's own channel is checked when its links are
                // decided; here only another channel or this one's page.
                match target {
                    Target::Page(page) => Destination::Page(*page),
                    Target::Service { name, page } => {
                        let address = descriptor.service(name).ok_or(Refusal::NotALink)?;
                        Destination::Channel(address.with_page(*page).ok_or(Refusal::NotALink)?)
                    }
                    Target::Channel(address) => Destination::Channel(address.clone()),
                    Target::Web(_) => return Err(Refusal::NotALink),
                }
            }
        };
        if matches!(destination, Destination::Web(_)) {
            return Err(Refusal::NotALink);
        }
        self.follow(destination, now)
    }
}
