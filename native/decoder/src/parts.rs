//! Sending a picture as the parts of it that changed (REQ-6399): what was
//! last sent of a sub-page's picture, the areas in which the next one
//! differs, and the numbered parts that carry them. The ReGIS and the
//! NAPLPS pictures are both sent this way.

use protocol::regis::{PictureLanguage, REGIS_COLOURS, REGIS_WIDTH, RegisArea, RegisPick, RegisPicture, pack_area};

/// What was last sent of a sub-page's picture.
pub(crate) struct Sent {
    /// Its pixels, one byte each.
    pixels: Vec<u8>,
    palette: [[u8; 3]; REGIS_COLOURS],
    /// Its number, which a part names as its base.
    number: u32,
    pick: RegisPick,
}

/// The number of the picture after `number`: the next, never 0, which
/// means no picture.
const fn next_number(number: u32) -> u32 {
    match number.wrapping_add(1) {
        0 => 1,
        next => next,
    }
}

/// Rows of no change that part one changed area from the next: fewer
/// than these between two changed rows, and the rows are one area.
const AREA_GAP_ROWS: usize = 8;
/// Columns of no change that part one changed area from the next within
/// the same rows.
const AREA_GAP_COLUMNS: usize = 16;
/// The most areas one repaint is sent as; a picture that changed in more
/// places is sent as the one area that holds them all.
const MAX_AREAS: usize = 16;

/// The areas in which `new` differs from `old`, both whole pictures of
/// one byte a pixel: each a run of changed rows and, within it, a run of
/// changed columns, its left edge and width even as pixels are packed.
/// Changes far apart, down or across the picture, are separate areas, so
/// that two small changes are not sent as everything between them
/// (REQ-6399). None when the two are the same.
pub(crate) fn changed_areas(old: &[u8], new: &[u8]) -> Vec<RegisArea> {
    let narrow = |value: usize| u16::try_from(value).unwrap_or(u16::MAX);
    // Runs of changed rows: (top, bottom), both inclusive.
    let mut bands: Vec<(usize, usize)> = Vec::new();
    for (row, (before, after)) in old.as_chunks::<REGIS_WIDTH>().0.iter().zip(new.as_chunks::<REGIS_WIDTH>().0).enumerate() {
        if before == after {
            continue;
        }
        match bands.last_mut().filter(|band| row.saturating_sub(band.1) <= AREA_GAP_ROWS) {
            Some(band) => band.1 = row,
            None => bands.push((row, row)),
        }
    }
    // Within each run of rows, runs of changed columns: (top, bottom, left, right).
    let mut areas: Vec<(usize, usize, usize, usize)> = Vec::new();
    for (top, bottom) in bands {
        let mut changed = [false; REGIS_WIDTH];
        let rows = bottom.saturating_sub(top).saturating_add(1);
        for (before, after) in old.as_chunks::<REGIS_WIDTH>().0.iter().zip(new.as_chunks::<REGIS_WIDTH>().0).skip(top).take(rows) {
            for (flag, (was, is)) in changed.iter_mut().zip(before.iter().zip(after)) {
                *flag |= was != is;
            }
        }
        let mut run: Option<(usize, usize)> = None;
        for column in changed.iter().enumerate().filter(|(_, flag)| **flag).map(|(column, _)| column) {
            run = match run {
                Some((left, right)) if column.saturating_sub(right) <= AREA_GAP_COLUMNS => Some((left, column)),
                Some((left, right)) => {
                    areas.push((top, bottom, left, right));
                    Some((column, column))
                }
                None => Some((column, column)),
            };
        }
        if let Some((left, right)) = run {
            areas.push((top, bottom, left, right));
        }
    }
    if areas.len() > MAX_AREAS {
        let all = areas.iter().fold((usize::MAX, 0, usize::MAX, 0), |all, area| (all.0.min(area.0), all.1.max(area.1), all.2.min(area.2), all.3.max(area.3)));
        areas = vec![all];
    }
    areas
        .into_iter()
        .map(|(top, bottom, left, right)| {
            // Outwards to even columns: two pixels travel in a byte.
            let left = left.saturating_sub(left % 2);
            let right = right.saturating_add(1).next_multiple_of(2).min(REGIS_WIDTH);
            RegisArea {
                x: narrow(left),
                y: narrow(top),
                width: narrow(right.saturating_sub(left)),
                height: narrow(bottom.saturating_sub(top).saturating_add(1)),
            }
        })
        .collect()
}

/// What a picture's parts all say of it beside their pixels.
#[derive(Clone, Copy)]
pub(crate) struct Figures {
    /// The count the status line shows: commits applied, or records read.
    pub commits: u32,
    /// What its drawing has cost.
    pub work: u64,
    /// Which language drew it.
    pub language: PictureLanguage,
}

/// The parts to send of a picture whose `pixels`, one byte each, `palette`
/// and `pick` record are given, and whether its pick record differs from
/// the one last sent: the areas that changed since `sent`, each a part of
/// the one before it; or the whole picture when nothing was sent, the
/// receiver holds another sub-page's (`same_sub` false) or its colours
/// changed. A picture that changed nowhere is a part of no pixels, which
/// carries its new figures. `sent` is brought up to date. `None` when the
/// pixels are not a whole picture of colours below sixteen.
pub(crate) fn parts_to_send(
    sent: &mut Option<Sent>,
    same_sub: bool,
    pixels: &[u8],
    palette: [[u8; 3]; REGIS_COLOURS],
    pick: &RegisPick,
    figures: Figures,
) -> Option<(Vec<RegisPicture>, bool)> {
    let base = sent.as_mut().filter(|sent| same_sub && sent.palette == palette && sent.pixels.len() == pixels.len());
    let (areas, base_number, pick_changed) = match &base {
        Some(sent) => (changed_areas(&sent.pixels, pixels), sent.number, sent.pick != *pick),
        None => (vec![RegisArea::WHOLE], 0, true),
    };
    let mut parts = Vec::with_capacity(areas.len().max(1));
    let mut number = base_number;
    for area in if areas.is_empty() { vec![RegisArea::default()] } else { areas } {
        let packed = pack_area(pixels, area)?;
        let before = number;
        number = next_number(number);
        parts.push(RegisPicture { commits: figures.commits, work: figures.work, language: figures.language, palette, number, base: before, area, packed });
    }
    match base {
        Some(sent) => {
            sent.pixels.copy_from_slice(pixels);
            sent.number = number;
            if pick_changed {
                sent.pick.clone_from(pick);
            }
        }
        None => *sent = Some(Sent { pixels: pixels.to_vec(), palette, number, pick: pick.clone() }),
    }
    Some((parts, pick_changed))
}
