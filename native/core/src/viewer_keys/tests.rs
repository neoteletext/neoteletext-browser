#![allow(clippy::unwrap_used, clippy::indexing_slicing, clippy::panic, clippy::arithmetic_side_effects, reason = "a test")]

use super::*;
use neoteletext_web::url::Url;

const NOW: u64 = 1_800_000_000;
const CHALLENGE: [u8; CHALLENGE_BYTES] = [0x5A; CHALLENGE_BYTES];

fn identity(address: &str) -> ChannelIdentity {
    ChannelIdentity::of(&Url::parse(address).unwrap()).unwrap()
}

fn asked(identity: &ChannelIdentity, create: bool, gesture: bool) -> Asked<'_> {
    Asked { private: false, identity, challenge: &CHALLENGE, create, gesture }
}

/// The proof's public key, or a panic naming the answer.
fn public_key(answer: Answer) -> [u8; PUBLIC_KEY_BYTES] {
    match answer {
        Answer::Proof { public_key, .. } => public_key,
        Answer::None(reason) => panic!("no proof: {reason:?}"),
    }
}

fn none(answer: Answer) -> Option<NoneReason> {
    match answer {
        Answer::None(reason) => Some(reason),
        Answer::Proof { .. } => None,
    }
}

/// VS-101 and VS-203: a key made on a challenge that asks, after a
/// gesture, signs exactly the bytes of VS-203 with Ed25519, and the
/// signature verifies under the public key it sends; the seed is 32 bytes.
#[test]
fn a_made_key_signs_the_bytes_of_vs_203() {
    let mut store = Store::default();
    let channel = identity("https://news.example/teletext/");
    let answer = store.answer(&asked(&channel, true, true), NOW);
    let Answer::Proof { lifetime, public_key, signature } = answer else { panic!("a proof") };
    assert_eq!(lifetime, Lifetime::Visit, "VS-105: made as a visit key");
    let mut bytes = [0_u8; MAX_SIGNED_BYTES];
    let length = signed_bytes(Algorithm::Ed25519, Lifetime::Visit, &CHALLENGE, &channel, &mut bytes).unwrap();
    let verifier = ring::signature::UnparsedPublicKey::new(&ring::signature::ED25519, public_key);
    verifier.verify(&bytes[..length], &signature).unwrap();
    assert!(verifier.verify(&bytes[..length - 1], &signature).is_err(), "only those bytes");
    assert_eq!(store.ordinary["https://news.example/teletext/"].seed.len(), SEED_BYTES);
}

/// VS-104: no key without `create`, none without a gesture, none for a
/// blocked channel or with "Never"; each answered with its reason.
#[test]
fn a_key_is_made_only_when_asked_after_a_gesture() {
    let mut store = Store::default();
    let channel = identity("https://news.example/teletext/");
    assert_eq!(none(store.answer(&asked(&channel, false, true), NOW)), Some(NoneReason::NoKey));
    assert_eq!(none(store.answer(&asked(&channel, true, false), NOW)), Some(NoneReason::NoGesture));
    store.block("https://news.example/teletext/", true);
    assert_eq!(none(store.answer(&asked(&channel, true, true), NOW)), Some(NoneReason::Declined));
    store.block("https://news.example/teletext/", false);
    store.never = true;
    assert_eq!(none(store.answer(&asked(&channel, true, true), NOW)), Some(NoneReason::Declined));
    assert!(store.listed().is_empty(), "nothing was made");
    store.never = false;
    let first = public_key(store.answer(&asked(&channel, true, true), NOW));
    assert_eq!(public_key(store.answer(&asked(&channel, false, false), NOW)), first, "a key once made proves without asking");
}

/// VS-102 and VS-110: four channels that differ by folder, scheme or port,
/// and a private window, have five different keys.
#[test]
fn each_channel_and_jar_has_its_own_key() {
    let mut store = Store::default();
    let addresses = ["https://example.org/news/", "https://example.org/news/sport/", "http://example.org/news/", "https://example.org:8443/news/"];
    let channels: Vec<ChannelIdentity> = addresses.iter().map(|address| identity(address)).collect();
    let mut keys: Vec<[u8; PUBLIC_KEY_BYTES]> = channels.iter().map(|channel| public_key(store.answer(&asked(channel, true, true), NOW))).collect();
    let private = Asked { private: true, ..asked(&channels[0], true, true) };
    keys.push(public_key(store.answer(&private, NOW)));
    let distinct: BTreeSet<[u8; PUBLIC_KEY_BYTES]> = keys.iter().copied().collect();
    assert_eq!(distinct.len(), keys.len());
    assert_eq!(store.listed().len(), addresses.len(), "the private key is never listed");
    store.close_private();
    let again = public_key(store.answer(&private, NOW));
    assert_ne!(again, keys[4], "a new private key after the last private window closed");
}

/// VS-106: a visit key unused for 12 hours goes, and so does one used
/// every hour once 7 days old; the next is a new key.
#[test]
fn visit_keys_keep_their_limits() {
    let mut store = Store::default();
    let channel = identity("https://news.example/teletext/");
    let first = public_key(store.answer(&asked(&channel, true, true), NOW));
    assert_eq!(public_key(store.answer(&asked(&channel, false, false), NOW + VISIT_KEY_IDLE_LIMIT - 1)), first);
    let idle = NOW + 2 * VISIT_KEY_IDLE_LIMIT;
    assert_eq!(none(store.answer(&asked(&channel, false, false), idle)), Some(NoneReason::NoKey), "gone when idle");
    let second = public_key(store.answer(&asked(&channel, true, true), idle));
    assert_ne!(second, first);
    let mut at = idle;
    while at < idle + VISIT_KEY_ABSOLUTE_LIMIT - SECONDS_PER_HOUR {
        at += SECONDS_PER_HOUR;
        assert_eq!(public_key(store.answer(&asked(&channel, false, false), at)), second);
    }
    assert_eq!(none(store.answer(&asked(&channel, false, false), idle + VISIT_KEY_ABSOLUTE_LIMIT)), Some(NoneReason::NoKey), "gone at 7 days");
}

/// VS-107 and VS-109: a remembered key proves as remembered; it goes after
/// 180 days unused, and at 400 days however often it is used.
#[test]
fn remembered_keys_keep_their_limits() {
    let mut store = Store::default();
    let channel = identity("https://news.example/teletext/");
    let key = public_key(store.answer(&asked(&channel, true, true), NOW));
    assert!(store.remember("https://news.example/teletext/", NOW));
    let Answer::Proof { lifetime, .. } = store.answer(&asked(&channel, false, false), NOW) else { panic!("a proof") };
    assert_eq!(lifetime, Lifetime::Remembered);
    let mut at = NOW;
    while at < NOW + REMEMBERED_KEY_ABSOLUTE_LIMIT - SECONDS_PER_DAY {
        at += SECONDS_PER_DAY;
        assert_eq!(public_key(store.answer(&asked(&channel, false, false), at)), key, "used daily");
    }
    assert_eq!(none(store.answer(&asked(&channel, false, false), NOW + REMEMBERED_KEY_ABSOLUTE_LIMIT)), Some(NoneReason::NoKey), "gone at 400 days");
    public_key(store.answer(&asked(&channel, true, true), NOW));
    store.remember("https://news.example/teletext/", NOW);
    let unused = NOW + REMEMBERED_KEY_IDLE_LIMIT;
    assert_eq!(none(store.answer(&asked(&channel, false, false), unused)), Some(NoneReason::NoKey), "gone after 180 days unused");
}

/// VS-113 and VS-204: Forget deletes the key and the next is new; a
/// `STATE_END` deletes the key only when its identifier names it.
#[test]
fn forgetting_and_ending_delete_only_the_key_named() {
    let mut store = Store::default();
    let channel = identity("https://news.example/teletext/");
    let name = "https://news.example/teletext/";
    let first = public_key(store.answer(&asked(&channel, true, true), NOW));
    assert!(store.forget(false, name));
    let second = public_key(store.answer(&asked(&channel, true, true), NOW));
    assert_ne!(first, second);
    assert!(!store.end(false, name, &key_id(Algorithm::Ed25519, &first)), "a late end names the old key");
    assert_eq!(store.known(false, name), Known::Visit);
    assert!(store.end(false, name, &key_id(Algorithm::Ed25519, &second)));
    assert_eq!(store.known(false, name), Known::Not);
}

/// VS-112: forgetting with history takes only the keys made in the range.
#[test]
fn forgetting_with_history_takes_the_keys_made_since() {
    let mut store = Store::default();
    let old = identity("https://old.example/");
    let new = identity("https://new.example/");
    public_key(store.answer(&asked(&old, true, true), NOW));
    public_key(store.answer(&asked(&new, true, true), NOW + 100));
    assert_eq!(store.forget_made_since(NOW + 50), ["https://new.example/"]);
    assert_eq!(store.known(false, "https://old.example/"), Known::Visit);
}

/// VS-105, VS-108 and VS-110: only remembered keys are written; they read
/// back with the same public key; a sealed file opens under its key alone;
/// a line altered or of another channel form is dropped, without its
/// content in the reason.
#[test]
fn only_remembered_keys_are_kept_and_read_back() {
    let mut store = Store::default();
    let kept = identity("https://kept.example/tt/");
    let visit = identity("https://visit.example/");
    let key = public_key(store.answer(&asked(&kept, true, true), NOW));
    public_key(store.answer(&asked(&visit, true, true), NOW));
    public_key(store.answer(&Asked { private: true, ..asked(&kept, true, true) }, NOW));
    store.remember("https://kept.example/tt/", NOW);
    let text = store.export();
    let written = String::from_utf8(text.clone()).unwrap();
    assert_eq!(written.lines().count(), 2, "the version and the one remembered key: {written}");
    assert!(!written.contains("visit.example"));
    let mut read = Store::default();
    assert!(read.import(&text, NOW).is_empty());
    assert_eq!(public_key(read.answer(&asked(&kept, false, false), NOW)), key);
    let mut altered = written.replace("https://kept.example/tt/", "https://kept.example/tt");
    altered.push_str("not\ta\tline\n");
    let dropped = Store::default().import(altered.as_bytes(), NOW);
    assert_eq!(dropped.len(), 2, "{dropped:?}");
    assert!(dropped.lines().iter().all(|reason| !reason.contains("kept.example")), "no content said");
    let sealing = [3; crate::sealed_file::KEY_BYTES];
    let folder = std::env::temp_dir().join(format!("neoteletext-viewer-keys-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&folder);
    file::save(&folder, &sealing, &store).unwrap();
    assert!(matches!(file::load(&folder, &sealing).unwrap(), file::Loaded::Text(opened) if opened == text));
    assert!(matches!(file::load(&folder, &[4; crate::sealed_file::KEY_BYTES]).unwrap(), file::Loaded::NotOpened), "VS-118: another key");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let mode = std::fs::metadata(folder.join(file::FILE_NAME)).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }
    std::fs::remove_dir_all(&folder).unwrap();
}
