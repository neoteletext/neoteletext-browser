//! The files passed between the standalone runner's two processes
//! (`script_run::blobs`): the unconfined process reads what the confined
//! worker wrote as untrusted. Arbitrary bytes are read or refused, never
//! a panic, with nothing allocated past what was sent; every name read
//! is safe as a file name and given once; and what is read is written
//! back as the same bytes.
#![no_main]

use libfuzzer_sys::fuzz_target;
use script_run::blobs::{is_name, read, write};
use script_run::limits::{MAX_BLOBS, MAX_BLOBS_BYTES};

fuzz_target!(|data: &[u8]| {
    let mut input = data;
    let Ok(blobs) = read(&mut input) else { return };
    assert!(blobs.len() <= MAX_BLOBS);
    assert!(blobs.iter().map(|(_, bytes)| bytes.len()).sum::<usize>() <= MAX_BLOBS_BYTES.min(data.len()));
    for (index, (name, _)) in blobs.iter().enumerate() {
        assert!(is_name(name) && !name.contains('/') && !name.contains(".."), "{name}");
        assert!(blobs.iter().skip(index + 1).all(|(other, _)| other != name), "{name} is given twice");
    }
    let mut written = Vec::new();
    write(&mut written, &blobs).expect("what was read is within every bound");
    assert_eq!(written, &data[..data.len() - input.len()]);
});
