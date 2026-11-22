//! A component's options as the script host reads them (script-api.md
//! section 8 and section 11.4, target 23; `neoteletext_script::ui::
//! options::read` and `Props::take`): a script's JSON text, then each
//! of the thirteen kinds of component taking it. Whatever the text,
//! nothing panics; what is read is written back as JSON that reads to
//! the same members; and what a component takes keeps within the bound
//! on a page's components' state.
#![no_main]

use libfuzzer_sys::fuzz_target;
use neoteletext_script::ui::Kind;
use neoteletext_script::ui::limits::MAX_UI_STATE_BYTES;
use neoteletext_script::ui::options::{Props, Value, read};

fuzz_target!(|data: &[u8]| {
    let Ok(members) = read(data) else { return };
    let mut written = String::new();
    Value::Object(members.clone()).write(&mut written);
    assert_eq!(read(written.as_bytes()).ok().as_ref(), Some(&members), "the options written back as {written:?} read differently");
    for kind in Kind::ALL {
        let mut props = Props::default();
        let taken = props.take(kind, &members);
        assert!(props.bytes() <= MAX_UI_STATE_BYTES, "a {kind:?} took {} bytes", props.bytes());
        // Taking is a function of the options alone.
        let mut again = Props::default();
        assert_eq!(again.take(kind, &members), taken);
        assert_eq!(again, props);
    }
});
