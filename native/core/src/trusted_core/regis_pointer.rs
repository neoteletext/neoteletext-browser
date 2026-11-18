//! The pointer on a page drawn in ReGIS: the trusted core carries out
//! what `neoteletext::regis_pointer` decides to say (REQ-6374, REQ-6375,
//! REQ-6377).

use interface_core::canvas::PageKey;
use neoteletext::regis_pointer::Say;
use protocol::ToInterface;
use protocol::decoder::ToDecoder;
use protocol::regis::RegisPointer;

use super::Core;

impl Core {
    /// The interface said where the pointer is on a ReGIS or NAPLPS
    /// picture. A pixel the picture held for the page has not is off it.
    pub(super) fn regis_pointer(&mut self, mut pointer: RegisPointer) -> Result<(), String> {
        pointer.at = pointer.at.filter(|at| self.regis.has_pixel(pointer.key, *at));
        self.regis_pointing.pointer(pointer, &self.regis);
        self.regis_repoint()
    }

    /// The page keys go to, if any, with the sub-page of it on screen.
    #[cfg(feature = "interaction")]
    fn page_taking_keys(&self) -> Option<(PageKey, Option<neoteletext_input::event::Stamp>)> {
        let key = self.typing.opened_page().filter(|key| self.typing.takes_keys(*key))?;
        Some((key, self.typing.on_screen(key.channel)))
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
        // (REQ-6376 item 3).
        #[cfg(feature = "links")]
        if let Some(pointing) = self.links.point_drawing(under) {
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
