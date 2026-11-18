//! Flash (EN 300 706 clause 12.3, the engine's decisions E.12 and E.48):
//! the shown pages that flash, each with the flash rates its cells use, so
//! the decoder wakes and draws again only when a rate one of them uses
//! changes phase. The phases, the rates and the tick each next changes at
//! are the engine's (animation.md ANIM-1): flash is a function of the
//! presentation clock's tick alone.

use std::collections::BTreeMap;

use neoteletext_core::config::Config;
use neoteletext_core::consts::{presentation_tick_at_us, presentation_tick_due_us};
use neoteletext_core::render::{FlashPhase, next_flash_tick};

/// The set of flash rates a page's cells use, as the engine finds it.
pub use neoteletext_core::render::FlashRates;

use crate::Rendered;

/// The shown pages with flashing cells, each with the flash phase it was
/// last drawn in and the rates it uses: the decoder wakes for flash only
/// when one of them is shown, and only when a rate one of them uses
/// changes phase, and draws a page again only when a rate it uses did
/// (requirement 9.7). A page with the Level 1 flash alone is drawn at its
/// two changes in every cycle of 76 ticks, 1.267 seconds (ANIM-1), not
/// at every unit of the faster rates.
#[derive(Debug, Default)]
pub struct Flashing {
    pages: BTreeMap<u16, (FlashPhase, FlashRates)>,
}

impl Flashing {
    /// Records how `page` was last drawn.
    pub fn drawn(&mut self, page: u16, rendered: &Rendered) {
        match rendered.flash {
            Some(flash) => self.pages.insert(page, flash),
            None => self.pages.remove(&page),
        };
    }

    /// Keeps only the pages still shown.
    pub fn retain(&mut self, shown: &[u16]) {
        self.pages.retain(|page, _| shown.contains(page));
    }

    /// The flashing pages that look different at `now_us` from how they
    /// were drawn: flash is a function of the presentation clock's tick
    /// alone (animation.md ANIM-1), the tick due at `now_us`.
    #[must_use]
    pub fn due(&self, now_us: u64, config: &Config) -> Vec<u16> {
        let phase = FlashPhase::at_tick(presentation_tick_at_us(now_us), config);
        self.pages.iter().filter(|&(_, &(drawn, rates))| phase.differs_for(drawn, rates)).map(|(&page, _)| page).collect()
    }

    /// When a rate some shown page uses next changes phase, if any shown
    /// page flashes: the time the first later tick of the presentation
    /// clock is due at which the Level 1 flash (the engine's decision E.12)
    /// or one of the twelve units of the one-second cycle (its decision
    /// E.48) shows a rate in use differently, as the engine's
    /// `next_flash_tick` finds it for each page (ANIM-1). A page with the
    /// Level 1 flash alone wakes the decoder twice in its cycle of 76
    /// ticks, not sixty times a second.
    #[must_use]
    pub fn next_us(&self, now_us: u64, config: &Config) -> Option<u64> {
        let tick = presentation_tick_at_us(now_us);
        self.pages.values().filter_map(|&(_, rates)| next_flash_tick(tick, config, rates)).min().map(presentation_tick_due_us)
    }
}
