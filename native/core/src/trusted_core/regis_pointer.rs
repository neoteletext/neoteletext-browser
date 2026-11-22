//! The pointer on a page drawn in ReGIS: the trusted core carries out
//! what `neoteletext::regis_pointer` decides to say (REQ-6374, REQ-6375,
//! REQ-6377).

use interface_core::canvas::PageKey;
use neoteletext::regis_pointer::Say;
use protocol::ToInterface;
use protocol::decoder::ToDecoder;
use protocol::regis::{PictureTarget, RegisPointer};

use super::Core;

impl Core {
    /// The interface said where the pointer is on a ReGIS or NAPLPS
    /// picture. A pixel the picture held for the page has not is off it.
    pub(super) fn regis_pointer(&mut self, mut pointer: RegisPointer) -> Result<(), String> {
        pointer.at = pointer.at.filter(|at| self.regis.has_pixel(pointer.key, *at));
        self.regis_pointing.pointer(pointer, &self.regis);
        self.regis_repoint()
    }

    /// Tells the interface the targets of `key`'s picture when they changed
    /// (renderer parity PAR-30): every drawing or object its pick record
    /// names, with its rectangle, in reading order, and whether it is a
    /// link. The interface never sees the pick record itself.
    pub(super) fn send_picture_targets(&mut self, key: PageKey) -> Result<(), String> {
        let targets: Vec<PictureTarget> = self
            .regis
            .targets(key)
            .map(|(sub, found)| {
                found
                    .into_iter()
                    .map(|(number, x, y, width, height)| PictureTarget { number, x, y, width, height, link: self.is_picture_link(key, sub, number) })
                    .collect()
            })
            .unwrap_or_default();
        let regis = &self.regis;
        self.picture_targets.retain(|held, _| regis.has(*held));
        if self.picture_targets.get(&key).map_or(targets.is_empty(), |sent| *sent == targets) {
            return Ok(());
        }
        self.picture_targets.insert(key, targets.clone());
        self.send(&ToInterface::PictureTargets { key, targets })
    }

    /// Whether drawing `number` of `key`'s sub-page `sub` is a link.
    #[cfg_attr(not(feature = "links"), expect(clippy::unused_self, reason = "without links no drawing is one"))]
    fn is_picture_link(&self, key: PageKey, sub: u16, number: u16) -> bool {
        #[cfg(feature = "links")]
        return self.links.is_drawing_link((key, sub, number));
        #[cfg(not(feature = "links"))]
        {
            let _ = (key, sub, number);
            false
        }
    }

    /// The target the interface was last told of `key`'s picture with that
    /// number, if there is one: the interface may name only those.
    fn told_target(&self, key: PageKey, number: u16) -> Option<PictureTarget> {
        self.picture_targets.get(&key)?.iter().find(|target| target.number == number).copied()
    }

    /// The keyboard's focus moved onto a picture target, or off every one
    /// (PAR-30): a link's destination shows in the strip at once, as under
    /// the pointer. A number the interface was not told is no target.
    pub(super) fn picture_focus(&mut self, key: PageKey, number: u16) -> Result<(), String> {
        self.picture_focus = self.told_target(key, number).map(|_| (key, number));
        self.regis_repoint()
    }

    /// Enter on the focused picture link (PAR-30): followed by the rules of
    /// a click on it (REQ-6376 item 4), so only while its destination is
    /// the one the strip shows and has led there for the dwell time. Only
    /// the target the focus is on, which the ring shows (the
    /// specification's threat 9).
    #[cfg_attr(not(feature = "links"), expect(clippy::unnecessary_wraps, reason = "only following a link can fail, and this build has none"))]
    pub(super) fn picture_follow(&mut self, key: PageKey, number: u16) -> Result<(), String> {
        let focused = self.picture_focus == Some((key, number)) && self.told_target(key, number).is_some_and(|target| target.link);
        #[cfg(feature = "links")]
        if focused && let Some((sub, _)) = self.regis.held(key) {
            let (active, now) = (self.active_channel(), self.now());
            let followed = self.links.click_drawing((key, sub, number), active, now);
            return self.follow(followed, "Enter on a picture link");
        }
        let _ = focused;
        Ok(())
    }

    /// The drawing the keyboard's focus is on, as the pointer's would be:
    /// its page, the sub-code of the picture held and its number.
    #[cfg(feature = "links")]
    fn focused_drawing(&self) -> Option<(PageKey, u16, u16)> {
        let (key, number) = self.picture_focus?;
        let (sub, _) = self.regis.held(key)?;
        Some((key, sub, number))
    }

    /// The page keys go to, if any, with the sub-page of it on screen. A
    /// page whose scripts hold the viewer's input is one too (TODO-7425):
    /// its drawings' hover and pressed looks are the receiver's to draw
    /// (REQ-6374), and it has no sub-page on screen as a channel that
    /// takes input has, so no channel is told of the pointer for it.
    #[cfg(feature = "interaction")]
    fn page_taking_keys(&self) -> Option<(PageKey, Option<neoteletext_input::event::Stamp>)> {
        let key = self.typing.opened_page()?;
        if self.typing.takes_keys(key) {
            return Some((key, self.typing.on_screen(key.channel)));
        }
        self.script_holds_input(key).then_some((key, None))
    }

    /// Without typing compiled in, keys go to no channel.
    #[cfg(not(feature = "interaction"))]
    #[allow(clippy::unused_self, reason = "the same shape as with typing compiled in")]
    fn page_taking_keys(&self) -> Option<(PageKey, Option<neoteletext_input::event::Stamp>)> {
        None
    }

    /// Works out again what lies under the pointer and says what changed:
    /// after the pointer moved, a new picture arrived, a sub-page's drawing
    /// links arrived, or keys started or stopped going to a channel.
    pub(super) fn regis_repoint(&mut self) -> Result<(), String> {
        let keys = self.page_taking_keys();
        #[cfg(feature = "links")]
        let under = self.regis_pointing.under(&self.regis);
        // The strip shows where a linked drawing under the pointer leads
        // (REQ-6376 item 3), else where the one the keyboard's focus is on
        // leads (PAR-30).
        #[cfg(feature = "links")]
        if let Some(pointing) = self.links.point_drawing(under.or_else(|| self.focused_drawing())) {
            self.link(protocol::links::Message::Pointing(pointing))?;
        }
        #[cfg(feature = "links")]
        let linked = under.is_some_and(|under| self.links.is_drawing_link(under));
        #[cfg(not(feature = "links"))]
        let linked = false;
        // What was said is recorded before it is delivered: delivering an
        // enter or a leave comes back here, and then finds nothing changed.
        for say in self.regis_pointing.decide(&self.regis, keys, linked) {
            match say {
                Say::Over { key, over } => self.send(&ToInterface::RegisOver { key, over })?,
                Say::Viewer { key, sub, viewer } => self.tell(key.channel, ToDecoder::RegisViewer { page: key.page, sub, viewer }),
                #[cfg(feature = "interaction")]
                Say::Hover { key, on, leave, drawing, commits } => {
                    let sends = self.typing.drawing_hover(key, on, leave, drawing, commits, self.now_us());
                    self.deliver_typing(sends)?;
                }
                #[cfg(not(feature = "interaction"))]
                Say::Hover { .. } => {}
            }
        }
        self.regis_click()
    }

    /// A click on a drawing that is a link follows it, by the rules of a
    /// click on a text link (REQ-6376 item 4).
    #[cfg_attr(not(feature = "links"), expect(clippy::unnecessary_wraps, reason = "only following a link can fail, and this build has none"))]
    fn regis_click(&mut self) -> Result<(), String> {
        let Some((key, drawing)) = self.regis_pointing.take_click() else { return Ok(()) };
        #[cfg(feature = "links")]
        if let Some((sub, _)) = self.regis.held(key) {
            let (active, now) = (self.active_channel(), self.now());
            let followed = self.links.click_drawing((key, sub, drawing), active, now);
            return self.follow(followed, "a click on a drawing");
        }
        let _ = (key, drawing);
        Ok(())
    }
}

#[cfg(all(test, unix, feature = "links"))]
#[allow(clippy::unwrap_used, clippy::indexing_slicing, reason = "a test reports a failure by panicking, and indexes data it built")]
mod tests {
    use protocol::ToInterface;
    use protocol::decoder::FromDecoder;
    use protocol::regis::{PictureLanguage, REGIS_COLOURS, REGIS_PIXELS, REGIS_WIDTH, RegisArea, RegisPick, RegisPicture};

    use super::super::test_support::{core_watched, sent_to_interface};
    use super::*;

    /// Renderer parity PAR-30: the interface is told a picture's targets,
    /// in reading order, with the links marked; the keyboard's focus on a
    /// link shows where it leads; Enter follows only the focused link, and
    /// only after the dwell time; a number the interface was not told is
    /// no target.
    #[test]
    fn the_keyboard_reaches_a_pictures_targets_and_follows_a_focused_link() {
        let (mut core, _broker, written) = core_watched("a.invalid");
        let channel = core.session.active_tab().unwrap().channel.id;
        let key = PageKey { channel, page: 0x100 };
        core.intent(protocol::ToCore::Opened(Some(key))).unwrap();
        let mut record = vec![0_u16; REGIS_PIXELS];
        record[20 * REGIS_WIDTH + 600] = 5;
        record[100 * REGIS_WIDTH + 30] = 3;
        let pick = RegisPick::of(&record, [(5, 0, 0), (3, 0, 0)].into_iter(), PictureLanguage::Regis);
        let language = PictureLanguage::Regis;
        let picture = RegisPicture {
            commits: 1,
            work: 0,
            language,
            palette: [[0; 3]; REGIS_COLOURS],
            number: 1,
            base: 0,
            area: RegisArea::whole(language),
            packed: vec![0; REGIS_PIXELS / 2],
        };
        core.decoded(channel, FromDecoder::Regis { page: key.page, sub: 0, picture, pick: Some(pick) }).unwrap();
        core.decoded(channel, FromDecoder::RegisLinks { page: key.page, sub: 0, links: vec![(3, "200".into())] }).unwrap();
        let sent = sent_to_interface(&mut core, &written);
        let targets: Vec<_> =
            sent.iter().filter_map(|message| if let ToInterface::PictureTargets { targets, .. } = message { Some(targets.clone()) } else { None }).collect();
        let link = PictureTarget { number: 3, x: 30, y: 100, width: 1, height: 1, link: true };
        let plain = PictureTarget { number: 5, x: 600, y: 20, width: 1, height: 1, link: false };
        assert_eq!(targets.last().unwrap(), &[plain, link], "reading order: drawing 5 is higher");

        core.picture_focus(key, 9).unwrap();
        assert_eq!(core.picture_focus, None, "a number the interface was not told");
        core.picture_focus(key, 3).unwrap();
        let pointing =
            sent_to_interface(&mut core, &written).into_iter().any(|message| matches!(message, ToInterface::Link(protocol::links::Message::Pointing(Some(_)))));
        assert!(pointing, "the focused link's destination shows in the strip at once");
        core.picture_follow(key, 3).unwrap();
        assert!(!sent_to_interface(&mut core, &written).iter().any(|message| matches!(message, ToInterface::Show(_))), "not within the dwell time");
        // Later than the dwell time, as if the link had led there that long.
        core.started = core.started.checked_sub(std::time::Duration::from_secs(1)).unwrap();
        core.picture_follow(key, 5).unwrap();
        assert!(!sent_to_interface(&mut core, &written).iter().any(|message| matches!(message, ToInterface::Show(_))), "not the focused target, nor a link");
        core.picture_follow(key, 3).unwrap();
        let shown = sent_to_interface(&mut core, &written).into_iter().any(|message| message == ToInterface::Show(PageKey { channel, page: 0x200 }));
        assert!(shown, "Enter follows the focused link");
    }
}
