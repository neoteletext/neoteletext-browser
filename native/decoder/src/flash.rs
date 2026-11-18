//! Flash (EN 300 706 clause 12.3, the engine's decisions E.12 and E.48):
//! which flash rates a page's cells use, and the shown pages that flash, so the
//! decoder wakes and draws again only when a rate one of them uses changes
//! phase.

use std::collections::BTreeMap;

use neoteletext_core::config::Config;
use neoteletext_core::consts::{PRESENTATION_TICKS_PER_SECOND, presentation_tick_at_us, presentation_tick_due_us};
use neoteletext_core::level1::FlashRate;
use neoteletext_core::render::FlashPhase;

use crate::Rendered;

/// The flash rates of the engine's decisions E.12 and E.48, in the order
/// of [`FlashRates`]' bits.
const RATES: [FlashRate; 5] = [FlashRate::Level1, FlashRate::Slow, FlashRate::FastPhase1, FlashRate::FastPhase2, FlashRate::FastPhase3];

/// The set of flash rates a page's cells use: one bit for each of
/// [`RATES`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FlashRates(u8);

impl FlashRates {
    /// The rates of `cells`.
    pub fn of(cells: impl Iterator<Item = FlashRate>) -> Self {
        Self(cells.fold(0, |used, rate| used | RATES.iter().position(|&each| each == rate).map_or(0, |index| 1_u8 << index)))
    }

    pub(crate) fn is_empty(self) -> bool {
        self.0 == 0
    }

    fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    /// Whether any of these rates is in a different phase in `drawn` and
    /// `now`: whether a page drawn in `drawn` looks different at `now`.
    fn differ(self, drawn: FlashPhase, now: FlashPhase) -> bool {
        RATES.iter().enumerate().any(|(index, &rate)| self.0 & (1_u8 << index) != 0 && drawn.is_on(rate) != now.is_on(rate))
    }
}

/// The shown pages with flashing cells, each with the flash phase it was
/// last drawn in and the rates it uses: the decoder wakes for flash only
/// when one of them is shown, and only when a rate one of them uses
/// changes phase, and draws a page again only when a rate it uses did
/// (requirement 9.7). A page with the Level 1 flash alone is drawn at its
/// two changes in every 1.28 seconds, not seven times a second.
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
        self.pages.iter().filter(|&(_, &(drawn, rates))| rates.differ(drawn, phase)).map(|(&page, _)| page).collect()
    }

    /// When a rate some shown page uses next changes phase, if any shown
    /// page flashes: the time the first later tick of the presentation
    /// clock is due at which the Level 1 flash (the engine's decision E.12)
    /// or one of the twelve units of the one-second cycle (its decision
    /// E.48) shows a rate in use differently (ANIM-1). The search is
    /// bounded by one cycle of the Level 1 flash or one second of the
    /// units, whichever is longer, within which every rate changes.
    #[must_use]
    pub fn next_us(&self, now_us: u64, config: &Config) -> Option<u64> {
        let used = self.pages.values().fold(FlashRates::default(), |used, &(_, rates)| used.union(rates));
        if used.is_empty() {
            return None;
        }
        let tick = presentation_tick_at_us(now_us);
        let now = FlashPhase::at_tick(tick, config);
        let cycle = config.flash_shown_ticks.saturating_add(config.flash_hidden_ticks).max(PRESENTATION_TICKS_PER_SECOND);
        (1..=cycle).map_while(|ahead| tick.checked_add(ahead)).find(|&later| used.differ(now, FlashPhase::at_tick(later, config))).map(presentation_tick_due_us)
    }
}
