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
pub(super) const KIND_COOKIE_QUESTION: u8 = 88;
pub(super) const KIND_REACHED_QUESTION: u8 = 89;
pub(super) const KIND_COOKIE_SET: u8 = 90;
pub(super) const KIND_CLEAR_SITE_DATA: u8 = 91;
pub(super) const KIND_CERTIFICATE_REFUSED: u8 = 92;
pub(super) const KIND_CONFINED: u8 = 93;
#[cfg(feature = "interaction")]
pub(super) const KIND_SEND: u8 = 96;
pub(super) const KIND_COOKIE_ANSWER: u8 = 97;
pub(super) const KIND_REACHED_ANSWER: u8 = 98;
// A page's catalogue of sub-pages, `NNN.sub` (the engine's REQ-WEB-034,
// decision 112), in the first numbers free across every pipe after the
// decoder's.
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
