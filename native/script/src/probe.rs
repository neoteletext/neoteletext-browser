//! A module that does nothing but forward to the container's imports, for
//! the tests and the fuzz target of the imports themselves (the security
//! review's finding S-09 item 5, TODO-7275; script-api.md section 11.4,
//! target `script_imports`).
//!
//! The imports must stay bounded against a module that has been taken
//! over: one whose code calls them with arguments of an attacker's
//! choosing over a memory of an attacker's making. The real module cannot
//! be made to do that on demand, so this one is assembled here, as the
//! WebAssembly binary format writes it (WebAssembly core specification
//! 2.0, section 5): for each import an exported function of the same
//! signature that passes its arguments straight through, and a memory
//! that starts as the bytes given.

use crate::machine::IMPORTS;

/// The value types (specification section 5.3.1).
const I32: u8 = 0x7F;
const I64: u8 = 0x7E;
/// Each import's parameters and results, in the order of [`IMPORTS`].
pub const SIGNATURES: [(&[u8], &[u8]); 7] =
    [(&[I32; 10], &[]), (&[], &[]), (&[I32, I64, I32], &[I32]), (&[I32; 4], &[I32]), (&[I32], &[I32]), (&[I32, I64, I32, I32], &[I32]), (&[I32, I32], &[I32])];
/// Whether each parameter of each import is 64 bits wide.
#[must_use]
pub fn is_wide(import: usize, parameter: usize) -> bool {
    SIGNATURES.get(import).and_then(|(parameters, _)| parameters.get(parameter)).is_some_and(|kind| *kind == I64)
}
/// The module's preamble: the magic number and version 1 (section 5.5.16).
const PREAMBLE: [u8; 8] = [0x00, 0x61, 0x73, 0x6D, 0x01, 0x00, 0x00, 0x00];
/// Section numbers (section 5.5.2).
const SECTION_TYPE: u8 = 1;
const SECTION_IMPORT: u8 = 2;
const SECTION_FUNCTION: u8 = 3;
const SECTION_MEMORY: u8 = 5;
const SECTION_EXPORT: u8 = 7;
const SECTION_CODE: u8 = 10;
const SECTION_DATA: u8 = 11;
/// A function type's tag (section 5.3.6); the import and export kinds for
/// a function and a memory (sections 5.5.5 and 5.5.10); a memory's limits
/// with a maximum (section 5.3.4).
const FUNCTION_TYPE: u8 = 0x60;
const KIND_FUNCTION: u8 = 0x00;
const KIND_MEMORY: u8 = 0x02;
const LIMITS_WITH_MAXIMUM: u8 = 0x01;
/// Instructions (section 5.4): `local.get`, `call`, `i32.const`, `end`.
const LOCAL_GET: u8 = 0x20;
const CALL: u8 = 0x10;
const I32_CONST: u8 = 0x41;
const END: u8 = 0x0B;
/// A WebAssembly page, in bytes (specification section 4.2.8).
pub const PAGE_BYTES: usize = 65_536;

/// Appends `value` as an unsigned LEB128 number (section 5.2.2).
fn unsigned(out: &mut Vec<u8>, mut value: usize) {
    loop {
        let low = u8::try_from(value & 0x7F).unwrap_or(0);
        value >>= 7;
        if value == 0 {
            out.push(low);
            return;
        }
        out.push(low | 0x80);
    }
}

/// Appends a section: its number, its length and its contents.
fn section(out: &mut Vec<u8>, number: u8, contents: &[u8]) {
    out.push(number);
    unsigned(out, contents.len());
    out.extend_from_slice(contents);
}

/// Appends a name: its length and its bytes.
fn name(out: &mut Vec<u8>, text: &str) {
    unsigned(out, text.len());
    out.extend_from_slice(text.as_bytes());
}

/// The forwarding module, its memory `pages` pages (and never more),
/// starting as `image` and zeros after it. Each import is exported as
/// `module.name` (`nt.call`, `wasi_snapshot_preview1.fd_write` and so on),
/// and the memory as `memory`.
#[must_use]
pub fn forwarder(image: &[u8], pages: usize) -> Vec<u8> {
    let image = image.get(..image.len().min(pages.saturating_mul(PAGE_BYTES))).unwrap_or(&[]);
    let count = IMPORTS.len();
    let mut module = PREAMBLE.to_vec();

    let mut types = Vec::new();
    unsigned(&mut types, count);
    for (parameters, results) in SIGNATURES {
        types.push(FUNCTION_TYPE);
        unsigned(&mut types, parameters.len());
        types.extend_from_slice(parameters);
        unsigned(&mut types, results.len());
        types.extend_from_slice(results);
    }
    section(&mut module, SECTION_TYPE, &types);

    let mut imports = Vec::new();
    unsigned(&mut imports, count);
    for (index, (from, what)) in IMPORTS.iter().enumerate() {
        name(&mut imports, from);
        name(&mut imports, what);
        imports.push(KIND_FUNCTION);
        unsigned(&mut imports, index);
    }
    section(&mut module, SECTION_IMPORT, &imports);

    let mut functions = Vec::new();
    unsigned(&mut functions, count);
    for index in 0..count {
        unsigned(&mut functions, index);
    }
    section(&mut module, SECTION_FUNCTION, &functions);

    let mut memory = Vec::new();
    unsigned(&mut memory, 1);
    memory.push(LIMITS_WITH_MAXIMUM);
    unsigned(&mut memory, pages);
    unsigned(&mut memory, pages);
    section(&mut module, SECTION_MEMORY, &memory);

    let mut exports = Vec::new();
    unsigned(&mut exports, count.saturating_add(1));
    name(&mut exports, "memory");
    exports.push(KIND_MEMORY);
    unsigned(&mut exports, 0);
    for (index, (from, what)) in IMPORTS.iter().enumerate() {
        name(&mut exports, &format!("{from}.{what}"));
        exports.push(KIND_FUNCTION);
        // The forwarders follow the imports in the function index space.
        unsigned(&mut exports, count.saturating_add(index));
    }
    section(&mut module, SECTION_EXPORT, &exports);

    let mut code = Vec::new();
    unsigned(&mut code, count);
    for (index, (parameters, _)) in SIGNATURES.iter().enumerate() {
        let mut body = Vec::new();
        // No locals beside the parameters.
        unsigned(&mut body, 0);
        for parameter in 0..parameters.len() {
            body.push(LOCAL_GET);
            unsigned(&mut body, parameter);
        }
        body.push(CALL);
        unsigned(&mut body, index);
        body.push(END);
        unsigned(&mut code, body.len());
        code.extend_from_slice(&body);
    }
    section(&mut module, SECTION_CODE, &code);

    let mut data = Vec::new();
    unsigned(&mut data, 1);
    // An active segment of memory 0 at offset 0 (section 5.5.14).
    unsigned(&mut data, 0);
    data.extend_from_slice(&[I32_CONST, 0, END]);
    unsigned(&mut data, image.len());
    data.extend_from_slice(image);
    section(&mut module, SECTION_DATA, &data);
    module
}

/// A module of one function holding `memory.fill` over its one page (the
/// bulk-memory proposal's instruction 0xFC 11), which the engine must
/// refuse: for the test of that refusal.
#[must_use]
pub fn bulk_memory_module() -> Vec<u8> {
    let mut module = PREAMBLE.to_vec();
    section(&mut module, SECTION_TYPE, &[1, FUNCTION_TYPE, 0, 0]);
    section(&mut module, SECTION_FUNCTION, &[1, 0]);
    section(&mut module, SECTION_MEMORY, &[1, LIMITS_WITH_MAXIMUM, 1, 1]);
    // (func (memory.fill (i32.const 0) (i32.const 0) (i32.const 0)))
    let body = [0, I32_CONST, 0, I32_CONST, 0, I32_CONST, 0, 0xFC, 11, 0, END];
    let mut code = vec![1];
    unsigned(&mut code, body.len());
    code.extend_from_slice(&body);
    section(&mut module, SECTION_CODE, &code);
    module
}
