//! Where a script's teletext sprites keep their frames (the script API's
//! specification, section 5.3), in the one place the script host and the
//! decoder both read it.
//!
//! A sprite's frame is a Passive object (EN 300 706 clause 13.5) on an
//! Object page the decoder writes from the batch's `TELETEXT_OBJECTS`
//! section and links from the open page as its POP by packet X/27/4. The
//! script host chooses each object's number, and an object's number on a
//! POP says where it lies (Table 28): its sub-page in N3 to N0 and its
//! place among the sub-page's Passive objects in N8 to N4. So the host
//! lays the objects out, within what an Object page really holds: sixteen
//! sub-pages, each of at most [`OBJECTS_PER_SUBPAGE`] Passive objects and
//! [`SUBPAGE_TRIPLETS`] triplets of definitions, less the pointer packets
//! past the second.

use neoteletext_core::consts::{
    OBJECT_BODY_TRIPLETS, OBJECT_DEFINITION_FIRST_PACKET, OBJECT_NUMBER_N4_SHIFT, OBJECT_NUMBER_NIBBLE, OBJECT_POINTER_FIRST_PACKET,
    OBJECTS_PER_POINTER_PACKET, OBJECTS_PER_TYPE, TRIPLETS_PER_PACKET,
};
use neoteletext_core::limits::{ENHANCEMENT_SUBPAGES, PAGE_ACI};

use super::batch::{MAX_OBJECT_FRAMES, MAX_OBJECT_NUMBER, MAX_SPRITES, MAX_TELETEXT_OBJECTS};

/// The page number of the Object page the decoder writes for a script's
/// sprites, in the form `0xMPP`: 1BE, which EN 300 706 Annex A.10
/// reserves for Automatic Channel Installation. No channel may have a
/// page there and no link may lead to it (the interface's
/// `is_channel_page` refuses every reserved number), a viewer cannot key
/// its hexadecimal digits, and nothing in NeoTeletext reads a page there,
/// since a NeoTeletext channel is not a broadcast to be installed. A page
/// of any magazine links it through the relative magazine of its X/27/4
/// link (Table 15).
pub const SCRIPT_OBJECT_PAGE: u16 = PAGE_ACI;

/// The sub-pages an Object page may have: S1 alone names one, 0 to F.
pub const OBJECT_SUBPAGES: usize = ENHANCEMENT_SUBPAGES;
/// The Passive objects one sub-page of an Object page may define: those
/// its pointer packets 1 to 4 can point to (clause 10.5.1.1).
pub const OBJECTS_PER_SUBPAGE: usize = OBJECTS_PER_TYPE;
/// The triplets of one sub-page's definitions: absolute triplets 0 to 506
/// (clause 10.5.1.2), which is also the longest an object may be.
pub const SUBPAGE_TRIPLETS: usize = OBJECT_BODY_TRIPLETS as usize;
/// The triplets an object's definition takes beside its body: its Object
/// Definition triplet before and its Termination Marker after (clause
/// 13.1).
pub const OBJECT_FRAMING_TRIPLETS: usize = 2;
/// The longest body of an object: what the batch's section carries for
/// one, so that the object with its framing is at most
/// [`SUBPAGE_TRIPLETS`] long.
pub const MAX_OBJECT_BODY: usize = SUBPAGE_TRIPLETS - OBJECT_FRAMING_TRIPLETS;

/// The sprites the channel context may have of a page's
/// [`MAX_SPRITES`], which are kept for it so that the page context
/// cannot leave it none, and which it cannot pass, so that it cannot
/// starve the page (the specification's section 3.1 and
/// `CHANNEL_SHARE_SPRITES` of section 10.6).
pub const CHANNEL_SHARE_SPRITES: usize = 4;
/// The sub-pages of the Object page that hold the channel context's
/// frames, the last ones: enough for every frame of its sprites.
pub const CHANNEL_SHARE_OBJECT_SUBPAGES: usize = CHANNEL_SHARE_SPRITES * MAX_OBJECT_FRAMES / OBJECTS_PER_SUBPAGE;
/// The sub-pages that hold the page context's frames, the first ones.
pub const PAGE_SHARE_OBJECT_SUBPAGES: usize = OBJECT_SUBPAGES - CHANNEL_SHARE_OBJECT_SUBPAGES;

// Each share has an object for every frame its sprites may have, the
// channel's sub-pages are whole, and an object number holds every place.
const _: () = assert!(CHANNEL_SHARE_SPRITES * MAX_OBJECT_FRAMES == CHANNEL_SHARE_OBJECT_SUBPAGES * OBJECTS_PER_SUBPAGE);
const _: () = assert!((MAX_SPRITES - CHANNEL_SHARE_SPRITES) * MAX_OBJECT_FRAMES <= PAGE_SHARE_OBJECT_SUBPAGES * OBJECTS_PER_SUBPAGE);
const _: () = assert!(OBJECT_SUBPAGES * OBJECTS_PER_SUBPAGE == MAX_TELETEXT_OBJECTS && MAX_TELETEXT_OBJECTS == MAX_OBJECT_NUMBER as usize + 1);
const _: () = assert!(OBJECT_SUBPAGES == OBJECT_NUMBER_NIBBLE as usize + 1 && SUBPAGE_TRIPLETS.is_multiple_of(TRIPLETS_PER_PACKET));

/// The number of the Passive object at `index` (0 to 31) among those of
/// sub-page `subpage` (0 to 15) of the Object page (Table 28: N3 to N0
/// the sub-page; N8 N7 the pointer packet, N6 N5 the group of pointer
/// triplets and N4 the half, which together are the index), or none when
/// either is out of range.
#[must_use]
pub const fn object_number(subpage: usize, index: usize) -> Option<u16> {
    if subpage >= OBJECT_SUBPAGES || index >= OBJECTS_PER_SUBPAGE {
        return None;
    }
    #[expect(clippy::cast_possible_truncation, reason = "the sub-page is below 16 and the index below 32, just above")]
    let number = ((index as u16) << OBJECT_NUMBER_N4_SHIFT) | subpage as u16;
    Some(number)
}

/// The sub-page and the index among its Passive objects of the object
/// numbered `number`, or none when that is no object number.
#[must_use]
pub const fn object_place(number: u16) -> Option<(usize, usize)> {
    if number > MAX_OBJECT_NUMBER {
        return None;
    }
    Some(((number & OBJECT_NUMBER_NIBBLE) as usize, (number >> OBJECT_NUMBER_N4_SHIFT) as usize))
}

/// The triplets of a sub-page's definition area that its pointer packets
/// take when its highest Passive object is at `highest_index`: packets 1
/// and 2 lie before the area, and packets 3 and 4 are pointers instead of
/// definitions once an object of the third or fourth eight is defined
/// (clause 10.5.1.3).
#[must_use]
pub const fn pointer_triplets(highest_index: usize) -> usize {
    let Some(packets) = highest_index.checked_div(OBJECTS_PER_POINTER_PACKET) else { return 0 };
    let last_pointer_packet = packets.saturating_add(OBJECT_POINTER_FIRST_PACKET as usize);
    last_pointer_packet.saturating_add(1).saturating_sub(OBJECT_DEFINITION_FIRST_PACKET as usize).saturating_mul(TRIPLETS_PER_PACKET)
}

#[cfg(test)]
mod tests {
    use neoteletext_core::limits::is_reserved_page;

    use super::*;

    /// Section 5.3: the Object page's number is one no channel page has
    /// and no viewer reaches, by the engine's own list of reserved
    /// numbers.
    #[test]
    fn the_script_object_page_is_a_reserved_number() {
        let [magazine, page] = SCRIPT_OBJECT_PAGE.to_be_bytes();
        assert!(is_reserved_page(magazine, page));
    }

    /// Table 28: every place on the Object page has one number, which
    /// names that place and no other.
    #[test]
    fn an_object_number_names_its_place() {
        let mut seen = Vec::new();
        for subpage in 0..OBJECT_SUBPAGES {
            for index in 0..OBJECTS_PER_SUBPAGE {
                let number = object_number(subpage, index).unwrap();
                assert_eq!(object_place(number), Some((subpage, index)));
                assert!(number <= MAX_OBJECT_NUMBER && !seen.contains(&number));
                seen.push(number);
            }
        }
        assert_eq!(seen.len(), MAX_TELETEXT_OBJECTS);
        assert_eq!((object_number(OBJECT_SUBPAGES, 0), object_number(0, OBJECTS_PER_SUBPAGE), object_place(MAX_OBJECT_NUMBER + 1)), (None, None, None));
    }

    /// Clause 10.5.1.3: definitions start at packet 3, or after the last
    /// pointer packet when the sub-page needs packet 3 or 4 for pointers.
    #[test]
    fn pointer_packets_past_the_second_take_definition_triplets() {
        assert_eq!([0, 7, 8, 15, 16, 23, 24, 31].map(pointer_triplets), [0, 0, 0, 0, 13, 13, 26, 26]);
    }
}
