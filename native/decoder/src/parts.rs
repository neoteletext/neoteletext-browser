//! Sending a picture as the parts of it that changed (REQ-6399): what was
//! last sent of a sub-page's picture, the areas in which the next one
//! differs, and the numbered parts that carry them. The ReGIS and the
//! NAPLPS pictures are both sent this way.

use protocol::regis::{PictureLanguage, REGIS_COLOURS, RegisArea, RegisPick, RegisPicture, pack_area};

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
/// one byte a pixel and `width` pixels across: each a run of changed rows and, within it, a run of
/// changed columns, its left edge and width even as pixels are packed.
/// Changes far apart, down or across the picture, are separate areas, so
/// that two small changes are not sent as everything between them
/// (REQ-6399). None when the two are the same.
pub(crate) fn changed_areas(old: &[u8], new: &[u8], width: usize) -> Vec<RegisArea> {
    let narrow = |value: usize| u16::try_from(value).unwrap_or(u16::MAX);
    // Runs of changed rows: (top, bottom), both inclusive.
    let mut bands: Vec<(usize, usize)> = Vec::new();
    for (row, (before, after)) in old.chunks_exact(width.max(1)).zip(new.chunks_exact(width.max(1))).enumerate() {
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
        let mut changed = vec![false; width];
        let rows = bottom.saturating_sub(top).saturating_add(1);
        for (before, after) in old.chunks_exact(width.max(1)).zip(new.chunks_exact(width.max(1))).skip(top).take(rows) {
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
            let right = right.saturating_add(1).next_multiple_of(2).min(width);
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

/// The parts to send of a picture drawn in `figures.language`, whose
/// `pixels`, one byte each, `palette`
/// and `pick` record are given, and whether its pick record differs from
/// the one last sent: the areas that changed since `sent`, each a part of
/// the one before it; or the whole picture when nothing was sent or the
/// receiver holds another sub-page's (`same_sub` false). A picture that
/// changed nowhere is a part of no pixels, which carries its new figures
/// and its colours: new colours alone send no pixels (ANIM-5). `sent` is brought up to date. `None` when the
/// pixels are not a whole picture of colours below sixteen.
#[allow(clippy::too_many_arguments, reason = "the picture, its colours, its pick record and what the renderer knows changed, each from its own source")]
pub(crate) fn parts_to_send(
    sent: &mut Option<Sent>,
    same_sub: bool,
    pixels: &[u8],
    palette: [[u8; 3]; REGIS_COLOURS],
    pick: &RegisPick,
    rows: Changed,
    figures: Figures,
) -> Option<(Vec<RegisPicture>, bool)> {
    // A part may carry new colours: the interface colours the picture
    // again from the pixels it holds, so a change of colours alone, as a
    // NAPLPS BLINK makes, sends no pixels (animation.md ANIM-5).
    let base = sent.as_mut().filter(|sent| same_sub && sent.pixels.len() == pixels.len());
    // Only the rows the renderer says changed are compared and kept: the
    // rest are the picture last sent (RULE-65's millisecond for the
    // repaint; a whole comparison and copy of the picture took up to 7).
    let language = figures.language;
    let width = usize::from(language.width());
    let span = |rows: (usize, usize)| rows.0.saturating_mul(width)..rows.1.saturating_add(1).saturating_mul(width).min(pixels.len());
    let (areas, base_number, pick_changed) = match &base {
        Some(sent) => {
            let areas = match rows {
                Changed::Unknown => changed_areas(&sent.pixels, pixels, width),
                Changed::Nothing => Vec::new(),
                Changed::Rows(first, last) => {
                    let within = span((first, last));
                    let narrow = |value: usize| u16::try_from(value).unwrap_or(u16::MAX);
                    let mut areas = changed_areas(sent.pixels.get(within.clone()).unwrap_or(&[]), pixels.get(within).unwrap_or(&[]), width);
                    for area in &mut areas {
                        area.y = area.y.saturating_add(narrow(first));
                    }
                    areas
                }
            };
            (areas, sent.number, sent.pick != *pick)
        }
        None => (vec![RegisArea::whole(language)], 0, true),
    };
    let mut parts = Vec::with_capacity(areas.len().max(1));
    let mut number = base_number;
    for area in if areas.is_empty() { vec![RegisArea::default()] } else { areas } {
        let Some(packed) = pack_area(pixels, area, language) else {
            // What was sent is no longer known: the next picture goes whole.
            *sent = None;
            return None;
        };
        let before = number;
        number = next_number(number);
        parts.push(RegisPicture { commits: figures.commits, work: figures.work, language: figures.language, palette, number, base: before, area, packed });
    }
    match base {
        Some(sent) => {
            match rows {
                Changed::Unknown => sent.pixels.copy_from_slice(pixels),
                Changed::Nothing => {}
                Changed::Rows(first, last) => {
                    let within = span((first, last));
                    if let (Some(kept), Some(new)) = (sent.pixels.get_mut(within.clone()), pixels.get(within)) {
                        kept.copy_from_slice(new);
                    }
                }
            }
            sent.palette = palette;
            sent.number = number;
            if pick_changed {
                sent.pick.clone_from(pick);
            }
        }
        None => *sent = Some(Sent { pixels: pixels.to_vec(), palette, number, pick: pick.clone() }),
    }
    Some((parts, pick_changed))
}

/// What the renderer says changed in a picture since the one it composed
/// before, which is the one last sent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Changed {
    /// It cannot say: the whole picture is compared.
    Unknown,
    /// No pixel changed.
    Nothing,
    /// The rows from the first to the last, both included.
    Rows(usize, usize),
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing, reason = "a test")]
mod tests {
    use super::*;
    use protocol::regis::{REGIS_HEIGHT, REGIS_WIDTH};

    /// ANIM-5: a picture whose colours alone changed, as a NAPLPS BLINK
    /// makes, is sent as a part of no pixels carrying the new colours.
    #[test]
    fn new_colours_alone_send_no_pixels() {
        let pixels = vec![1_u8; PictureLanguage::Naplps.pixels()];
        let mut palette = [[0_u8; 3]; REGIS_COLOURS];
        palette[1] = [255, 255, 255];
        let figures = Figures { commits: 0, work: 0, language: PictureLanguage::Naplps };
        let mut sent = None;
        let (first, _) = parts_to_send(&mut sent, false, &pixels, palette, &RegisPick::default(), Changed::Unknown, figures).unwrap();
        assert!(first[0].whole());
        palette[1] = [255, 0, 0];
        let (blinked, _) = parts_to_send(&mut sent, true, &pixels, palette, &RegisPick::default(), Changed::Unknown, figures).unwrap();
        assert_eq!(blinked.len(), 1);
        assert!(!blinked[0].whole() && blinked[0].packed.is_empty() && blinked[0].area == RegisArea::default(), "no pixels");
        assert_eq!(blinked[0].palette, palette, "the new colours");
    }

    /// RULE-65: the rows the renderer says changed give the same parts as
    /// comparing the whole picture, and a picture that changed nowhere
    /// sends one part of no pixels.
    #[test]
    fn the_changed_rows_give_the_parts_the_whole_comparison_gives() {
        let mut pixels = vec![1_u8; REGIS_WIDTH * REGIS_HEIGHT];
        let palette = [[0_u8; 3]; REGIS_COLOURS];
        let figures = Figures { commits: 0, work: 0, language: PictureLanguage::Regis };
        let (mut whole, mut hinted) = (None, None);
        for sent in [&mut whole, &mut hinted] {
            parts_to_send(sent, false, &pixels, palette, &RegisPick::default(), Changed::Unknown, figures).unwrap();
        }
        for row in 100..=110 {
            pixels[row * REGIS_WIDTH + 30] = 2;
            pixels[row * REGIS_WIDTH + 700] = 3;
        }
        let (all, _) = parts_to_send(&mut whole, true, &pixels, palette, &RegisPick::default(), Changed::Unknown, figures).unwrap();
        let (rows, _) = parts_to_send(&mut hinted, true, &pixels, palette, &RegisPick::default(), Changed::Rows(100, 110), figures).unwrap();
        let areas = |parts: &[RegisPicture]| parts.iter().map(|part| (part.area, part.packed.clone())).collect::<Vec<_>>();
        assert_eq!(areas(&rows), areas(&all));
        assert_eq!(all.len(), 2, "two areas apart");
        let (nothing, _) = parts_to_send(&mut hinted, true, &pixels, palette, &RegisPick::default(), Changed::Nothing, figures).unwrap();
        assert_eq!(nothing.len(), 1);
        assert_eq!(nothing[0].packed, [] as [u8; 0]);
        let (again, _) = parts_to_send(&mut hinted, true, &pixels, palette, &RegisPick::default(), Changed::Unknown, figures).unwrap();
        assert_eq!(again[0].packed, [] as [u8; 0], "what the rows kept is the picture sent");
    }
}
