//! The XDG desktop portal on Linux (xdg-desktop-portal's documentation of
//! `org.freedesktop.portal.FileChooser`, `org.freedesktop.portal.Request`
//! and `org.freedesktop.portal.Settings`), over the browser's own D-Bus
//! client (DEC-7168). The portal draws the dialog in the desktop's process
//! and answers with the chosen file's address.

use std::path::Path;

use dbus::{Connection, Error, Message, Value};

/// The portal's bus name and object.
const PORTAL: &str = "org.freedesktop.portal.Desktop";
const PORTAL_PATH: &str = "/org/freedesktop/portal/desktop";
const FILE_CHOOSER: &str = "org.freedesktop.portal.FileChooser";
const REQUEST: &str = "org.freedesktop.portal.Request";
const SETTINGS: &str = "org.freedesktop.portal.Settings";
/// `org.freedesktop.portal.Request`'s `Response`: 0 the user's choice, 1
/// cancelled; anything else ended the interaction otherwise.
const RESPONSE_CHOSEN: u32 = 0;
const RESPONSE_CANCELLED: u32 = 1;
/// The request's token (`handle_token`), from which the portal makes the
/// request's object path: `/org/freedesktop/portal/desktop/request/`, the
/// caller's unique name without its `:` and with `_` for each `.`, `/` and
/// the token. One request is made per connection, so one token serves.
const HANDLE_TOKEN: &str = "neoteletext_chooser";
/// `FileChooser`'s filter entries: a glob pattern is kind 0, a media type 1.
const FILTER_GLOB: u32 = 0;
const FILTER_MEDIA_TYPE: u32 = 1;

/// One dialog, as the chooser asks for it.
pub(crate) struct Dialog<'a> {
    /// The window to parent the dialog to: `x11:` and its number in
    /// hexadecimal, or empty.
    pub(crate) parent: String,
    pub(crate) title: &'a str,
    /// A save dialog, offering this name; otherwise an open dialog.
    pub(crate) save_name: Option<&'a str>,
    /// An open dialog for a folder.
    pub(crate) directory: bool,
    /// Where the dialog starts.
    pub(crate) folder: Option<&'a Path>,
    /// One filter: its name, its media types and its file name endings.
    pub(crate) filter: Option<(&'a str, &'a [&'a str], &'a [&'a str])>,
}

/// The path of a folder as the portal takes it: its bytes ended by a zero
/// (`current_folder`, "a null-terminated byte string").
fn folder_bytes(folder: &Path) -> Result<Vec<u8>, String> {
    let mut bytes = folder.as_os_str().as_encoded_bytes().to_vec();
    if bytes.contains(&0) {
        return Err(String::from("a folder whose path holds a zero byte"));
    }
    bytes.push(0);
    Ok(bytes)
}

/// The dialog's options (`a{sv}`), as `OpenFile` and `SaveFile` take them.
fn options(dialog: &Dialog<'_>) -> Result<Value, String> {
    let mut options = vec![("handle_token", Value::String(HANDLE_TOKEN.to_owned()))];
    if let Some(name) = dialog.save_name {
        options.push(("current_name", Value::String(name.to_owned())));
    } else {
        options.push(("multiple", Value::Boolean(false)));
        options.push(("directory", Value::Boolean(dialog.directory)));
    }
    if let Some((name, media_types, endings)) = dialog.filter {
        let entry = dbus::Type::Struct(vec![dbus::Type::Uint32, dbus::Type::String]);
        let rules = media_types
            .iter()
            .map(|kind| Value::Struct(vec![Value::Uint32(FILTER_MEDIA_TYPE), Value::String((*kind).to_owned())]))
            .chain(endings.iter().map(|ending| Value::Struct(vec![Value::Uint32(FILTER_GLOB), Value::String(format!("*.{ending}"))])))
            .collect();
        let filter = Value::Struct(vec![Value::String(name.to_owned()), Value::Array(entry.clone(), rules)]);
        options.push(("filters", Value::Array(dbus::Type::Struct(vec![dbus::Type::String, dbus::Type::Array(Box::new(entry))]), vec![filter])));
    }
    if let Some(folder) = dialog.folder {
        options.push(("current_folder", Value::Bytes(folder_bytes(folder)?)));
    }
    Ok(Value::variant_dict(options))
}

/// The request object the portal will answer on, for the connection named
/// `unique_name`.
fn request_path(unique_name: &str) -> String {
    format!("{PORTAL_PATH}/request/{}/{HANDLE_TOKEN}", unique_name.trim_start_matches(':').replace('.', "_"))
}

fn response_rule(path: &str) -> String {
    format!("type='signal',sender='{PORTAL}',path='{path}',interface='{REQUEST}',member='Response'")
}

/// The chosen file's address, or none when the dialog was cancelled.
pub(crate) fn choose(dialog: &Dialog<'_>) -> Result<Option<String>, String> {
    let failed = |error: Error| error.to_string();
    let mut bus = Connection::session().map_err(failed)?;
    let owner = bus.name_owner(PORTAL).map_err(failed)?;
    // The rule is in place before the request is made, so the answer
    // cannot come before it (the `Request` documentation's advice).
    let expected = request_path(bus.unique_name());
    bus.add_match(&response_rule(&expected)).map_err(failed)?;
    let member = if dialog.save_name.is_some() { "SaveFile" } else { "OpenFile" };
    let call = Message::method_call(
        PORTAL,
        PORTAL_PATH,
        FILE_CHOOSER,
        member,
        vec![Value::String(dialog.parent.clone()), Value::String(dialog.title.to_owned()), options(dialog)?],
    );
    let handle = match bus.call(call).map_err(failed)?.as_slice() {
        [Value::ObjectPath(handle)] => handle.clone(),
        _ => return Err(format!("the portal's {member} did not answer a request")),
    };
    // A portal older than the token's rule answers on a path of its own.
    if handle != expected {
        bus.add_match(&response_rule(&handle)).map_err(failed)?;
    }
    let response = bus.wait_for_signal(&owner, &handle, REQUEST, "Response").map_err(failed)?;
    chosen(&response)
}

/// The first address of a `Response`'s results, by its code.
fn chosen(response: &[Value]) -> Result<Option<String>, String> {
    match response {
        [Value::Uint32(RESPONSE_CHOSEN), results] => {
            let uris = results.lookup("uris").and_then(Value::as_variant).and_then(Value::as_array).ok_or("the portal's answer has no addresses")?;
            Ok(uris.first().and_then(Value::as_str).map(str::to_owned))
        }
        [Value::Uint32(RESPONSE_CANCELLED), _] => Ok(None),
        [Value::Uint32(code), _] => Err(format!("the portal ended the dialog (response {code})")),
        _ => Err(String::from("the portal's answer is not a response")),
    }
}

/// The portal's settings, read one key at a time.
pub(crate) struct Settings {
    bus: Connection,
}

impl Settings {
    /// A connection to the session bus for the portal's settings.
    pub(crate) fn new() -> Result<Self, String> {
        Connection::session().map(|bus| Self { bus }).map_err(|error| error.to_string())
    }

    /// The value of `key` in `namespace`, by `Read`, which every version of
    /// the interface has: deprecated for `ReadOne` since version 2, and
    /// answering the value wrapped in one more variant there, which is
    /// unwrapped. A key the portal does not know is answered with an error,
    /// and is none here.
    pub(crate) fn read(&mut self, namespace: &str, key: &str) -> Option<Value> {
        let arguments = vec![Value::String(namespace.to_owned()), Value::String(key.to_owned())];
        let reply = self.bus.call(Message::method_call(PORTAL, PORTAL_PATH, SETTINGS, "Read", arguments)).ok()?;
        match <[Value; 1]>::try_from(reply) {
            Ok([Value::Variant(value)]) => Some(match *value {
                Value::Variant(inner) => *inner,
                value => value,
            }),
            _ => None,
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, reason = "a test")]
mod tests {
    use super::*;

    #[test]
    fn the_request_path_is_the_portals_rule() {
        assert_eq!(request_path(":1.234"), "/org/freedesktop/portal/desktop/request/1_234/neoteletext_chooser");
    }

    #[test]
    fn a_picture_dialog_asks_for_its_filter_and_folder() {
        let types = ["image/png"];
        let endings = ["png"];
        let dialog = Dialog {
            parent: String::new(),
            title: "Convert Picture to Teletext",
            save_name: None,
            directory: false,
            folder: Some(Path::new("/home/a")),
            filter: Some(("Pictures", &types, &endings)),
        };
        let options = options(&dialog).unwrap();
        assert_eq!(options.lookup("current_folder"), Some(&Value::Variant(Box::new(Value::Bytes(b"/home/a\0".to_vec())))));
        assert_eq!(options.lookup("directory"), Some(&Value::Variant(Box::new(Value::Boolean(false)))));
        let filters = options.lookup("filters").and_then(Value::as_variant).and_then(Value::as_array).unwrap();
        let rules = filters.first().and_then(Value::as_struct).and_then(|filter| filter.get(1)).and_then(Value::as_array).unwrap();
        assert_eq!(
            rules,
            [
                Value::Struct(vec![Value::Uint32(FILTER_MEDIA_TYPE), Value::String("image/png".into())]),
                Value::Struct(vec![Value::Uint32(FILTER_GLOB), Value::String("*.png".into())])
            ]
        );
        let save = Dialog { save_name: Some("picture.tti"), filter: None, folder: None, ..dialog };
        let options = super::options(&save).unwrap();
        assert_eq!(options.lookup("current_name"), Some(&Value::Variant(Box::new(Value::String("picture.tti".into())))));
        assert_eq!(options.lookup("multiple"), None);
    }

    #[test]
    fn a_folder_with_a_zero_byte_is_refused() {
        use std::os::unix::ffi::OsStrExt as _;
        assert!(folder_bytes(Path::new(std::ffi::OsStr::from_bytes(b"/a\0b"))).is_err());
    }

    #[test]
    fn a_response_gives_the_first_address_or_a_cancel() {
        let uris = |list: &[&str]| {
            Value::variant_dict(vec![("uris", Value::Array(dbus::Type::String, list.iter().map(|uri| Value::String((*uri).to_owned())).collect()))])
        };
        assert_eq!(chosen(&[Value::Uint32(RESPONSE_CHOSEN), uris(&["file:///a", "file:///b"])]), Ok(Some("file:///a".into())));
        assert_eq!(chosen(&[Value::Uint32(RESPONSE_CANCELLED), uris(&[])]), Ok(None));
        assert!(chosen(&[Value::Uint32(2), uris(&[])]).is_err());
        assert!(chosen(&[Value::Uint32(RESPONSE_CHOSEN), Value::variant_dict(Vec::new())]).is_err());
        assert!(chosen(&[Value::String("0".into())]).is_err());
    }
}
