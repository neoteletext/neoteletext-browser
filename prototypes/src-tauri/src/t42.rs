//! Rules for what the URL bar may open. The chrome shows these error
//! messages verbatim, so they are written for people.

use url::Url;

const EXTENSION: &str = ".t42";
// Far above any real .t42 address; anything longer is a mistake or abuse.
pub const MAX_SRC_LENGTH: usize = 2048;
const ALLOWED_SCHEMES: [&str; 3] = ["https", "http", "file"];

pub fn validate(src: &str) -> Result<Url, String> {
    let trimmed = src.trim();
    if trimmed.is_empty() {
        return Err("Enter the address of a .t42 file".to_owned());
    }
    if trimmed.len() > MAX_SRC_LENGTH {
        return Err(format!("Addresses are limited to {MAX_SRC_LENGTH} characters"));
    }
    let url = Url::parse(trimmed).map_err(|_| "That isn't a full address. Include https:// or file://".to_owned())?;
    if !ALLOWED_SCHEMES.contains(&url.scheme()) {
        return Err(format!("{}: addresses can't be opened. Use https, http or file", url.scheme()));
    }
    let file_name = url.path_segments().and_then(Iterator::last).unwrap_or_default();
    let is_t42 = file_name.len() > EXTENSION.len() && file_name.to_ascii_lowercase().ends_with(EXTENSION);
    if !is_t42 {
        return Err("Only .t42 files can be opened".to_owned());
    }
    Ok(url)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_t42_over_allowed_schemes() {
        for src in [
            "https://example.com/pictures/wall.t42",
            "http://localhost:8080/wall.t42",
            "file:///Users/someone/Pictures/wall.T42",
            "  https://example.com/a/b/c.t42?version=2  ",
        ] {
            assert!(validate(src).is_ok(), "{src} should be accepted");
        }
    }

    #[test]
    fn rejects_other_files_and_schemes() {
        let cases = [
            ("", "Enter the address of a .t42 file"),
            ("wall.t42", "That isn't a full address. Include https:// or file://"),
            ("https://example.com/wall.png", "Only .t42 files can be opened"),
            ("https://example.com/", "Only .t42 files can be opened"),
            ("https://example.com/.t42", "Only .t42 files can be opened"),
            ("https://example.com/wall.t42/", "Only .t42 files can be opened"),
            ("javascript:alert(1)//x.t42", "javascript: addresses can't be opened. Use https, http or file"),
            ("ftp://example.com/wall.t42", "ftp: addresses can't be opened. Use https, http or file"),
        ];
        for (src, message) in cases {
            assert_eq!(validate(src).unwrap_err(), message, "for {src:?}");
        }
    }

    #[test]
    fn rejects_overlong_addresses() {
        let src = format!("https://example.com/{}.t42", "a".repeat(MAX_SRC_LENGTH));
        assert_eq!(validate(&src).unwrap_err(), format!("Addresses are limited to {MAX_SRC_LENGTH} characters"));
    }
}
