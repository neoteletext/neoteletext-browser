//! The wire format ("Marshaling (Wire Format)" in the specification): how
//! each type is written and read, aligned to its own size from the start of
//! the message.

use crate::Error;
use crate::value::{MAX_DEPTH, MAX_SIGNATURE_BYTES, Type, Value, parse_signature, parse_single, signature_of, valid_object_path, wipe, wipe_bytes};

/// "Marshaling (Wire Format)", arrays: an array's length is at most 2^26
/// bytes (64 MiB). Every message here is far shorter
/// ([`crate::MAX_MESSAGE_BYTES`]); this is the specification's own rule.
const MAX_ARRAY_BYTES: usize = 1 << 26;
/// The bytes of a `u32` on the wire.
const U32_BYTES: usize = 4;

fn too_long() -> Error {
    Error::Malformed("a length overflows")
}

/// Bytes being written, little-endian (the `l` of the message header).
/// What was written is wiped when the writer is dropped, so that a secret
/// in a message whose encoding fails part way stays in no freed buffer
/// (TODO-7173 finding A4); [`Writer::into_bytes`] hands the bytes on.
#[derive(Default)]
pub(crate) struct Writer {
    pub(crate) bytes: Vec<u8>,
}

impl Drop for Writer {
    fn drop(&mut self) {
        self.wipe();
    }
}

impl Writer {
    /// Zero bytes up to the next multiple of `alignment` from the start.
    pub(crate) fn pad(&mut self, alignment: usize) -> Result<(), Error> {
        let target = self.bytes.len().checked_next_multiple_of(alignment).ok_or_else(too_long)?;
        self.bytes.resize(target, 0);
        Ok(())
    }

    pub(crate) fn u8(&mut self, byte: u8) {
        self.bytes.push(byte);
    }

    pub(crate) fn u32(&mut self, number: u32) -> Result<(), Error> {
        self.pad(U32_BYTES)?;
        self.bytes.extend_from_slice(&number.to_le_bytes());
        Ok(())
    }

    fn sized(&mut self, alignment: usize, bytes: &[u8]) -> Result<(), Error> {
        self.pad(alignment)?;
        self.bytes.extend_from_slice(bytes);
        Ok(())
    }

    fn string(&mut self, text: &str) -> Result<(), Error> {
        if text.contains('\0') {
            return Err(Error::Malformed("a string holds a zero byte"));
        }
        self.u32(u32::try_from(text.len()).map_err(|_| too_long())?)?;
        self.bytes.extend_from_slice(text.as_bytes());
        self.bytes.push(0);
        Ok(())
    }

    fn signature(&mut self, signature: &str) -> Result<(), Error> {
        parse_signature(signature)?;
        self.u8(u8::try_from(signature.len()).map_err(|_| Error::Malformed("a signature longer than 255 bytes"))?);
        self.bytes.extend_from_slice(signature.as_bytes());
        self.bytes.push(0);
        Ok(())
    }

    /// The start of an array: its length's place and the padding to its
    /// first element. The length is filled in by [`Writer::end_array`].
    fn start_array(&mut self, element: &Type) -> Result<(usize, usize), Error> {
        self.u32(0)?;
        let length_at = self.bytes.len().checked_sub(U32_BYTES).ok_or_else(too_long)?;
        self.pad(element.alignment())?;
        Ok((length_at, self.bytes.len()))
    }

    fn end_array(&mut self, (length_at, start): (usize, usize)) -> Result<(), Error> {
        let length = self.bytes.len().checked_sub(start).ok_or_else(too_long)?;
        if length > MAX_ARRAY_BYTES {
            return Err(Error::Malformed("an array longer than 64 MiB"));
        }
        let length = u32::try_from(length).map_err(|_| too_long())?.to_le_bytes();
        self.bytes.get_mut(length_at..length_at.checked_add(U32_BYTES).ok_or_else(too_long)?).ok_or_else(too_long)?.copy_from_slice(&length);
        Ok(())
    }

    /// `value`, `depth` containers deep.
    pub(crate) fn value(&mut self, value: &Value, depth: usize) -> Result<(), Error> {
        if depth > MAX_DEPTH {
            return Err(Error::Malformed("containers nested more than 64 deep"));
        }
        let inner = depth.saturating_add(1);
        match value {
            Value::Byte(byte) => self.u8(*byte),
            Value::Boolean(value) => self.u32(u32::from(*value))?,
            Value::Int16(number) => self.sized(Type::Int16.alignment(), &number.to_le_bytes())?,
            Value::Uint16(number) => self.sized(Type::Uint16.alignment(), &number.to_le_bytes())?,
            Value::Int32(number) => self.sized(Type::Int32.alignment(), &number.to_le_bytes())?,
            Value::Uint32(number) | Value::UnixFd(number) => self.u32(*number)?,
            Value::Int64(number) => self.sized(Type::Int64.alignment(), &number.to_le_bytes())?,
            Value::Uint64(number) | Value::Double(number) => self.sized(Type::Uint64.alignment(), &number.to_le_bytes())?,
            Value::String(text) => self.string(text)?,
            Value::ObjectPath(path) => {
                if !valid_object_path(path) {
                    return Err(Error::Malformed("an invalid object path"));
                }
                self.string(path)?;
            }
            Value::Signature(signature) => self.signature(signature)?,
            Value::Bytes(bytes) => {
                let array = self.start_array(&Type::Byte)?;
                self.bytes.extend_from_slice(bytes);
                self.end_array(array)?;
            }
            Value::Array(element, elements) => {
                let array = self.start_array(element)?;
                for item in elements {
                    if item.kind() != *element {
                        return Err(Error::Malformed("an array's element of another type"));
                    }
                    self.value(item, inner)?;
                }
                self.end_array(array)?;
            }
            Value::Dict(key_type, value_type, entries) => {
                if !key_type.is_basic() {
                    return Err(Error::Malformed("a dictionary's key is not a basic type"));
                }
                let array = self.start_array(&Type::Struct(Vec::new()))?;
                for (key, entry) in entries {
                    if key.kind() != *key_type || entry.kind() != *value_type {
                        return Err(Error::Malformed("a dictionary's entry of another type"));
                    }
                    self.pad(Type::Struct(Vec::new()).alignment())?;
                    self.value(key, inner)?;
                    self.value(entry, inner)?;
                }
                self.end_array(array)?;
            }
            Value::Struct(fields) => {
                if fields.is_empty() {
                    return Err(Error::Malformed("an empty structure"));
                }
                self.pad(Type::Struct(Vec::new()).alignment())?;
                for field in fields {
                    self.value(field, inner)?;
                }
            }
            Value::Variant(held) => {
                let signature = signature_of(&[held.kind()]);
                if signature.len() > MAX_SIGNATURE_BYTES {
                    return Err(Error::Malformed("a variant's signature longer than 255 bytes"));
                }
                self.signature(&signature)?;
                self.value(held, inner)?;
            }
        }
        Ok(())
    }

    /// Clears what was written, as [`crate::wipe`] clears values.
    pub(crate) fn wipe(&mut self) {
        wipe_bytes(&mut self.bytes);
    }

    /// What was written, for the caller to send and wipe.
    pub(crate) fn into_bytes(mut self) -> Vec<u8> {
        std::mem::take(&mut self.bytes)
    }
}

/// Bytes being read: the whole message, so that alignment is counted from
/// its start, in the byte order its header names.
pub(crate) struct Reader<'a> {
    pub(crate) bytes: &'a [u8],
    pub(crate) position: usize,
    pub(crate) big_endian: bool,
    /// The nodes still allowed to be decoded (each value, and each type an
    /// array or a dictionary keeps; [`crate::value::nodes`]), so that what a
    /// message decodes to is bounded by its size
    /// ([`crate::value::MAX_DECODED_NODES_PER_BYTE`]).
    pub(crate) budget: usize,
}

impl<'a> Reader<'a> {
    /// Spends `count` nodes of the budget, or refuses the message.
    fn charge(&mut self, count: usize) -> Result<(), Error> {
        self.budget = self.budget.checked_sub(count).ok_or(Error::Malformed("a message that decodes to more values than it has bytes"))?;
        Ok(())
    }

    fn take(&mut self, count: usize) -> Result<&'a [u8], Error> {
        let end = self.position.checked_add(count).ok_or_else(too_long)?;
        let taken = self.bytes.get(self.position..end).ok_or(Error::Malformed("a message ends inside a value"))?;
        self.position = end;
        Ok(taken)
    }

    /// Past the padding to the next multiple of `alignment`, which
    /// "Marshaling (Wire Format)" says must be zero.
    pub(crate) fn pad(&mut self, alignment: usize) -> Result<(), Error> {
        let target = self.position.checked_next_multiple_of(alignment).ok_or_else(too_long)?;
        let count = target.checked_sub(self.position).ok_or_else(too_long)?;
        if self.take(count)?.iter().any(|&byte| byte != 0) {
            return Err(Error::Malformed("padding that is not zero"));
        }
        Ok(())
    }

    fn fixed<const N: usize>(&mut self) -> Result<[u8; N], Error> {
        self.pad(N)?;
        let mut bytes = [0; N];
        bytes.copy_from_slice(self.take(N)?);
        if self.big_endian {
            bytes.reverse();
        }
        Ok(bytes)
    }

    pub(crate) fn u8(&mut self) -> Result<u8, Error> {
        self.take(1)?.first().copied().ok_or(Error::Malformed("a message ends inside a value"))
    }

    pub(crate) fn u32(&mut self) -> Result<u32, Error> {
        self.fixed::<4>().map(u32::from_le_bytes)
    }

    /// A string's bytes and its closing zero, after a length of `length`.
    fn text(&mut self, length: usize) -> Result<String, Error> {
        let text = self.take(length)?;
        if self.u8()? != 0 {
            return Err(Error::Malformed("a string not ended by a zero byte"));
        }
        if text.contains(&0) {
            return Err(Error::Malformed("a string holds a zero byte"));
        }
        String::from_utf8(text.to_vec()).map_err(|_| Error::Malformed("a string that is not UTF-8"))
    }

    pub(crate) fn string(&mut self) -> Result<String, Error> {
        let length = usize::try_from(self.u32()?).map_err(|_| too_long())?;
        self.text(length)
    }

    pub(crate) fn signature(&mut self) -> Result<String, Error> {
        let length = usize::from(self.u8()?);
        let signature = self.text(length)?;
        parse_signature(&signature)?;
        Ok(signature)
    }

    /// An array's length and its end, once past the padding to its first
    /// element.
    fn array_end(&mut self, element: &Type) -> Result<usize, Error> {
        let length = usize::try_from(self.u32()?).map_err(|_| too_long())?;
        if length > MAX_ARRAY_BYTES {
            return Err(Error::Malformed("an array longer than 64 MiB"));
        }
        self.pad(element.alignment())?;
        let end = self.position.checked_add(length).ok_or_else(too_long)?;
        if end > self.bytes.len() {
            return Err(Error::Malformed("an array longer than its message"));
        }
        Ok(end)
    }

    fn array_done(&self, end: usize) -> Result<bool, Error> {
        match self.position.cmp(&end) {
            std::cmp::Ordering::Less => Ok(false),
            std::cmp::Ordering::Equal => Ok(true),
            std::cmp::Ordering::Greater => Err(Error::Malformed("an array's last element runs past its length")),
        }
    }

    /// A value of `kind`, `depth` containers deep.
    pub(crate) fn value(&mut self, kind: &Type, depth: usize) -> Result<Value, Error> {
        if depth > MAX_DEPTH {
            return Err(Error::Malformed("containers nested more than 64 deep"));
        }
        let inner = depth.saturating_add(1);
        self.charge(1)?;
        Ok(match kind {
            Type::Byte => Value::Byte(self.u8()?),
            Type::Boolean => match self.u32()? {
                0 => Value::Boolean(false),
                1 => Value::Boolean(true),
                _ => return Err(Error::Malformed("a boolean other than 0 or 1")),
            },
            Type::Int16 => Value::Int16(i16::from_le_bytes(self.fixed()?)),
            Type::Uint16 => Value::Uint16(u16::from_le_bytes(self.fixed()?)),
            Type::Int32 => Value::Int32(i32::from_le_bytes(self.fixed()?)),
            Type::Uint32 => Value::Uint32(self.u32()?),
            Type::UnixFd => Value::UnixFd(self.u32()?),
            Type::Int64 => Value::Int64(i64::from_le_bytes(self.fixed()?)),
            Type::Uint64 => Value::Uint64(u64::from_le_bytes(self.fixed()?)),
            Type::Double => Value::Double(u64::from_le_bytes(self.fixed()?)),
            Type::String => Value::String(self.string()?),
            Type::ObjectPath => {
                let path = self.string()?;
                if !valid_object_path(&path) {
                    return Err(Error::Malformed("an invalid object path"));
                }
                Value::ObjectPath(path)
            }
            Type::Signature => Value::Signature(self.signature()?),
            Type::Array(element) if **element == Type::Byte => {
                let end = self.array_end(element)?;
                let count = end.checked_sub(self.position).ok_or_else(too_long)?;
                Value::Bytes(self.take(count)?.to_vec())
            }
            Type::Array(element) => {
                self.charge(element.nodes())?;
                let end = self.array_end(element)?;
                let mut elements = Vec::new();
                loop {
                    match self.array_done(end).and_then(|done| if done { Ok(None) } else { self.value(element, inner).map(Some) }) {
                        Ok(Some(item)) => elements.push(item),
                        Ok(None) => break,
                        Err(error) => return Err(wiped(elements, error)),
                    }
                }
                Value::Array((**element).clone(), elements)
            }
            Type::Dict(key, value) => {
                self.charge(key.nodes().saturating_add(value.nodes()))?;
                let entry = Type::Struct(Vec::new());
                let end = self.array_end(&entry)?;
                let mut entries = Vec::new();
                loop {
                    match self.array_done(end) {
                        Ok(true) => break,
                        Ok(false) => {}
                        Err(error) => return Err(wiped_entries(entries, error)),
                    }
                    let read = self.pad(entry.alignment()).and_then(|()| self.value(key, inner));
                    match read.and_then(|read_key| match self.value(value, inner) {
                        Ok(read_value) => Ok((read_key, read_value)),
                        Err(error) => Err(wiped(vec![read_key], error)),
                    }) {
                        Ok(pair) => entries.push(pair),
                        Err(error) => return Err(wiped_entries(entries, error)),
                    }
                }
                Value::Dict((**key).clone(), (**value).clone(), entries)
            }
            Type::Struct(fields) => {
                self.pad(kind.alignment())?;
                let mut read = Vec::with_capacity(fields.len());
                for field in fields {
                    match self.value(field, inner) {
                        Ok(value) => read.push(value),
                        Err(error) => return Err(wiped(read, error)),
                    }
                }
                Value::Struct(read)
            }
            Type::Variant => {
                let held = parse_single(&self.signature()?)?;
                Value::Variant(Box::new(self.value(&held, inner)?))
            }
        })
    }
}

/// `error`, once the values read before it are wiped: a secret read part
/// way into a value that turns out malformed stays in no freed buffer
/// (TODO-7173 finding A4).
fn wiped(mut values: Vec<Value>, error: Error) -> Error {
    wipe(&mut values);
    error
}

/// [`wiped`], for a dictionary's entries.
fn wiped_entries(mut entries: Vec<(Value, Value)>, error: Error) -> Error {
    for (key, value) in &mut entries {
        wipe(std::slice::from_mut(key));
        wipe(std::slice::from_mut(value));
    }
    error
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing, reason = "a test")]
mod tests {
    use super::*;

    fn written(value: &Value) -> Vec<u8> {
        let mut writer = Writer::default();
        writer.value(value, 0).unwrap();
        writer.into_bytes()
    }

    fn read(bytes: &[u8], kind: &Type) -> Result<Value, Error> {
        let mut reader = Reader { bytes, position: 0, big_endian: false, budget: bytes.len() };
        let value = reader.value(kind, 0)?;
        assert_eq!(reader.position, bytes.len());
        Ok(value)
    }

    /// "Marshaling (Wire Format)": a string is its length, its bytes and a
    /// zero; an array of 64-bit numbers pads to 8 after its length, and the
    /// length leaves that padding out.
    #[test]
    fn values_are_written_as_the_specification_shows() {
        assert_eq!(written(&Value::String("foo".into())), b"\x03\0\0\0foo\0");
        assert_eq!(written(&Value::Signature("a{sv}".into())), b"\x05a{sv}\0");
        assert_eq!(written(&Value::Array(Type::Uint64, vec![Value::Uint64(5)])), [8, 0, 0, 0, 0, 0, 0, 0, 5, 0, 0, 0, 0, 0, 0, 0]);
        assert_eq!(written(&Value::Variant(Box::new(Value::Uint32(7)))), [1, b'u', 0, 0, 7, 0, 0, 0]);
        assert_eq!(written(&Value::Bytes(vec![1, 2, 3])), [3, 0, 0, 0, 1, 2, 3]);
    }

    #[test]
    fn what_is_written_reads_back() {
        let values = [
            Value::variant_dict(vec![
                ("handle_token", Value::String("t1".into())),
                ("directory", Value::Boolean(true)),
                ("current_folder", Value::Bytes(b"/home\0".to_vec())),
            ]),
            Value::Struct(vec![
                Value::ObjectPath("/org/freedesktop/secrets/session/1".into()),
                Value::Bytes(Vec::new()),
                Value::Bytes(vec![9; 32]),
                Value::String("application/octet-stream".into()),
            ]),
            Value::Array(
                Type::Struct(vec![Type::String, Type::Array(Box::new(Type::Struct(vec![Type::Uint32, Type::String])))]),
                vec![Value::Struct(vec![
                    Value::String("Pictures".into()),
                    Value::Array(Type::Struct(vec![Type::Uint32, Type::String]), vec![Value::Struct(vec![Value::Uint32(0), Value::String("*.png".into())])]),
                ])],
            ),
            Value::Array(Type::Int16, vec![Value::Int16(-2)]),
            Value::Variant(Box::new(Value::Variant(Box::new(Value::Double(0x3ff0_0000_0000_0000))))),
        ];
        for value in values {
            assert_eq!(read(&written(&value), &value.kind()).unwrap(), value);
        }
    }

    #[test]
    fn bad_bytes_are_refused() {
        // A boolean of 2, a string without its zero, one with a zero in it,
        // padding that is not zero, an array past its message, invalid
        // UTF-8, an invalid object path, an unknown variant type.
        let cases: [(&[u8], Type); 8] = [
            (&[2, 0, 0, 0], Type::Boolean),
            (b"\x03\0\0\0fooX", Type::String),
            (b"\x03\0\0\0f\0o\0", Type::String),
            (&[8, 0, 0, 0, 1, 0, 0, 0, 5, 0, 0, 0, 0, 0, 0, 0], Type::Array(Box::new(Type::Uint64))),
            (&[9, 0, 0, 0, 1], Type::Array(Box::new(Type::Byte))),
            (b"\x01\0\0\0\xff\0", Type::String),
            (b"\x03\0\0\0/a/\0", Type::ObjectPath),
            (b"\x01z\0", Type::Variant),
        ];
        for (bytes, kind) in cases {
            assert!(read(bytes, &kind).is_err(), "{bytes:?} as {kind:?}");
        }
    }

    #[test]
    fn variants_nest_no_deeper_than_the_bound() {
        let mut value = Value::Byte(1);
        for _ in 0..=MAX_DEPTH {
            value = Value::Variant(Box::new(value));
        }
        let mut writer = Writer::default();
        assert!(writer.value(&value, 0).is_err());
        // The same bytes, written by hand, are refused when read.
        let mut bytes = Vec::new();
        for _ in 0..=MAX_DEPTH {
            bytes.extend_from_slice(b"\x01v\0");
        }
        bytes.extend_from_slice(b"\x01y\0\x01");
        assert_eq!(read(&bytes, &Type::Variant), Err(Error::Malformed("containers nested more than 64 deep")));
    }

    #[test]
    fn a_big_endian_number_reads_as_its_value() {
        let mut reader = Reader { bytes: &[0, 0, 1, 2], position: 0, big_endian: true, budget: 0 };
        assert_eq!(reader.u32().unwrap(), 0x0102);
    }

    #[test]
    fn mismatched_elements_are_not_written() {
        let mut writer = Writer::default();
        assert!(writer.value(&Value::Array(Type::String, vec![Value::Uint32(1)]), 0).is_err());
        assert!(writer.value(&Value::ObjectPath("no".into()), 0).is_err());
        assert!(writer.value(&Value::Struct(Vec::new()), 0).is_err());
    }
}
