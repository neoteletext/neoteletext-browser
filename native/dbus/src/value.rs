//! The D-Bus type system ("Type System" in the specification): the types
//! a signature names and the values a message carries.

use crate::Error;

/// D-Bus Specification, "Valid Signatures": a signature is at most 255
/// bytes.
pub const MAX_SIGNATURE_BYTES: usize = 255;
/// D-Bus Specification, "Valid Signatures": arrays nest at most 32 deep.
pub const MAX_ARRAY_DEPTH: usize = 32;
/// D-Bus Specification, "Valid Signatures": structures (and dictionary
/// entries) nest at most 32 deep.
pub const MAX_STRUCT_DEPTH: usize = 32;
/// D-Bus Specification, "Valid Signatures" and "Marshaling (Wire Format)",
/// variant: containers of every kind, variants among them, nest at most 64
/// deep in all, which also bounds the decoder's recursion.
pub const MAX_DEPTH: usize = 64;
/// The most nodes (each [`Value`], and each [`Type`] an array or a
/// dictionary keeps, as [`nodes`] counts them) a message may decode to, per
/// byte of the message: one. Without it a message's memory is not bounded
/// by its size: an array of empty arrays of a 250-field structure keeps the
/// structure's type in each of its 16,342 elements, and one 128 KiB message
/// of that shape decoded to 99 MB (the security review of 22 November 2026,
/// TODO-7173 finding A1). One a byte holds every message the browser reads:
/// a header field (`(yv)`, at least 8 bytes) is four nodes, a dictionary
/// entry of the portal's `a{sv}` three in at least 12 bytes. So a message
/// of at most [`crate::MAX_MESSAGE_BYTES`] holds at most that many nodes,
/// each at most `size_of::<Value>()` bytes and twice that in a growing
/// vector, plus its strings and byte arrays, which are no longer than the
/// message.
pub const MAX_DECODED_NODES_PER_BYTE: usize = 1;

/// A complete type, as one signature names it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Type {
    /// `y`.
    Byte,
    /// `b`.
    Boolean,
    /// `n`.
    Int16,
    /// `q`.
    Uint16,
    /// `i`.
    Int32,
    /// `u`.
    Uint32,
    /// `x`.
    Int64,
    /// `t`.
    Uint64,
    /// `d`.
    Double,
    /// `s`.
    String,
    /// `o`.
    ObjectPath,
    /// `g`.
    Signature,
    /// `h`: an index into the file descriptors passed with a message.
    UnixFd,
    /// `v`.
    Variant,
    /// `a` and its element's type.
    Array(Box<Type>),
    /// `a{..}`: an array of dictionary entries, the key's type and the
    /// value's.
    Dict(Box<Type>, Box<Type>),
    /// `(..)`, its fields' types.
    Struct(Vec<Type>),
}

impl Type {
    /// The type's signature, appended to `out`.
    pub fn write_signature(&self, out: &mut String) {
        match self {
            Self::Byte => out.push('y'),
            Self::Boolean => out.push('b'),
            Self::Int16 => out.push('n'),
            Self::Uint16 => out.push('q'),
            Self::Int32 => out.push('i'),
            Self::Uint32 => out.push('u'),
            Self::Int64 => out.push('x'),
            Self::Uint64 => out.push('t'),
            Self::Double => out.push('d'),
            Self::String => out.push('s'),
            Self::ObjectPath => out.push('o'),
            Self::Signature => out.push('g'),
            Self::UnixFd => out.push('h'),
            Self::Variant => out.push('v'),
            Self::Array(element) => {
                out.push('a');
                element.write_signature(out);
            }
            Self::Dict(key, value) => {
                out.push_str("a{");
                key.write_signature(out);
                value.write_signature(out);
                out.push('}');
            }
            Self::Struct(fields) => {
                out.push('(');
                for field in fields {
                    field.write_signature(out);
                }
                out.push(')');
            }
        }
    }

    /// The type's alignment on the wire ("Marshaling (Wire Format)", the
    /// table of alignments).
    #[must_use]
    pub fn alignment(&self) -> usize {
        match self {
            Self::Byte | Self::Signature | Self::Variant => 1,
            Self::Int16 | Self::Uint16 => 2,
            Self::Boolean | Self::Int32 | Self::Uint32 | Self::UnixFd | Self::String | Self::ObjectPath | Self::Array(_) | Self::Dict(..) => 4,
            Self::Int64 | Self::Uint64 | Self::Double | Self::Struct(_) => 8,
        }
    }

    /// The nodes of the type's tree: one for itself and one for each type
    /// it holds. A decoded array keeps its element's type, so the decoder
    /// counts these against its budget ([`MAX_DECODED_NODES_PER_BYTE`]).
    #[must_use]
    pub fn nodes(&self) -> usize {
        let held = match self {
            Self::Array(element) => element.nodes(),
            Self::Dict(key, value) => key.nodes().saturating_add(value.nodes()),
            Self::Struct(fields) => fields.iter().map(Self::nodes).fold(0, usize::saturating_add),
            _ => 0,
        };
        held.saturating_add(1)
    }

    /// Whether this is a basic type, the only kind a dictionary's key may
    /// be ("Type System", dictionary entries).
    #[must_use]
    pub fn is_basic(&self) -> bool {
        !matches!(self, Self::Variant | Self::Array(_) | Self::Dict(..) | Self::Struct(_))
    }
}

/// The signature of `types`, one after another, as a message's body has it.
#[must_use]
pub fn signature_of(types: &[Type]) -> String {
    let mut out = String::new();
    for kind in types {
        kind.write_signature(&mut out);
    }
    out
}

/// A value of one complete type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Value {
    /// `y`.
    Byte(u8),
    /// `b`.
    Boolean(bool),
    /// `n`.
    Int16(i16),
    /// `q`.
    Uint16(u16),
    /// `i`.
    Int32(i32),
    /// `u`.
    Uint32(u32),
    /// `x`.
    Int64(i64),
    /// `t`.
    Uint64(u64),
    /// `d`, kept as its IEEE 754 bits: nothing here does arithmetic on one.
    Double(u64),
    /// `s`.
    String(String),
    /// `o`, a valid object path.
    ObjectPath(String),
    /// `g`, a valid signature.
    Signature(String),
    /// `h`.
    UnixFd(u32),
    /// `ay`, kept as bytes: a secret, a path and a portal's folder are
    /// byte arrays, and [`wipe`] clears them.
    Bytes(Vec<u8>),
    /// `a` of any other element type: the element's type and the elements.
    Array(Type, Vec<Value>),
    /// `a{..}`: the key's type, the value's, and the entries.
    Dict(Type, Type, Vec<(Value, Value)>),
    /// `(..)`.
    Struct(Vec<Value>),
    /// `v`.
    Variant(Box<Value>),
}

impl Value {
    /// The value's type.
    #[must_use]
    pub fn kind(&self) -> Type {
        match self {
            Self::Byte(_) => Type::Byte,
            Self::Boolean(_) => Type::Boolean,
            Self::Int16(_) => Type::Int16,
            Self::Uint16(_) => Type::Uint16,
            Self::Int32(_) => Type::Int32,
            Self::Uint32(_) => Type::Uint32,
            Self::Int64(_) => Type::Int64,
            Self::Uint64(_) => Type::Uint64,
            Self::Double(_) => Type::Double,
            Self::String(_) => Type::String,
            Self::ObjectPath(_) => Type::ObjectPath,
            Self::Signature(_) => Type::Signature,
            Self::UnixFd(_) => Type::UnixFd,
            Self::Bytes(_) => Type::Array(Box::new(Type::Byte)),
            Self::Array(element, _) => Type::Array(Box::new(element.clone())),
            Self::Dict(key, value, _) => Type::Dict(Box::new(key.clone()), Box::new(value.clone())),
            Self::Struct(fields) => Type::Struct(fields.iter().map(Self::kind).collect()),
            Self::Variant(_) => Type::Variant,
        }
    }

    /// A string, as text.
    #[must_use]
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::String(text) => Some(text),
            _ => None,
        }
    }

    /// An object path, as text.
    #[must_use]
    pub fn as_object_path(&self) -> Option<&str> {
        match self {
            Self::ObjectPath(path) => Some(path),
            _ => None,
        }
    }

    /// A `u`.
    #[must_use]
    pub fn as_u32(&self) -> Option<u32> {
        match self {
            Self::Uint32(number) => Some(*number),
            _ => None,
        }
    }

    /// A `b`.
    #[must_use]
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Self::Boolean(value) => Some(*value),
            _ => None,
        }
    }

    /// An `ay`.
    #[must_use]
    pub fn as_bytes(&self) -> Option<&[u8]> {
        match self {
            Self::Bytes(bytes) => Some(bytes),
            _ => None,
        }
    }

    /// The elements of an array of any type but bytes.
    #[must_use]
    pub fn as_array(&self) -> Option<&[Value]> {
        match self {
            Self::Array(_, elements) => Some(elements),
            _ => None,
        }
    }

    /// The fields of a structure.
    #[must_use]
    pub fn as_struct(&self) -> Option<&[Value]> {
        match self {
            Self::Struct(fields) => Some(fields),
            _ => None,
        }
    }

    /// What a variant holds.
    #[must_use]
    pub fn as_variant(&self) -> Option<&Value> {
        match self {
            Self::Variant(inner) => Some(inner),
            _ => None,
        }
    }

    /// The value of a dictionary with string keys (`a{s..}`) under `key`.
    #[must_use]
    pub fn lookup(&self, key: &str) -> Option<&Value> {
        match self {
            Self::Dict(_, _, entries) => entries.iter().find(|(name, _)| name.as_str() == Some(key)).map(|(_, value)| value),
            _ => None,
        }
    }

    /// An `a{ss}` of `pairs`.
    #[must_use]
    pub fn string_dict(pairs: &[(&str, &str)]) -> Self {
        Self::Dict(Type::String, Type::String, pairs.iter().map(|(key, value)| (Self::String((*key).to_owned()), Self::String((*value).to_owned()))).collect())
    }

    /// An `a{sv}` of `pairs`, each value wrapped in its variant.
    #[must_use]
    pub fn variant_dict(pairs: Vec<(&str, Value)>) -> Self {
        Self::Dict(Type::String, Type::Variant, pairs.into_iter().map(|(key, value)| (Self::String(key.to_owned()), Self::Variant(Box::new(value)))).collect())
    }
}

/// The nodes `values` hold: each value, each value inside it, and each type
/// an array or a dictionary keeps. A decoded message holds at most
/// [`MAX_DECODED_NODES_PER_BYTE`] for each of its bytes.
#[must_use]
pub fn nodes(values: &[Value]) -> usize {
    values
        .iter()
        .map(|value| {
            let held = match value {
                Value::Array(element, elements) => element.nodes().saturating_add(nodes(elements)),
                Value::Dict(key, entry, entries) => entries
                    .iter()
                    .map(|(name, held)| nodes(std::slice::from_ref(name)).saturating_add(nodes(std::slice::from_ref(held))))
                    .fold(key.nodes().saturating_add(entry.nodes()), usize::saturating_add),
                Value::Struct(fields) => nodes(fields),
                Value::Variant(inner) => nodes(std::slice::from_ref(inner.as_ref())),
                _ => 0,
            };
            held.saturating_add(1)
        })
        .fold(0, usize::saturating_add)
}

/// Clears every byte array in `values`, to their last byte, before they
/// are dropped: a secret read from or written to the bus stays in no
/// buffer of this process.
pub fn wipe(values: &mut [Value]) {
    for value in values {
        match value {
            Value::Bytes(bytes) => wipe_bytes(bytes),
            Value::Array(_, elements) | Value::Struct(elements) => wipe(elements),
            Value::Dict(_, _, entries) => {
                for (key, entry) in entries {
                    wipe(std::slice::from_mut(key));
                    wipe(std::slice::from_mut(entry));
                }
            }
            Value::Variant(inner) => wipe(std::slice::from_mut(inner.as_mut())),
            _ => {}
        }
    }
}

/// Sets `bytes` to zero in a way the optimiser keeps: `black_box` makes the
/// zeroed buffer observed, so the stores are not removed as dead before the
/// buffer is freed.
pub(crate) fn wipe_bytes(bytes: &mut [u8]) {
    bytes.fill(0);
    std::hint::black_box(&*bytes);
}

/// A signature's complete types ("Valid Signatures"): at most
/// [`MAX_SIGNATURE_BYTES`], arrays and structures nested within their
/// bounds, dictionary entries only as an array's element and keyed by a
/// basic type, and no empty structure.
///
/// # Errors
///
/// [`Error::Malformed`], naming the rule `signature` breaks.
pub fn parse_signature(signature: &str) -> Result<Vec<Type>, Error> {
    if signature.len() > MAX_SIGNATURE_BYTES {
        return Err(Error::Malformed("a signature longer than 255 bytes"));
    }
    let mut parser = SignatureParser { bytes: signature.as_bytes(), position: 0 };
    let mut types = Vec::new();
    while parser.position < parser.bytes.len() {
        types.push(parser.complete(0, 0)?);
    }
    Ok(types)
}

/// Exactly one complete type.
pub(crate) fn parse_single(signature: &str) -> Result<Type, Error> {
    let mut types = parse_signature(signature)?;
    match (types.pop(), types.is_empty()) {
        (Some(kind), true) => Ok(kind),
        _ => Err(Error::Malformed("a variant's signature is not one complete type")),
    }
}

struct SignatureParser<'a> {
    bytes: &'a [u8],
    position: usize,
}

impl SignatureParser<'_> {
    fn next(&mut self) -> Result<u8, Error> {
        let byte = *self.bytes.get(self.position).ok_or(Error::Malformed("a signature ends inside a type"))?;
        self.position = self.position.checked_add(1).ok_or(Error::Malformed("a signature too long"))?;
        Ok(byte)
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.position).copied()
    }

    fn complete(&mut self, arrays: usize, structs: usize) -> Result<Type, Error> {
        if arrays.checked_add(structs).is_none_or(|depth| depth > MAX_DEPTH) {
            return Err(Error::Malformed("a signature nests containers too deep"));
        }
        Ok(match self.next()? {
            b'y' => Type::Byte,
            b'b' => Type::Boolean,
            b'n' => Type::Int16,
            b'q' => Type::Uint16,
            b'i' => Type::Int32,
            b'u' => Type::Uint32,
            b'x' => Type::Int64,
            b't' => Type::Uint64,
            b'd' => Type::Double,
            b's' => Type::String,
            b'o' => Type::ObjectPath,
            b'g' => Type::Signature,
            b'h' => Type::UnixFd,
            b'v' => Type::Variant,
            b'a' => {
                let arrays = arrays.checked_add(1).filter(|&depth| depth <= MAX_ARRAY_DEPTH).ok_or(Error::Malformed("arrays nested more than 32 deep"))?;
                if self.peek() == Some(b'{') {
                    self.position = self.position.checked_add(1).ok_or(Error::Malformed("a signature too long"))?;
                    let structs =
                        structs.checked_add(1).filter(|&depth| depth <= MAX_STRUCT_DEPTH).ok_or(Error::Malformed("structures nested more than 32 deep"))?;
                    let key = self.complete(arrays, structs)?;
                    if !key.is_basic() {
                        return Err(Error::Malformed("a dictionary's key is not a basic type"));
                    }
                    let value = self.complete(arrays, structs)?;
                    if self.next()? != b'}' {
                        return Err(Error::Malformed("a dictionary entry holds other than one key and one value"));
                    }
                    Type::Dict(Box::new(key), Box::new(value))
                } else {
                    Type::Array(Box::new(self.complete(arrays, structs)?))
                }
            }
            b'(' => {
                let structs =
                    structs.checked_add(1).filter(|&depth| depth <= MAX_STRUCT_DEPTH).ok_or(Error::Malformed("structures nested more than 32 deep"))?;
                let mut fields = Vec::new();
                while self.peek() != Some(b')') {
                    fields.push(self.complete(arrays, structs)?);
                }
                self.position = self.position.checked_add(1).ok_or(Error::Malformed("a signature too long"))?;
                if fields.is_empty() {
                    return Err(Error::Malformed("an empty structure"));
                }
                Type::Struct(fields)
            }
            _ => return Err(Error::Malformed("a signature names an unknown type, or a dictionary entry outside an array")),
        })
    }
}

/// Whether `path` is a valid object path ("Valid Object Paths"): `/`, or
/// `/` and elements of ASCII letters, digits and underscores, each not
/// empty, separated by `/`, with none at the end.
#[must_use]
pub fn valid_object_path(path: &str) -> bool {
    if path == "/" {
        return true;
    }
    let Some(rest) = path.strip_prefix('/') else { return false };
    rest.split('/').all(|element| !element.is_empty() && element.bytes().all(|byte| byte.is_ascii_alphanumeric() || byte == b'_'))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, reason = "a test")]
mod tests {
    use super::*;

    #[test]
    fn signatures_read_and_write_back() {
        for signature in ["", "y", "s", "a{sv}", "(oayays)", "aa{ss}", "ua{sv}", "a(sa(us))", "ao", "v", "a{oa{sv}}"] {
            assert_eq!(signature_of(&parse_signature(signature).unwrap()), signature, "{signature}");
        }
        assert_eq!(parse_signature("a{sv}").unwrap(), vec![Type::Dict(Box::new(Type::String), Box::new(Type::Variant))]);
    }

    #[test]
    fn invalid_signatures_are_refused() {
        let deep_arrays = "a".repeat(MAX_ARRAY_DEPTH + 1) + "y";
        let deep_structs = "(".repeat(MAX_STRUCT_DEPTH + 1) + "y" + &")".repeat(MAX_STRUCT_DEPTH + 1);
        let long = "y".repeat(MAX_SIGNATURE_BYTES + 1);
        for signature in ["a", "()", "(y", "{sv}", "a{vs}", "a{s}", "a{svs}", "z", "y)", deep_arrays.as_str(), deep_structs.as_str(), long.as_str()] {
            assert!(parse_signature(signature).is_err(), "{signature}");
        }
        assert!(parse_signature(&("a".repeat(MAX_ARRAY_DEPTH) + "y")).is_ok());
        assert!(parse_signature(&"y".repeat(MAX_SIGNATURE_BYTES)).is_ok());
    }

    #[test]
    fn object_paths_follow_the_specification() {
        for valid in ["/", "/org/freedesktop/portal/desktop", "/a_b/C9"] {
            assert!(valid_object_path(valid), "{valid}");
        }
        for invalid in ["", "org", "/org/", "//", "/a//b", "/a-b", "/a.b", "/é"] {
            assert!(!valid_object_path(invalid), "{invalid}");
        }
    }

    #[test]
    fn wiping_clears_every_byte_array() {
        let mut values = vec![Value::Struct(vec![Value::ObjectPath("/".into()), Value::Bytes(vec![1, 2]), Value::Variant(Box::new(Value::Bytes(vec![3])))])];
        wipe(&mut values);
        assert_eq!(values, vec![Value::Struct(vec![Value::ObjectPath("/".into()), Value::Bytes(vec![0, 0]), Value::Variant(Box::new(Value::Bytes(vec![0])))])]);
    }
}
