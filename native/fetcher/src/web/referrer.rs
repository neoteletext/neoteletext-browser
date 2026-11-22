//! The Referrer Policy standard: the `Referer` a request carries (its
//! section 8.3, "determine request's referrer") and the policy an answer
//! sets for the redirects after it (section 8.1).

use protocol::web::{Header, ReferrerPolicy};
use url::Url;

use super::limits::MAX_REFERRER;

/// The policy when none is stated (the standard's section 3: the default
/// referrer policy).
pub(crate) const DEFAULT_POLICY: ReferrerPolicy = ReferrerPolicy::StrictOriginWhenCrossOrigin;

/// Whether a request to `url` is over a channel the standard trusts not to
/// leak a referrer: `https`, or this computer itself.
fn is_trustworthy(url: &Url) -> bool {
    url.scheme() == "https"
        || match url.host() {
            Some(url::Host::Domain(name)) => name == "localhost" || name.ends_with(".localhost"),
            Some(url::Host::Ipv4(address)) => address.is_loopback(),
            Some(url::Host::Ipv6(address)) => address.is_loopback(),
            None => false,
        }
}

/// `url` as a referrer: without its user name, password and fragment, and
/// its origin alone when asked or when it is longer than
/// [`MAX_REFERRER`] (the standard's section 8.4).
fn stripped(url: &Url, origin_only: bool) -> Option<String> {
    if !matches!(url.scheme(), "http" | "https") {
        return None;
    }
    let mut url = url.clone();
    url.set_fragment(None);
    // An `http` or `https` address always takes these.
    url.set_username("").ok()?;
    url.set_password(None).ok()?;
    if !origin_only && url.as_str().len() <= MAX_REFERRER {
        return Some(url.into());
    }
    url.set_path("/");
    url.set_query(None);
    Some(url.into())
}

/// The `Referer` value of a request to `target` from the page `referrer`
/// under `policy`, or none. `same` is whether the target is inside the
/// asking page's own channel, which stands where the standard asks whether
/// the request is same-origin (neoteletext-common's DEC-7264 item 3): a
/// page's address is not told whole to another channel of its origin.
pub(crate) fn referer(policy: ReferrerPolicy, referrer: &Url, target: &Url, same: bool) -> Option<String> {
    let downgrade = is_trustworthy(referrer) && !is_trustworthy(target);
    match policy {
        ReferrerPolicy::NoReferrer => None,
        ReferrerPolicy::Origin => stripped(referrer, true),
        ReferrerPolicy::UnsafeUrl => stripped(referrer, false),
        ReferrerPolicy::StrictOrigin => (!downgrade).then(|| stripped(referrer, true)).flatten(),
        ReferrerPolicy::Unset | ReferrerPolicy::StrictOriginWhenCrossOrigin => {
            if same {
                stripped(referrer, false)
            } else {
                (!downgrade).then(|| stripped(referrer, true)).flatten()
            }
        }
        ReferrerPolicy::SameOrigin => same.then(|| stripped(referrer, false)).flatten(),
        ReferrerPolicy::OriginWhenCrossOrigin => stripped(referrer, !same),
        ReferrerPolicy::NoReferrerWhenDowngrade => (!downgrade).then(|| stripped(referrer, false)).flatten(),
    }
}

/// The policy the answer's `Referrer-Policy` headers set: the last token
/// that is a referrer policy (the standard's section 8.1), or none. A
/// policy is one of the standard's strings as it writes them, in lower
/// case: a token in other case is none.
pub(crate) fn policy_of(headers: &[Header]) -> Option<ReferrerPolicy> {
    let mut policy = None;
    for (_, value) in headers.iter().filter(|(name, _)| name == "referrer-policy") {
        for token in value.split(|&byte| byte == b',') {
            let named = match token.trim_ascii() {
                b"no-referrer" => ReferrerPolicy::NoReferrer,
                b"no-referrer-when-downgrade" => ReferrerPolicy::NoReferrerWhenDowngrade,
                b"same-origin" => ReferrerPolicy::SameOrigin,
                b"origin" => ReferrerPolicy::Origin,
                b"strict-origin" => ReferrerPolicy::StrictOrigin,
                b"origin-when-cross-origin" => ReferrerPolicy::OriginWhenCrossOrigin,
                b"strict-origin-when-cross-origin" => ReferrerPolicy::StrictOriginWhenCrossOrigin,
                b"unsafe-url" => ReferrerPolicy::UnsafeUrl,
                _ => continue,
            };
            policy = Some(named);
        }
    }
    policy
}

#[cfg(test)]
mod tests {
    use super::*;

    fn value(policy: ReferrerPolicy, referrer: &str, target: &str, same: bool) -> Option<String> {
        referer(policy, &Url::parse(referrer).unwrap(), &Url::parse(target).unwrap(), same)
    }

    /// The standard's table (section 3), with the page
    /// `https://a.example/home/100?q#f` asking inside its channel, another
    /// origin over `https`, and one over `http`.
    #[test]
    fn each_policy_sends_what_the_standard_says() {
        use ReferrerPolicy::{
            NoReferrer, NoReferrerWhenDowngrade, Origin, OriginWhenCrossOrigin, SameOrigin, StrictOrigin, StrictOriginWhenCrossOrigin, UnsafeUrl, Unset,
        };
        let page = "https://user:pw@a.example/home/100?q#f";
        let full = Some("https://a.example/home/100?q".to_owned());
        let origin = Some("https://a.example/".to_owned());
        let cases = [
            (NoReferrer, None, None, None),
            (NoReferrerWhenDowngrade, full.clone(), full.clone(), None),
            (SameOrigin, full.clone(), None, None),
            (Origin, origin.clone(), origin.clone(), origin.clone()),
            (StrictOrigin, origin.clone(), origin.clone(), None),
            (OriginWhenCrossOrigin, full.clone(), origin.clone(), origin.clone()),
            (StrictOriginWhenCrossOrigin, full.clone(), origin.clone(), None),
            (Unset, full.clone(), origin.clone(), None),
            (UnsafeUrl, full.clone(), full.clone(), full.clone()),
        ];
        for (policy, inside, other, downgraded) in cases {
            assert_eq!(value(policy, page, "https://a.example/home/200", true), inside, "{policy:?} inside the channel");
            assert_eq!(value(policy, page, "https://b.example/x", false), other, "{policy:?} to another origin");
            assert_eq!(value(policy, page, "http://b.example/x", false), downgraded, "{policy:?} downgraded");
        }
        // DEC-7264: another channel of the same origin is told the origin
        // alone, never the page.
        assert_eq!(value(Unset, page, "https://a.example/demosite/1", false), origin);
    }

    #[test]
    fn a_long_or_unusual_referrer_is_cut_or_withheld() {
        let long = format!("https://a.example/{}", "a".repeat(MAX_REFERRER));
        assert_eq!(value(ReferrerPolicy::UnsafeUrl, &long, "https://a.example/x", true), Some("https://a.example/".to_owned()));
        assert_eq!(value(ReferrerPolicy::UnsafeUrl, "data:text/plain,x", "https://a.example/x", false), None);
        // To this computer itself is no downgrade.
        assert_eq!(value(ReferrerPolicy::StrictOrigin, "https://a.example/", "http://127.0.0.1:8000/", false), Some("https://a.example/".to_owned()));
    }

    #[test]
    fn an_answers_policy_is_its_last_known_token() {
        let headers = |values: &[&str]| values.iter().map(|value| ("referrer-policy".to_owned(), value.as_bytes().to_vec())).collect::<Vec<_>>();
        assert_eq!(policy_of(&headers(&["no-referrer"])), Some(ReferrerPolicy::NoReferrer));
        assert_eq!(policy_of(&headers(&["no-referrer, strict-origin , future-policy"])), Some(ReferrerPolicy::StrictOrigin));
        assert_eq!(policy_of(&headers(&["no-referrer, Strict-Origin"])), Some(ReferrerPolicy::NoReferrer), "a token in other case is no policy");
        assert_eq!(policy_of(&headers(&["origin", "unsafe-url"])), Some(ReferrerPolicy::UnsafeUrl));
        assert_eq!(policy_of(&headers(&["never"])), None);
        assert_eq!(policy_of(&[]), None);
    }
}
