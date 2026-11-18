//! The kind bytes of the messages between the trusted core and a
//! fetcher.

pub(super) const KIND_CONTENTS: u8 = 80;
pub(super) const KIND_PAGE: u8 = 81;
pub(super) const KIND_NOTICE: u8 = 82;
#[cfg(feature = "links")]
pub(super) const KIND_LINKS: u8 = 83;
#[cfg(feature = "interaction")]
pub(super) const KIND_INPUT: u8 = 84;
pub(super) const KIND_PUSHING: u8 = 85;
pub(super) const KIND_EXCHANGE: u8 = 86;
pub(super) const KIND_PROXY_FAILED: u8 = 87;
// 88 to 91, 97 and 98 were the cookie messages, removed with cookies
// (DEC-6471); never reused.
pub(super) const KIND_CERTIFICATE_REFUSED: u8 = 92;
pub(super) const KIND_CONFINED: u8 = 93;
#[cfg(feature = "interaction")]
pub(super) const KIND_SEND: u8 = 96;
// A part of a page's list of sub-pages, `NNN/P.sub` (REQ-1888, decision
// 112), in the first numbers free across every pipe after the decoder's.
pub(super) const KIND_SUBPAGES: u8 = 105;
pub(super) const KIND_SUBPAGES_MISSING: u8 = 106;
pub(super) const KIND_FETCH_SUBPAGES: u8 = 107;
pub(super) const KIND_FETCH_SUBPAGE: u8 = 108;
// The push connection's round trip, measured by WebSocket ping and pong
// while keys go to the channel (`RETURN-CHANNEL.md` REQ-RCH-016), in the
// next numbers free across every pipe.
#[cfg(feature = "interaction")]
pub(super) const KIND_MEASURE_ROUND_TRIP: u8 = 109;
#[cfg(feature = "interaction")]
pub(super) const KIND_ROUND_TRIP: u8 = 110;
// What a live channel's push connection says beyond its pages
// (neoteletext-common's DEC-1459): its LOG records, the count of invalid
// packets it carried (REQ-1462) and whether it is still connecting
// (DEC-1467), in the next numbers free across every pipe.
pub(super) const KIND_CHANNEL_LOG: u8 = 130;
pub(super) const KIND_INVALID_PACKETS: u8 = 131;
pub(super) const KIND_CONNECTING: u8 = 132;
pub(super) const KIND_INITIAL_PAGE: u8 = 133;
// A redirect that leaves the channel (REQ-701), in the next number free
// across every pipe.
pub(super) const KIND_REDIRECTED: u8 = 134;
// A REDIRECT record of the channel's program (DOC-1960), to a
// page of it or to another channel's address.
pub(super) const KIND_PAGE_REDIRECT: u8 = 135;
pub(super) const KIND_PROGRAM_REDIRECT: u8 = 136;
/// A selection the channel refused (DOC-1961).
pub(super) const KIND_DENIED: u8 = 137;
/// A channel's default character set region (REQ-WEB-002).
pub(super) const KIND_REGION: u8 = 138;
// A static channel's refused response (REQ-708), past the last number used
// across every pipe.
pub(super) const KIND_REFUSED: u8 = 140;
// A static channel's pages as they are wanted (REQ-703), and its refresh
// (REQ-704), in the next numbers.
pub(super) const KIND_NO_CONTENTS: u8 = 141;
pub(super) const KIND_REFRESH: u8 = 142;
pub(super) const KIND_FETCH_PAGE: u8 = 143;
// A part of a page's list that could not be fetched (REQ-1888), in the next
// number.
pub(super) const KIND_SUBPAGES_FAILED: u8 = 144;
// A static channel's bundle settled, and its listing asked for when it is
// wanted (REQ-703, DEC-997). 150 and 151, clear of the numbers other work
// in flight on 3 November 2026 may take next.
pub(super) const KIND_BUNDLE_SETTLED: u8 = 150;
pub(super) const KIND_FETCH_CONTENTS: u8 = 151;
// A connection whose opening ran out of its deadline (TODO-1277), in the
// next number.
pub(super) const KIND_TIMED_OUT: u8 = 145;
// Viewer state (neoteletext-common's `docs/viewer-state.md` VS-119), in
// numbers clear of those other work in flight on 18 November 2026 may take
// next: the fetcher's, then the trusted core's.
pub(super) const KIND_STATE_CHALLENGED: u8 = 170;
pub(super) const KIND_STATE_END_RECEIVED: u8 = 171;
pub(super) const KIND_STATE_KEEP_OFFERED: u8 = 172;
pub(super) const KIND_STATE_ANSWER: u8 = 173;
pub(super) const KIND_STATE_SEND_OFFER: u8 = 174;
pub(super) const KIND_STATE_SEND_NONE: u8 = 175;
// A RULES_QUERY record the channel's push connection carried (page-rules.md
// RULE-47, RULE-52 item 6), in the next number free across every pipe
// after the decoder's rule messages.
pub(super) const KIND_RULES_QUERY: u8 = 186;
