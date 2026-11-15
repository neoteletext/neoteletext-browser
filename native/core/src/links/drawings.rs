//! Links on the numbered drawings of a page drawn in ReGIS (REQ-6376): a
//! channel names the drawings of a sub-page's picture that are links and
//! where each leads (the engine's `DRAWING_LINKS` record), and a click on
//! one is followed by the rules of every link. Each target is parsed and
//! resolved as a text link's is, its destination is shown in the strip
//! while the pointer is on its drawing, and a click follows only a
//! destination that is being shown and has led there for the dwell time.

use interface_core::canvas::PageKey;
use protocol::links::Pointing;
use protocol::regis::MAX_REGIS_LINKS;

use super::destination::Descriptor;
use super::{Channel, DWELL_MILLISECONDS, Destination, Follow, Links, Refusal, parse_target};

/// The most pages whose drawing links are kept: more than a window shows
/// of one channel at once, as for their pick records. One more forgets the
/// lowest page of the lowest channel, whose channel names them again when
/// the page is next opened.
const MAX_PAGES: usize = 16;

/// A numbered drawing of a picture, by its page, its sub-code and its number.
pub type Drawing = (PageKey, u16, u16);

/// One drawing that is a link.
struct DrawingLink {
    drawing: u16,
    destination: Destination,
    /// When it began to lead there, in milliseconds.
    since: u64,
}

/// The drawing links of the picture of one sub-page of a page.
#[derive(Default)]
pub(super) struct DrawingLinks {
    sub: u16,
    links: Vec<DrawingLink>,
}

impl Links {
    /// The decoder read a sub-page's drawing links: all of them, replacing
    /// those it had. A target that does not parse or leads nowhere is
    /// dropped, as a text link's is; a drawing that led to the same place
    /// before keeps the time it began to.
    pub fn found_drawings(&mut self, key: PageKey, sub: u16, found: &[(u16, String)], channel: &Channel<'_>, now: u64) {
        let empty = Descriptor::default();
        let descriptor = self.descriptors.get(&key.channel).unwrap_or(&empty);
        let before = self.drawings.get(&key).filter(|held| held.sub == sub);
        let links = found
            .iter()
            .take(MAX_REGIS_LINKS)
            .filter_map(|(drawing, target)| {
                let destination = parse_target(target).and_then(|target| descriptor.resolve(&target, channel))?;
                let led = before.and_then(|held| held.links.iter().find(|link| link.drawing == *drawing && link.destination == destination));
                Some(DrawingLink { drawing: *drawing, since: led.map_or(now, |link| link.since), destination })
            })
            .collect();
        if !self.drawings.contains_key(&key) && self.drawings.len() >= MAX_PAGES {
            self.drawings.pop_first();
        }
        self.drawings.insert(key, DrawingLinks { sub, links });
    }

    fn drawing_link(&self, (key, sub, drawing): Drawing) -> Option<&DrawingLink> {
        self.drawings.get(&key).filter(|held| held.sub == sub)?.links.iter().find(|link| link.drawing == drawing)
    }

    /// Whether a drawing is a link.
    #[must_use]
    pub fn is_drawing_link(&self, drawing: Drawing) -> bool {
        self.drawing_link(drawing).is_some()
    }

    /// The pointer is on a drawing, or on none: the strip shows where a
    /// linked drawing leads. Returns what the strip shows, when that
    /// changed.
    pub fn point_drawing(&mut self, at: Option<Drawing>) -> Option<Option<Pointing>> {
        let wanted = at.and_then(|at| Some((at, self.drawing_link(at)?.destination.clone())));
        if wanted == self.pointed_drawing {
            return None;
        }
        self.pointed_drawing = wanted;
        Some(self.pointing())
    }

    /// A click on a drawing. It follows a link only when the drawing is of
    /// the open page of the active channel, is a link whose destination
    /// the strip is showing, and has led there for at least half a second:
    /// the rules of a click on a text link (requirement 17.7).
    ///
    /// # Errors
    /// Why nothing is followed.
    pub fn click_drawing(&mut self, drawing: Drawing, active: Option<u32>, now: u64) -> Result<Follow, Refusal> {
        let (key, _, _) = drawing;
        if self.opened != Some(key) || active != Some(key.channel) {
            return Err(Refusal::NotOpen);
        }
        let link = self.drawing_link(drawing).ok_or(Refusal::NotALink)?;
        if self.pointed_drawing.as_ref().is_none_or(|(shown, destination)| *shown != drawing || *destination != link.destination) {
            return Err(Refusal::Unshown);
        }
        if now.saturating_sub(link.since) < DWELL_MILLISECONDS {
            return Err(Refusal::Changed);
        }
        let destination = link.destination.clone();
        self.follow(destination, now)
    }
}
