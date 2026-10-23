//! The developer console (`settings-home-console-proxy.md` section 3,
//! requirements 19.8 and 19.9): entries the trusted core keeps for each
//! tab's channel and sends, as cleaned text, while the console is open. It
//! never carries a cookie, a form's values, a typed key or a secret.

use super::{Fields, ProtocolError, push_text};

pub use interface_core::console::{ConsoleEntry, ConsoleTab, ConsoleUpdate, KEPT, Level, MAX_ENTRY};
/// The most entries a message carries.
pub const MAX_ENTRIES: usize = 200;

fn tab_byte(tab: ConsoleTab) -> u8 {
    u8::try_from(tab.index()).unwrap_or(u8::MAX)
}

fn tab_from_byte(byte: u8) -> Result<ConsoleTab, ProtocolError> {
    ConsoleTab::ALL.into_iter().find(|&tab| tab_byte(tab) == byte).ok_or(ProtocolError::BadValue("console tab"))
}

fn level_byte(level: Level) -> u8 {
    match level {
        Level::Error => 0,
        Level::Warning => 1,
        Level::Info => 2,
    }
}

fn level_from_byte(byte: u8) -> Result<Level, ProtocolError> {
    Level::ALL.into_iter().find(|&level| level_byte(level) == byte).ok_or(ProtocolError::BadValue("log level"))
}

pub(crate) fn push_update(payload: &mut Vec<u8>, update: &ConsoleUpdate) -> Result<(), ProtocolError> {
    if update.entries.len() > MAX_ENTRIES {
        return Err(ProtocolError::BadValue("too many console entries"));
    }
    payload.extend_from_slice(&update.channel.to_be_bytes());
    payload.push(u8::from(update.replace));
    payload.extend_from_slice(&u16::try_from(update.entries.len()).unwrap_or(0).to_be_bytes());
    for entry in &update.entries {
        payload.extend_from_slice(&[tab_byte(entry.tab), level_byte(entry.level)]);
        payload.extend_from_slice(&entry.at_ms.to_be_bytes());
        push_text(payload, &entry.text, MAX_ENTRY)?;
    }
    Ok(())
}

pub(crate) fn read_update(fields: &mut Fields<'_>) -> Result<ConsoleUpdate, ProtocolError> {
    let channel = fields.u32()?;
    let replace = match fields.u8()? {
        0 => false,
        1 => true,
        _ => return Err(ProtocolError::BadValue("flag")),
    };
    let count = usize::from(fields.u16()?);
    if count > MAX_ENTRIES {
        return Err(ProtocolError::BadValue("too many console entries"));
    }
    let entries = (0..count)
        .map(|_| {
            let tab = tab_from_byte(fields.u8()?)?;
            let level = level_from_byte(fields.u8()?)?;
            let at_ms = u64::from_be_bytes(fields.array()?);
            Ok(ConsoleEntry { tab, level, at_ms, text: fields.text(MAX_ENTRY)? })
        })
        .collect::<Result<Vec<_>, ProtocolError>>()?;
    Ok(ConsoleUpdate { channel, replace, entries })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_update_round_trips_within_its_bounds() {
        let update = ConsoleUpdate {
            channel: 3,
            replace: true,
            entries: ConsoleTab::ALL
                .iter()
                .zip(Level::ALL.iter().cycle())
                .map(|(&tab, &level)| ConsoleEntry { tab, level, at_ms: 1_234, text: tab.label().into() })
                .collect(),
        };
        let mut payload = Vec::new();
        push_update(&mut payload, &update).unwrap();
        assert_eq!(read_update(&mut Fields { bytes: &payload }).unwrap(), update);
        let many = ConsoleUpdate { entries: vec![update.entries[0].clone(); MAX_ENTRIES + 1], ..update.clone() };
        assert!(push_update(&mut Vec::new(), &many).is_err());
        let hidden = ConsoleUpdate { entries: vec![ConsoleEntry { text: "a\u{200b}b".into(), ..update.entries[0].clone() }], ..update };
        assert!(push_update(&mut Vec::new(), &hidden).is_err(), "only cleaned text");
    }
}
