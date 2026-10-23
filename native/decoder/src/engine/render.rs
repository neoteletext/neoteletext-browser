//! Drawing a page: its view (reveal, hold, mix, the sub-page), its grid
//! composed at the profile's presentation level, and its palette indices
//! rendered by the engine's row renderer; the pages exported; and a page's
//! Fastext.

use super::{ALL_ROWS, Engine, Exported, GLYPHS, PAGE_DOTS, PICTURE, Rendered};
#[cfg(feature = "links")]
use super::{FASTEXT_ROW, Found};
use crate::FlashRates;

#[cfg(feature = "links")]
use crate::links;
use crate::title::title;
use interface_core::canvas::PAGE_WIDTH;
use interface_core::channel::page_index;
use neoteletext_core::acq::StoredPage;
use neoteletext_core::charset::{CharRef, CharSet, G2Set};
use neoteletext_core::compose::compose_level;
use neoteletext_core::consts::{G2_RIGHT_GUILLEMET, ROWS};
use neoteletext_core::fsm::PageState;
use neoteletext_core::level1::{DisplayOptions, Glyph, Grid, build_grid};
#[cfg(feature = "links")]
use neoteletext_core::nav::PageLinks;
use neoteletext_core::render::{FlashPhase, IndexBuffer, IndexFormat, RenderOptions, render_screen};
use neoteletext_core::types::{Instant, PageAddress, SubCode};
use neoteletext_image::palette::colour_map_palette;
use protocol::decoder::View;

impl Engine {
    /// How `page` is drawn from now on: a page held keeps the version it
    /// shows now (`page-controls.md` section 3, decision 67).
    pub fn set_view(&mut self, view: View) {
        let page = view.page;
        if !view.hold {
            self.held.remove(&page);
        } else if !self.held.contains_key(&page) || self.views.get(&page).map(|before| before.subpage) != Some(view.subpage) {
            let showing = self.stored(page, view.subpage).copied();
            if let Some(stored) = showing {
                self.held.insert(page, Box::new(stored));
            }
        }
        if view == (View { page, ..View::default() }) {
            self.views.remove(&page);
        } else {
            self.views.insert(page, view);
        }
    }

    /// The stored sub-page of `page` to draw: the one chosen, while the
    /// store has it, else the latest.
    fn stored(&self, page: u16, subpage: Option<u16>) -> Option<&StoredPage> {
        let address = PageAddress::from_mpp(page)?;
        let store = self.decoder.store();
        subpage.and_then(|subpage| store.get(address, SubCode::new(subpage))).or_else(|| store.get(address, SubCode::ANY))
    }

    /// The sub-codes stored for `page`, in ascending order, at most
    /// [`interface_core::controls::MAX_SUBPAGES`].
    fn subcodes(&self, page: u16) -> Vec<u16> {
        let Some(address) = PageAddress::from_mpp(page) else { return Vec::new() };
        let mut codes = [SubCode::ANY; neoteletext_core::viewer::MAX_LISTED_SUBPAGES];
        let count = self.decoder.store().subcodes(address, &mut codes);
        codes.iter().take(count).map(|code| code.value()).collect()
    }

    /// Every display page received completely, each with its 25 rows of
    /// stored 7-bit codes and its title: for tools that rebuild channels
    /// from recordings.
    #[must_use]
    pub fn export(&self) -> Vec<Exported> {
        let mut pages: Vec<Exported> = self
            .decoder
            .store()
            .pages()
            .iter()
            .filter(|page| page.state == PageState::Complete && page_index(page.key.address.mpp()).is_some())
            .map(|page| {
                let number = page.key.address.mpp();
                let rows = page.cells.iter().take(ROWS).copied().collect();
                Exported { page: number, rows, title: self.grid(number).map(|(grid, _)| title(&grid)).unwrap_or_default() }
            })
            .collect();
        pages.sort_by_key(|page| page.page);
        pages.dedup_by_key(|page| page.page);
        pages
    }

    /// The page's grid, as the engine composes it for display in its view
    /// (`page-controls.md`), and the stored sub-page it shows.
    pub(super) fn grid(&self, page: u16) -> Option<(Grid, &StoredPage)> {
        let address = PageAddress::from_mpp(page)?;
        let view = self.views.get(&page).copied().unwrap_or_default();
        let stored = match self.held.get(&page) {
            Some(held) => held,
            None => self.stored(page, view.subpage)?,
        };
        let config = self.decoder.config();
        let magazine = self.decoder.magazine_data(address.magazine);
        let receiver = self.receiver_rows.get(&(page, stored.key.subcode.value()));
        let options = DisplayOptions { mix: view.mix, row24: receiver.map(|row| row.codes), ..DisplayOptions::default() };
        let mut grid = Grid::EMPTY;
        build_grid(&mut grid, Some(stored), &magazine, config, &options);
        let lookup = self.decoder.enhancement_lookup();
        let _ = compose_level(&mut grid, stored, &magazine, &lookup, config, &options);
        // The "»" marks, as the engine's viewer places them: Latin G2's
        // right guillemet where the codes hold `>`.
        for &column in receiver.map_or(&[][..], |row| row.marks.as_slice()) {
            if let Some(cell) = grid.cell_mut(ROWS.saturating_sub(1), column) {
                cell.glyph = Glyph::Char(CharRef::new(CharSet::G2(G2Set::Latin), G2_RIGHT_GUILLEMET));
            }
        }
        Some((grid, stored))
    }

    /// Renders the latest sub-page of `page` at the engine's flash phase
    /// for now, in the resolved colour mode, whose palette is the page's
    /// Colour Map of at most 32 colours. The page's own underline is not
    /// drawn: in NeoTeletext an underline means a link, and the browser
    /// draws links itself (decision 42, the engine's decision E.88).
    #[must_use]
    pub fn render(&self, page: u16) -> Option<Rendered> {
        self.render_at(page, self.now_us())
    }

    /// Renders `page` at the flash phase of `now_us`, on
    /// [`Engine::now_us`]'s clock.
    #[must_use]
    pub fn render_at(&self, page: u16, now_us: u64) -> Option<Rendered> {
        self.render_into(page, now_us, Vec::new())
    }

    /// [`Engine::render_at`] into `indices`, a page's worth of room kept
    /// from an earlier frame: a flashing page is rendered seven times a
    /// second, and need not take 240 KB afresh each time.
    #[must_use]
    pub fn render_into(&self, page: u16, now_us: u64, indices: Vec<u8>) -> Option<Rendered> {
        self.render_over(page, now_us, None, indices)
    }

    /// [`Engine::render_into`], drawn over `last`, the page's last frame,
    /// when there is one: only the rows that changed since it are drawn
    /// again (the engine's E.107). The page's change tracker follows every
    /// frame drawn, so `last` must be the last frame drawn of it, or one
    /// the same.
    #[must_use]
    pub fn render_over(&self, page: u16, now_us: u64, last: Option<&[u8]>, mut indices: Vec<u8>) -> Option<Rendered> {
        page_index(page)?;
        let (grid, stored) = self.grid(page)?;
        let subcode = stored.key.subcode.value();
        let reveal = self.views.get(&page).is_some_and(|view| view.reveal);
        let config = self.decoder.config();
        let phase = FlashPhase::at(Instant(now_us));
        let render = RenderOptions::resolved(config, reveal, phase).with_underline(false);
        let changed = self.trackers.borrow_mut().entry(page).or_default().changed_rows(&grid, render);
        indices.clear();
        let rows = if let Some(last) = last.filter(|last| last.len() == PAGE_DOTS) {
            indices.extend_from_slice(last);
            changed
        } else {
            indices.resize(PAGE_DOTS, 0);
            ALL_ROWS
        };
        let mut buffer = IndexBuffer { format: IndexFormat::Bits8, data: &mut indices, stride: usize::from(PAGE_WIDTH) };
        render_screen(&grid, rows, &GLYPHS, render, &mut buffer).ok()?;
        let palette: Vec<[u8; 3]> = colour_map_palette(&grid.colour_map, PICTURE).iter().map(|colour| [colour.r, colour.g, colour.b]).collect();
        #[cfg(feature = "links")]
        let links = {
            let (candidates, numbers) = links::find(&grid, &palette, self.lines.get(page, subcode));
            Found { candidates, numbers, fastext: fastext(stored, &grid) }
        };
        let rates = FlashRates::of(grid.cells.iter().flatten().filter(|cell| cell.flash).map(|cell| cell.attributes.flash_rate()));
        Some(Rendered {
            palette,
            screen: grid.screen_colour,
            indices,
            flash: (!rates.is_empty()).then_some((phase, rates)),
            #[cfg(feature = "links")]
            links,
            subpages: (self.subcodes(page), subcode),
        })
    }
}

/// A page's Fastext from its packet X/27/0, which the engine writes from a
/// TTI page's link lines too (the engine's REQ-NAV-003), and whether its
/// row 24 is shown as the engine's viewer decides it: drawn, and not boxed
/// away (REQ-SEC-024).
#[cfg(feature = "links")]
fn fastext(stored: &StoredPage, grid: &Grid) -> protocol::links::Fastext {
    let links = PageLinks::from_page(stored);
    let target = |link: Option<neoteletext_core::nav::LinkTarget>| {
        let link = link?;
        let page = link.address.mpp();
        page_index(page)?;
        let subcode = (!link.subcode.is_any()).then(|| link.subcode.value());
        Some(protocol::links::FastextTarget { page, subcode, service: link.service })
    };
    let [red, green, yellow, cyan] = links.keys;
    let row24 = !grid.boxed_mode && grid.cells.get(FASTEXT_ROW).is_some_and(|row| row.iter().any(|cell| !cell.glyph.is_blank()));
    protocol::links::Fastext { keys: [target(red), target(green), target(yellow), target(cyan), target(links.index)], row24 }
}
