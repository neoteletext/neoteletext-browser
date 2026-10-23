//! What changed in the engine since the decoder last asked: the pages
//! whose reception moved, or everything, when data every page may draw
//! from changed.

use std::collections::BTreeSet;

use neoteletext_core::acq::Event;

/// What the engine's events said changed.
#[derive(Default)]
pub(crate) struct Changes {
    pub(crate) touched: BTreeSet<u16>,
    pub(crate) completed: BTreeSet<u16>,
    /// Pages and sub-pages whose reception started.
    pub(crate) started: BTreeSet<(u16, u16)>,
    pub(crate) everything: bool,
}

impl Changes {
    pub(crate) fn note(&mut self, event: &Event) {
        match event {
            Event::PageStarted(key) => {
                self.touched.insert(key.address.mpp());
                self.started.insert((key.address.mpp(), key.subcode.value()));
            }
            Event::PageErased(key) | Event::UpdateIndicator(key) | Event::PageUpdated { key, .. } => {
                self.touched.insert(key.address.mpp());
            }
            Event::PageCompleted(key) => {
                self.touched.insert(key.address.mpp());
                self.completed.insert(key.address.mpp());
            }
            // Data every page of a magazine, or any page, may draw from.
            Event::MagazineData(_) | Event::EnhancementPageChanged(_) => self.everything = true,
            _ => {}
        }
    }
}

/// What changed since [`Engine::take_changed`](crate::Engine::take_changed) was last called.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Changed {
    /// The display pages whose reception moved.
    pub pages: BTreeSet<u16>,
    /// Every page may draw differently.
    pub everything: bool,
}
