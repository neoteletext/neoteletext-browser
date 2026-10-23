//! The lists the address field and the Back and Forward buttons show
//! (`history-and-bookmarks.md` section 5): suggestions for what is typed,
//! and the active tab's Back or Forward list.

use super::{
    MAX_CHAPTER_STOPS, MAX_NAME, MAX_SHOWN_ADDRESS, MAX_STEPS, MAX_SUGGESTIONS, MAX_TAB_LIST, Suggestion, Suggestions, TabEntry, TabList, push_page, read_page,
};
use crate::{Fields, MAX_ADDRESS, ProtocolError, push_count, push_flag, push_text};

pub(crate) fn push_suggestions(payload: &mut Vec<u8>, suggestions: &Suggestions) -> Result<(), ProtocolError> {
    push_text(payload, &suggestions.text, MAX_ADDRESS)?;
    push_text(payload, &suggestions.inline, MAX_ADDRESS)?;
    push_count(payload, suggestions.rows.len(), MAX_SUGGESTIONS, "too many rows")?;
    for row in &suggestions.rows {
        payload.extend_from_slice(&row.number.to_be_bytes());
        push_text(payload, &row.address, MAX_SHOWN_ADDRESS)?;
        push_text(payload, &row.title, MAX_NAME)?;
        push_flag(payload, row.bookmarked);
    }
    Ok(())
}

pub(crate) fn read_suggestions(fields: &mut Fields<'_>) -> Result<Suggestions, ProtocolError> {
    let text = fields.text(MAX_ADDRESS)?;
    let inline = fields.text(MAX_ADDRESS)?;
    let count = fields.count_u16(MAX_SUGGESTIONS, "too many rows")?;
    let rows = (0..count)
        .map(|_| {
            Ok(Suggestion { number: fields.u32()?, address: fields.text(MAX_SHOWN_ADDRESS)?, title: fields.text(MAX_NAME)?, bookmarked: fields.flag("flag")? })
        })
        .collect::<Result<_, ProtocolError>>()?;
    Ok(Suggestions { text, inline, rows })
}

pub(crate) fn push_tab_list(payload: &mut Vec<u8>, list: &TabList) -> Result<(), ProtocolError> {
    push_flag(payload, list.forward);
    for (entries, most) in [(&list.entries, MAX_TAB_LIST), (&list.chapters, MAX_CHAPTER_STOPS)] {
        push_count(payload, entries.len(), most, "too many rows")?;
        for entry in entries {
            if entry.steps == 0 || usize::from(entry.steps) > MAX_STEPS {
                return Err(ProtocolError::BadValue("steps"));
            }
            payload.push(entry.steps);
            push_page(payload, entry.page)?;
            push_text(payload, &entry.title, MAX_NAME)?;
            push_text(payload, &entry.address, MAX_SHOWN_ADDRESS)?;
        }
    }
    Ok(())
}

pub(crate) fn read_tab_list(fields: &mut Fields<'_>) -> Result<TabList, ProtocolError> {
    let forward = fields.flag("flag")?;
    let mut read = |most: usize| -> Result<Vec<TabEntry>, ProtocolError> {
        let count = fields.count_u16(most, "too many rows")?;
        (0..count)
            .map(|_| {
                let steps = read_steps(fields)?;
                Ok(TabEntry { steps, page: read_page(fields)?, title: fields.text(MAX_NAME)?, address: fields.text(MAX_SHOWN_ADDRESS)? })
            })
            .collect()
    };
    let entries = read(MAX_TAB_LIST)?;
    let chapters = read(MAX_CHAPTER_STOPS)?;
    Ok(TabList { forward, entries, chapters })
}

/// How many steps a Back or Forward list's entry is, 1 to [`MAX_STEPS`].
pub(crate) fn read_steps(fields: &mut Fields<'_>) -> Result<u8, ProtocolError> {
    let steps = fields.u8()?;
    if steps == 0 || usize::from(steps) > MAX_STEPS { Err(ProtocolError::BadValue("steps")) } else { Ok(steps) }
}
