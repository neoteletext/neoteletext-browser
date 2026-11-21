//! The freedesktop Secret Service (the Secret Service API Draft, version
//! 0.2: `org.freedesktop.Secret.Service`, `.Collection`, `.Item` and
//! `.Prompt`), over the browser's own D-Bus client.
//!
//! An entry is the one item of the default collection whose attributes
//! `service` and `username` are the service and account, as `keyring`
//! 4.2.0's Secret Service store made it, labelled as it labelled it.
//!
//! The session is the specification's `plain` algorithm ("Transfer of
//! Secrets"): the secret crosses the session bus unencrypted, which only
//! the user's own processes can read and which those can already ask the
//! service for the item itself (`docs/security-analysis.md` section 5.11).

use dbus::{Connection, Error as BusError, Message, Type, Value};

use crate::{Error, Secret};

const SERVICE: &str = "org.freedesktop.secrets";
const SERVICE_PATH: &str = "/org/freedesktop/secrets";
const SERVICE_INTERFACE: &str = "org.freedesktop.Secret.Service";
const COLLECTION_INTERFACE: &str = "org.freedesktop.Secret.Collection";
const ITEM_INTERFACE: &str = "org.freedesktop.Secret.Item";
const PROMPT_INTERFACE: &str = "org.freedesktop.Secret.Prompt";
/// "Transfer of Secrets": the algorithm that sends secrets as they are,
/// with an empty input.
const PLAIN: &str = "plain";
/// "Collection Aliases": the collection new items go to.
const DEFAULT_ALIAS: &str = "default";
/// The object path the service answers for "no prompt needed" and "no
/// collection" ("Prompts and Prompting", `ReadAlias`).
const NONE: &str = "/";
/// What `keyring` stored the secret as.
const CONTENT_TYPE: &str = "application/octet-stream";
/// `CreateItem`'s properties.
const LABEL_PROPERTY: &str = "org.freedesktop.Secret.Item.Label";
const ATTRIBUTES_PROPERTY: &str = "org.freedesktop.Secret.Item.Attributes";
/// The attributes that name an entry, as `keyring` named them.
const SERVICE_ATTRIBUTE: &str = "service";
const ACCOUNT_ATTRIBUTE: &str = "username";
/// `Prompt`'s window: none, so the service places its prompt itself.
const NO_WINDOW: &str = "";

/// What a bus failure is to the caller: no store when the service cannot
/// be reached at all, a refusal otherwise.
fn refused(error: BusError) -> Error {
    let text = error.to_string();
    match error {
        BusError::NoBus(_) | BusError::Closed => Error::NoStore(text),
        BusError::Remote { name, .. } if name == "org.freedesktop.DBus.Error.ServiceUnknown" || name == "org.freedesktop.DBus.Error.NameHasNoOwner" => {
            Error::NoStore(text)
        }
        _ => Error::Refused(text),
    }
}

fn unexpected(what: &'static str) -> Error {
    Error::Refused(format!("the Secret Service answered {what} unexpectedly"))
}

/// The label `keyring` gave an item it made.
fn label(service: &str, account: &str) -> String {
    format!("keyring:{account}@{service}")
}

fn attributes(service: &str, account: &str) -> Value {
    Value::string_dict(&[(SERVICE_ATTRIBUTE, service), (ACCOUNT_ATTRIBUTE, account)])
}

/// The object paths of an `ao`.
fn paths(value: &Value) -> Option<Vec<String>> {
    match value {
        Value::Array(Type::ObjectPath, paths) => paths.iter().map(|path| path.as_object_path().map(str::to_owned)).collect(),
        _ => None,
    }
}

/// `SearchItems`' answer: the unlocked items, then the locked.
fn search_reply(reply: &[Value]) -> Result<(Vec<String>, Vec<String>), Error> {
    match reply {
        [unlocked, locked] => Ok((paths(unlocked).ok_or_else(|| unexpected("a search"))?, paths(locked).ok_or_else(|| unexpected("a search"))?)),
        _ => Err(unexpected("a search")),
    }
}

/// A secret structure's value (`(oayays)`: the session, the parameters,
/// the value, the content type), copied into a [`Secret`].
fn secret_reply(reply: &[Value]) -> Result<Secret, Error> {
    match reply {
        [Value::Struct(fields)] => match fields.as_slice() {
            [Value::ObjectPath(_), Value::Bytes(_), Value::Bytes(value), Value::String(_)] => Secret::copied(value),
            _ => Err(unexpected("a secret")),
        },
        _ => Err(unexpected("a secret")),
    }
}

/// One path and a prompt's path, as `Unlock`, `CreateItem` and `Delete`
/// answer.
fn prompt_of(reply: &[Value]) -> Result<String, Error> {
    match reply {
        [_, Value::ObjectPath(prompt)] | [Value::ObjectPath(prompt)] => Ok(prompt.clone()),
        _ => Err(unexpected("a prompt")),
    }
}

/// An open session with the Secret Service.
struct Session {
    bus: Connection,
    /// The unique name that owns the service, from which its prompts'
    /// signals are taken.
    owner: String,
    /// The session's object.
    path: String,
}

impl Session {
    fn open() -> Result<Self, Error> {
        let mut bus = Connection::session().map_err(|error| Error::NoStore(error.to_string()))?;
        // Called first, so that the bus starts the service if it is
        // activatable and not running.
        let reply = bus
            .call(Message::method_call(
                SERVICE,
                SERVICE_PATH,
                SERVICE_INTERFACE,
                "OpenSession",
                vec![Value::String(PLAIN.to_owned()), Value::Variant(Box::new(Value::String(String::new())))],
            ))
            .map_err(refused)?;
        let session = match reply.as_slice() {
            [Value::Variant(_), Value::ObjectPath(session)] => session.clone(),
            _ => return Err(unexpected("the session")),
        };
        let owner = bus.name_owner(SERVICE).map_err(refused)?;
        Ok(Self { bus, owner, path: session })
    }

    fn call(&mut self, path: &str, interface: &str, member: &str, body: Vec<Value>) -> Result<Vec<Value>, Error> {
        self.bus.call(Message::method_call(SERVICE, path, interface, member, body)).map_err(refused)
    }

    /// Runs the prompt at `prompt`, unless it is none, and waits for the
    /// person to answer it ("Prompts and Prompting"): the signal is asked
    /// for before the prompt is shown, and taken only from the service.
    fn prompt(&mut self, prompt: &str) -> Result<(), Error> {
        if prompt == NONE {
            return Ok(());
        }
        let rule = format!("type='signal',sender='{SERVICE}',path='{prompt}',interface='{PROMPT_INTERFACE}',member='Completed'");
        self.bus.add_match(&rule).map_err(refused)?;
        self.call(prompt, PROMPT_INTERFACE, "Prompt", vec![Value::String(NO_WINDOW.to_owned())])?;
        let owner = self.owner.clone();
        match self.bus.wait_for_signal(&owner, prompt, PROMPT_INTERFACE, "Completed").map_err(refused)?.as_slice() {
            [Value::Boolean(false), _] => Ok(()),
            [Value::Boolean(true), _] => Err(Error::Refused(String::from("the prompt was dismissed"))),
            _ => Err(unexpected("a prompt")),
        }
    }

    /// Unlocks `objects`, prompting if the service asks to.
    fn unlock(&mut self, objects: Vec<String>) -> Result<(), Error> {
        let reply =
            self.call(SERVICE_PATH, SERVICE_INTERFACE, "Unlock", vec![Value::Array(Type::ObjectPath, objects.into_iter().map(Value::ObjectPath).collect())])?;
        let prompt = prompt_of(&reply)?;
        self.prompt(&prompt)
    }

    /// The items of `service` and `account`, unlocked, as `keyring` found
    /// them: the locked ones are unlocked first.
    fn items(&mut self, service: &str, account: &str) -> Result<Vec<String>, Error> {
        let reply = self.call(SERVICE_PATH, SERVICE_INTERFACE, "SearchItems", vec![attributes(service, account)])?;
        let (mut unlocked, locked) = search_reply(&reply)?;
        if !locked.is_empty() {
            self.unlock(locked.clone())?;
            unlocked.extend(locked);
        }
        Ok(unlocked)
    }

    /// The one item of `service` and `account`, if there is one.
    fn item(&mut self, service: &str, account: &str) -> Result<Option<String>, Error> {
        let mut items = self.items(service, account)?;
        match (items.pop(), items.is_empty()) {
            (None, _) => Ok(None),
            (Some(item), true) => Ok(Some(item)),
            (Some(_), false) => Err(Error::Ambiguous),
        }
    }

    /// A secret structure of `secret` for this session.
    fn secret(&self, secret: &[u8]) -> Value {
        Value::Struct(vec![
            Value::ObjectPath(self.path.clone()),
            Value::Bytes(Vec::new()),
            Value::Bytes(secret.to_vec()),
            Value::String(CONTENT_TYPE.to_owned()),
        ])
    }
}

pub(crate) fn read(service: &str, account: &str) -> Result<Option<Secret>, Error> {
    let mut session = Session::open()?;
    let Some(item) = session.item(service, account)? else { return Ok(None) };
    let session_path = session.path.clone();
    let mut reply = session.call(&item, ITEM_INTERFACE, "GetSecret", vec![Value::ObjectPath(session_path)])?;
    let secret = secret_reply(&reply);
    dbus::wipe(&mut reply);
    secret.map(Some)
}

pub(crate) fn write(service: &str, account: &str, secret: &[u8]) -> Result<(), Error> {
    let mut session = Session::open()?;
    // Each secret structure is made just before the call that sends it:
    // the connection clears a message's byte arrays once it is sent, so no
    // copy outlives its call.
    if let Some(item) = session.item(service, account)? {
        let value = session.secret(secret);
        session.call(&item, ITEM_INTERFACE, "SetSecret", vec![value])?;
        return Ok(());
    }
    let collection = match session.call(SERVICE_PATH, SERVICE_INTERFACE, "ReadAlias", vec![Value::String(DEFAULT_ALIAS.to_owned())])?.as_slice() {
        [Value::ObjectPath(collection)] if collection != NONE => collection.clone(),
        [Value::ObjectPath(_)] => return Err(Error::NoStore(String::from("the Secret Service has no default collection"))),
        _ => return Err(unexpected("the default collection")),
    };
    session.unlock(vec![collection.clone()])?;
    let properties = Value::variant_dict(vec![(LABEL_PROPERTY, Value::String(label(service, account))), (ATTRIBUTES_PROPERTY, attributes(service, account))]);
    let replace = Value::Boolean(true);
    let value = session.secret(secret);
    let reply = session.call(&collection, COLLECTION_INTERFACE, "CreateItem", vec![properties, value, replace])?;
    let prompt = prompt_of(&reply)?;
    session.prompt(&prompt)
}

pub(crate) fn delete(service: &str, account: &str) -> Result<(), Error> {
    let mut session = Session::open()?;
    for item in session.items(service, account)? {
        let reply = session.call(&item, ITEM_INTERFACE, "Delete", Vec::new())?;
        let prompt = prompt_of(&reply)?;
        session.prompt(&prompt)?;
    }
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, reason = "a test")]
mod tests {
    use super::*;

    fn object_paths(list: &[&str]) -> Value {
        Value::Array(Type::ObjectPath, list.iter().map(|path| Value::ObjectPath((*path).to_owned())).collect())
    }

    /// The names `keyring` 4.2.0 gave an entry (its Secret Service store's
    /// `search_attributes` and default label), so a key it stored is found.
    #[test]
    fn an_entry_is_named_as_keyring_named_it() {
        assert_eq!(label("NeoTeletext viewer keys", "viewer-keys.bin"), "keyring:viewer-keys.bin@NeoTeletext viewer keys");
        assert_eq!(attributes("NeoTeletext viewer keys", "viewer-keys.bin").lookup("username").and_then(Value::as_str), Some("viewer-keys.bin"));
        assert_eq!(attributes("NeoTeletext viewer keys", "viewer-keys.bin").lookup("service").and_then(Value::as_str), Some("NeoTeletext viewer keys"));
    }

    #[test]
    fn replies_are_read_strictly() {
        let item = "/org/freedesktop/secrets/collection/login/1";
        assert_eq!(search_reply(&[object_paths(&[item]), object_paths(&[])]).unwrap(), (vec![item.to_owned()], Vec::new()));
        assert!(search_reply(&[object_paths(&[item])]).is_err());
        assert!(search_reply(&[Value::Array(Type::String, Vec::new()), object_paths(&[])]).is_err());
        let secret = |value: Vec<u8>| {
            Value::Struct(vec![Value::ObjectPath("/s".into()), Value::Bytes(Vec::new()), Value::Bytes(value), Value::String(CONTENT_TYPE.into())])
        };
        assert_eq!(secret_reply(&[secret(vec![5; 32])]).unwrap().as_bytes(), [5; 32]);
        assert_eq!(secret_reply(&[secret(vec![5; crate::MAX_SECRET_BYTES + 1])]).err(), Some(Error::TooLong));
        assert!(secret_reply(&[Value::Struct(vec![Value::Bytes(vec![5])])]).is_err());
        assert_eq!(prompt_of(&[object_paths(&[]), Value::ObjectPath("/".into())]).unwrap(), "/");
        assert_eq!(prompt_of(&[Value::ObjectPath("/p/1".into())]).unwrap(), "/p/1");
        assert!(prompt_of(&[Value::String("/".into())]).is_err());
    }

    #[test]
    fn a_missing_service_is_no_store_and_anything_else_a_refusal() {
        let remote = |name: &str| BusError::Remote { name: name.into(), message: String::new() };
        assert!(matches!(refused(remote("org.freedesktop.DBus.Error.ServiceUnknown")), Error::NoStore(_)));
        assert!(matches!(refused(BusError::NoBus("x".into())), Error::NoStore(_)));
        assert!(matches!(refused(remote("org.freedesktop.Secret.Error.IsLocked")), Error::Refused(_)));
    }
}
