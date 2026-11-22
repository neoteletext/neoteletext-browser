//! What a decoder reads from its script host, which runs a channel's
//! script and is untrusted (the script API's specification section 11.2,
//! DEC-7266, TODO-7274): a step's batch of typed sections. Every batch
//! read keeps to its bounds and holds only the sections a batch has, so
//! it is written again as it was read, within one frame; and the script
//! host's reader takes none of it.
#![no_main]

use libfuzzer_sys::fuzz_target;
use protocol::script::batch::{
    FIRST_SCRIPT_ROW, LAST_SCRIPT_ROW, MAX_BATCH_BYTES, MAX_LABELS, MAX_LOCAL_TRIPLETS, MAX_NAPLPS_OBJECTS, MAX_SOUND_OPERATIONS, MAX_SPRITES,
    MAX_TELETEXT_OBJECTS,
};
use protocol::script::{
    Batch, FromScriptHost, MAX_LOG_CHARS, MAX_LOG_LINES, MAX_STEP_EVENTS, is_to_decoder_kind, is_to_host_kind, read_to_decoder, read_to_host, write_to_decoder,
};

/// The bytes of a frame before its payload: its kind and its length.
const HEADER_BYTES: usize = 5;

/// Holds a batch to what its reader promises the decoder.
fn check(batch: &Batch) {
    assert!(batch.verdicts.as_ref().is_none_or(|verdicts| verdicts.len() <= MAX_STEP_EVENTS));
    // Rows 0 and 24 are never a script's.
    assert!(batch.teletext_rows.iter().flatten().all(|row| (FIRST_SCRIPT_ROW..=LAST_SCRIPT_ROW).contains(&row.row)));
    assert!(batch.teletext_enhancement.as_ref().is_none_or(|triplets| triplets.len() <= MAX_LOCAL_TRIPLETS));
    assert!(batch.teletext_objects.as_ref().is_none_or(|objects| objects.len() <= MAX_TELETEXT_OBJECTS));
    assert!(batch.sprites.as_ref().is_none_or(|sprites| sprites.len() <= MAX_SPRITES));
    assert!(batch.animate.iter().flatten().all(|operation| operation.check().is_ok()));
    assert!(batch.naplps_objects.as_ref().is_none_or(|objects| objects.objects.len() <= MAX_NAPLPS_OBJECTS && (!objects.one || objects.objects.len() == 1)));
    assert!(batch.sound.as_ref().is_none_or(|operations| operations.len() <= MAX_SOUND_OPERATIONS));
    assert!(batch.labels.as_ref().is_none_or(|labels| labels.len() <= MAX_LABELS));
    assert!(batch.log.as_ref().is_none_or(|lines| lines.len() <= MAX_LOG_LINES && lines.iter().all(|line| line.text.chars().count() <= MAX_LOG_CHARS)));
    assert!(batch.title.iter().chain(&batch.navigate).all(|text| !text.chars().any(char::is_control)));
}

fuzz_target!(|data: &[u8]| {
    let mut input = data;
    while let Ok(Some(message)) = read_to_decoder(&mut input) {
        if let FromScriptHost::Batch(batch) = &message {
            check(batch);
        }
        let mut written = Vec::new();
        write_to_decoder(&mut written, &message).expect("what was read is within its bounds");
        assert!(written.len() <= HEADER_BYTES + MAX_BATCH_BYTES, "a batch is always one frame");
        assert!(written.first().is_some_and(|&kind| is_to_decoder_kind(kind) && !is_to_host_kind(kind)));
        assert_eq!(read_to_decoder(&mut written.as_slice()).ok().flatten(), Some(message));
        assert!(read_to_host(&mut written.as_slice()).is_err(), "a script host's message is not a decoder's");
    }
});
