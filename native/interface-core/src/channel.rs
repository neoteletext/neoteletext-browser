//! A channel as the interface shows it: its banks and the status of each of
//! its 800 pages. Teletext pages run from 100 to 899 in eight banks, one
//! per magazine; a page number is three digits, the magazine and two
//! decimal digits (EN 300 706 clause 7.1.2; numbers with the digits A to F
//! are not pages for display).

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use core::ops::Range;

/// Banks of pages: one per magazine, 1 to 8.
pub const BANKS: usize = 8;
/// Display pages in a bank: two decimal digits, 00 to 99.
pub const PAGES_PER_BANK: usize = 100;
/// Display pages in a channel, 100 to 899.
pub const PAGES: usize = BANKS * PAGES_PER_BANK;
const MAGAZINE_SHIFT: u16 = 8;
const TENS_SHIFT: u16 = 4;
const DIGIT_MASK: u16 = 0xF;
const DECIMAL: u16 = 10;

/// What is known about a page, as the trusted core reports it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    /// The channel's contents list it, and it has not been received yet.
    Available = 1,
    /// Received and decoded.
    Loaded = 2,
}

impl Status {
    /// The status a protocol byte names, or `None` for any other byte.
    #[must_use]
    pub fn from_byte(byte: u8) -> Option<Self> {
        match byte {
            1 => Some(Self::Available),
            2 => Some(Self::Loaded),
            _ => None,
        }
    }
}

/// The page number of index 0 to 799: bank 0 holds 100 to 199.
#[must_use]
pub fn page_number(index: usize) -> Option<u16> {
    if index >= PAGES {
        return None;
    }
    let index = u16::try_from(index).ok()?;
    let bank = index / 100;
    let tens = index % 100 / DECIMAL;
    let ones = index % DECIMAL;
    Some(bank.saturating_add(1) << MAGAZINE_SHIFT | tens << TENS_SHIFT | ones)
}

/// The index 0 to 799 of a display page number, or `None` for a number
/// outside 100 to 899 or with a digit above 9.
#[must_use]
pub fn page_index(page: u16) -> Option<usize> {
    let magazine = page >> MAGAZINE_SHIFT;
    let tens = page >> TENS_SHIFT & DIGIT_MASK;
    let ones = page & DIGIT_MASK;
    if !(1..=8).contains(&magazine) || tens >= DECIMAL || ones >= DECIMAL {
        return None;
    }
    let within = usize::from(tens).saturating_mul(usize::from(DECIMAL)).saturating_add(usize::from(ones));
    Some(bank_indices(usize::from(magazine.saturating_sub(1))).start.saturating_add(within))
}

/// The indices of bank `bank`'s pages (0 to 7, magazines 1 to 8).
#[must_use]
pub fn bank_indices(bank: usize) -> Range<usize> {
    let first = bank.saturating_mul(PAGES_PER_BANK);
    first..first.saturating_add(PAGES_PER_BANK)
}

/// One page of a channel.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PageInfo {
    /// The page's title, from the channel's contents or the page itself.
    pub title: String,
    /// Whether it has arrived.
    pub status: Status,
}

/// A channel's banks and pages, as the trusted core reports them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Channel {
    /// The channel, as the trusted core numbers it.
    pub channel: u32,
    /// The channel's title.
    pub title: String,
    /// Each bank's name, [`BANKS`] of them.
    pub banks: Vec<String>,
    /// One entry per page index; `None` is a page the channel does not have.
    pub pages: Vec<Option<PageInfo>>,
}

impl Channel {
    /// A channel with no title and no pages.
    #[must_use]
    pub fn empty() -> Self {
        Self { channel: 0, title: String::new(), banks: vec![String::new(); BANKS], pages: vec![None; PAGES] }
    }

    /// How many pages the channel has, and how many of them are loaded.
    #[must_use]
    pub fn counts(&self) -> (usize, usize) {
        let present = self.pages.iter().flatten().count();
        let loaded = self.pages.iter().flatten().filter(|page| page.status == Status::Loaded).count();
        (present, loaded)
    }

    /// How many pages bank `bank` has.
    #[must_use]
    pub fn bank_count(&self, bank: usize) -> usize {
        self.pages.get(bank_indices(bank)).map_or(0, |pages| pages.iter().flatten().count())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn page_numbers_and_indices_agree() {
        assert_eq!(page_number(0), Some(0x100));
        assert_eq!(page_number(99), Some(0x199));
        assert_eq!(page_number(799), Some(0x899));
        assert_eq!(page_number(800), None);
        for index in 0..PAGES {
            assert_eq!(page_index(page_number(index).unwrap()), Some(index));
        }
        assert_eq!(page_index(0x1A0), None, "a hexadecimal page is not a display page");
        assert_eq!(page_index(0x900), None);
        assert_eq!(page_index(0x099), None);
    }
}
