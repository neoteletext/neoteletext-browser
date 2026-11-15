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
    /// The interface said where the pointer is on a ReGIS picture.
    pub(super) fn regis_pointer(&mut self, pointer: RegisPointer) -> Result<(), String> {
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
    /// after the pointer moved, a new picture arrived, or keys started or
    /// stopped going to a channel.
    pub(super) fn regis_repoint(&mut self) -> Result<(), String> {
        let keys = self.page_taking_keys();
        // What was said is recorded before it is delivered: delivering an
        // enter or a leave comes back here, and then finds nothing changed.
        for say in self.regis_pointing.decide(&self.regis, keys) {
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
        Ok(())
    }
}
