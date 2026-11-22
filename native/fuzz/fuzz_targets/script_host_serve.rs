//! Everything a script host is given (the script API's specification
//! sections 11.1 and 11.2, TODO-7292): arbitrary bytes as its input,
//! once as they stand and once after a greeting, so that openings,
//! source pieces, steps, closes and the web client's answers in any
//! order reach the code behind it. Whatever it is given, the script host
//! does not panic; it says READY first; everything it writes is a
//! message the trusted core relays and the decoder's reader takes whole;
//! every step and close it took is answered by a batch or the run ends;
//! and it ends.
#![no_main]

use std::sync::OnceLock;
use std::time::Duration;

use libfuzzer_sys::fuzz_target;
use neoteletext_script::{Engine, Settings, TimeLimits};
use protocol::script::relay::{HostOutput, read_host_output};
use protocol::script::{FromScriptHost, LINK_VERSION, ToScriptHost, read_to_decoder, write_to_host};
use script_host::{End, serve};

/// The limits the inputs run under: the browser's shapes, a tenth to a
/// twentieth of their lengths, as the script library's own target has.
const LIMITS: TimeLimits = TimeLimits {
    start: Duration::from_millis(60),
    callback: Duration::from_millis(10),
    step: Duration::from_millis(12),
    second: Duration::from_millis(100),
    close: Duration::from_millis(10),
};

fn engine() -> &'static Engine {
    static ENGINE: OnceLock<Engine> = OnceLock::new();
    ENGINE.get_or_init(|| Engine::new().unwrap_or_else(|stop| panic!("the module does not compile: {stop}")))
}

/// Serves `input` and holds what was written to the link's rules.
fn run(input: &[u8]) {
    let mut output = Vec::new();
    let settings = Settings { time: LIMITS, watchdog: None };
    // A seed that is the same every time, so that a fault is found again.
    let end = serve(&mut &*input, &mut output, engine(), &settings, &mut |seed| {
        seed.fill(0x5A);
        true
    });
    let mut written = output.as_slice();
    let mut said = Vec::new();
    while let Some(relayed) = read_host_output(&mut written).unwrap_or_else(|error| panic!("the trusted core would not relay what it wrote: {error}")) {
        if let HostOutput::ToDecoder(relayed) = relayed {
            let message = read_to_decoder(&mut relayed.frame()).unwrap_or_else(|error| panic!("the decoder would refuse it: {error}"));
            said.push(message.unwrap_or_else(|| panic!("an empty message")));
        }
    }
    assert!(matches!(said.first(), Some(FromScriptHost::Ready { link_version: LINK_VERSION, .. })), "READY is said first");
    assert_eq!(said.iter().filter(|message| matches!(message, FromScriptHost::Ready { .. })).count(), 1, "READY is said once");
    let failed = said.iter().any(|message| matches!(message, FromScriptHost::Failed { .. }));
    match end {
        End::Told => assert!(!failed, "it failed and ended as if told to"),
        End::Stopped(_) | End::Link(_) => {}
    }
}

fuzz_target!(|data: &[u8]| {
    run(data);
    let mut greeted = Vec::new();
    write_to_host(&mut greeted, &ToScriptHost::Hello { link_version: LINK_VERSION }).unwrap_or_else(|error| panic!("the greeting: {error}"));
    greeted.extend_from_slice(data);
    run(&greeted);
});
