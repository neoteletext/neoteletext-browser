//! Flash (EN 300 706 clause 12.3, the engine's decision E.48): which
//! flash rates a page's cells use, and the shown pages that flash, so the
//! decoder wakes and draws again only when a rate one of them uses changes
//! phase.

use std::collections::BTreeMap;

use neoteletext_core::consts::{FLASH_CYCLE_US, FLASH_UNITS};
use neoteletext_core::level1::FlashRate;
use neoteletext_core::render::FlashPhase;
use neoteletext_core::types::Instant;

use crate::Rendered;

/// The flash rates of the engine's decision E.48, in the order of
/// [`FlashRates`]' bits.
const RATES: [FlashRate; 4] = [FlashRate::Slow, FlashRate::FastPhase1, FlashRate::FastPhase2, FlashRate::FastPhase3];

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
/// (requirement 9.7). A page with slow flash alone is drawn twice a
/// second, not seven times.
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
    /// were drawn.
    #[must_use]
    pub fn due(&self, now_us: u64) -> Vec<u16> {
        let phase = FlashPhase::at(Instant(now_us));
        self.pages.iter().filter(|&(_, &(drawn, rates))| rates.differ(drawn, phase)).map(|(&page, _)| page).collect()
    }

    /// When a rate some shown page uses next changes phase, if any shown
    /// page flashes: the start of that one of the twelve units of the
    /// one-second cycle (the engine's decision E.48).
    #[must_use]
    pub fn next_us(&self, now_us: u64) -> Option<u64> {
        let used = self.pages.values().fold(FlashRates::default(), |used, &(_, rates)| used.union(rates));
        if used.is_empty() {
            return None;
        }
        let units = u64::from(FLASH_UNITS);
        let cycle = now_us.checked_div(FLASH_CYCLE_US).unwrap_or(0).saturating_mul(FLASH_CYCLE_US);
        let within = now_us.saturating_sub(cycle);
        let unit = within.saturating_mul(units).checked_div(FLASH_CYCLE_US).unwrap_or(0);
        let phase = FlashPhase::at_unit(u8::try_from(unit).unwrap_or(0));
        // The first later unit whose phase differs for a rate in use; every
        // rate changes within a cycle, so one is found.
        let ahead = (1..=units).find(|&ahead| used.differ(phase, FlashPhase::at_unit(u8::try_from(unit.saturating_add(ahead)).unwrap_or(0)))).unwrap_or(1);
        let next = unit.saturating_add(ahead).saturating_mul(FLASH_CYCLE_US).div_ceil(units);
        Some(cycle.saturating_add(next))
    }
}
