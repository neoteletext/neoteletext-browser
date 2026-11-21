//! What a message decodes to is bounded by its size (TODO-7173 finding A1):
//! at most `MAX_DECODED_NODES_PER_BYTE` values, and types kept by arrays,
//! for each of its bytes. Built only through the crate's public interface.
#![allow(clippy::unwrap_used, clippy::indexing_slicing, reason = "a test")]

use neoteletext_dbus::{Error, Kind, MAX_DECODED_NODES_PER_BYTE, MAX_MESSAGE_BYTES, MAX_STRUCT_DEPTH, Message, Type, Value, nodes};

/// Where the fixed header keeps the body's length ("Message Format").
const BODY_LENGTH_AT: usize = 4;
/// The bytes of an array's length.
const LENGTH_BYTES: usize = 4;
/// The fields of the review's structure: 250 bytes, so that the signature
/// `aa(` + 250 `y` + `)` is 254 bytes, within the 255 a signature may be.
const STRUCTURE_FIELDS: usize = 250;

/// A signal whose body is one array of `element`, all zero, as long as
/// `MAX_MESSAGE_BYTES` allows: its first element `first_element` bytes and
/// each after it `element_stride`. The header comes from encoding an empty
/// array, then the array's length and the body's are written over and the
/// zeros appended.
fn longest_zero_array(element: Type, element_stride: usize, first_element: usize) -> Vec<u8> {
    let mut signal = Message::method_call("org.example.Victim", "/org/example", "org.example.Test", "Flood", vec![Value::Array(element, Vec::new())]);
    signal.kind = Kind::Signal;
    signal.serial = 1;
    let mut bytes = signal.encode().unwrap();
    let length_field = BODY_LENGTH_AT..BODY_LENGTH_AT + LENGTH_BYTES;
    // The empty array is its length and the padding to its first element.
    let array_at = bytes.len() - usize::try_from(u32::from_le_bytes(bytes[length_field.clone()].try_into().unwrap())).unwrap();
    let room = MAX_MESSAGE_BYTES - bytes.len();
    let content = first_element + (room - first_element) / element_stride * element_stride;
    bytes[array_at..array_at + LENGTH_BYTES].copy_from_slice(&u32::try_from(content).unwrap().to_le_bytes());
    bytes.resize(bytes.len() + content, 0);
    let body = u32::try_from(bytes.len() - array_at).unwrap();
    bytes[length_field].copy_from_slice(&body.to_le_bytes());
    bytes
}

/// The review's probe: 128 KiB of `aa(y…y)`, an outer array of 16,342 empty
/// inner arrays, each keeping the 250-field structure's type. Before the
/// bound it decoded to 99,237,976 bytes held, 757 times its size.
#[test]
fn an_array_of_empty_arrays_of_a_wide_structure_is_refused() {
    let wide = Type::Struct(vec![Type::Byte; STRUCTURE_FIELDS]);
    // Each empty inner array is its length and the padding to its
    // structure's 8-byte alignment; the first, at an offset of 4, has none.
    let bytes = longest_zero_array(Type::Array(Box::new(wide)), 8, 4);
    assert!(bytes.len() > MAX_MESSAGE_BYTES - 8 && bytes.len() <= MAX_MESSAGE_BYTES);
    assert_eq!(Message::decode(&bytes).err(), Some(Error::Malformed("a message that decodes to more values than it has bytes")));
}

/// Structures nested 32 deep around one byte: 33 values in each 8 bytes,
/// each element one byte and the padding to the next.
#[test]
fn an_array_of_deeply_nested_structures_is_refused() {
    let mut nested = Type::Byte;
    for _ in 0..MAX_STRUCT_DEPTH {
        nested = Type::Struct(vec![nested]);
    }
    let bytes = longest_zero_array(nested, 8, 1);
    assert_eq!(Message::decode(&bytes).err(), Some(Error::Malformed("a message that decodes to more values than it has bytes")));
}

/// The longest array of empty byte arrays a message holds still decodes,
/// within the bound.
#[test]
fn the_longest_array_of_empty_byte_arrays_decodes_within_the_bound() {
    let bytes = longest_zero_array(Type::Array(Box::new(Type::Byte)), 4, 4);
    let message = Message::decode(&bytes).unwrap();
    let held = nodes(&message.body);
    assert!(held > MAX_MESSAGE_BYTES / 8, "{held} nodes");
    assert!(held <= bytes.len() * MAX_DECODED_NODES_PER_BYTE, "{held} nodes in {} bytes", bytes.len());
}
