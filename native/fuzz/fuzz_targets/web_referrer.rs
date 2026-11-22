//! The referrer a script's request tells (the Referrer Policy standard,
//! sections 8.3 and 8.4; the script API's specification section 11.4,
//! target 21; `fetcher::web_fuzzing::referrer`, over `referrer::referer`
//! and `referrer::policy_of`): every policy, an arbitrary page and
//! target, of one channel or not, and an answer's `Referrer-Policy`
//! value. The opening asserts that nothing is told under no-referrer,
//! never a fragment or a user name, only what the policy allows, and
//! nothing to another channel under same-origin.
#![no_main]

use libfuzzer_sys::fuzz_target;

/// What parts the page, the target and the header.
const SEPARATOR: u8 = 0;

fuzz_target!(|data: &[u8]| {
    let Some((&[policy, same], rest)) = data.split_first_chunk::<2>() else { return };
    let mut fields = rest.splitn(3, |&byte| byte == SEPARATOR);
    let page = String::from_utf8_lossy(fields.next().unwrap_or_default());
    let page = if page.is_empty() { "https://user:pw@a.example/news/100?q=1#top" } else { &page };
    let target = String::from_utf8_lossy(fields.next().unwrap_or_default());
    let header = fields.next().unwrap_or_default();
    core::hint::black_box(fetcher::web_fuzzing::referrer(policy, page, &target, same & 1 != 0, header));
});
