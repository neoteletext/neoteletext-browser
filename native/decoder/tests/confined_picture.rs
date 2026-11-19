//! A decoder, confined as the broker confines it, converts a picture for the
//! Tools menu (`docs/picture-tools.md`, REQ-2393): the picture's decoders
//! run only there, never in the trusted core.

use std::ffi::OsString;
use std::io::{BufReader, Write as _};
use std::path::Path;

use confine::{Role, Spawn};
use interface_core::canvas::PictureLanguage;
use interface_core::picture::PictureChoice;
use protocol::decoder::{FromDecoder, ToDecoder, read_from_decoder, write_to_decoder};

/// A 24-bit BMP of 64 by 48 pixels, written by hand.
fn bmp() -> Vec<u8> {
    let (width, height) = (64_u32, 48_u32);
    let row = (width * 3).next_multiple_of(4);
    let mut bytes = b"BM".to_vec();
    for value in [54 + row * height, 0, 54, 40, width, height] {
        bytes.extend(value.to_le_bytes());
    }
    bytes.extend(1_u16.to_le_bytes());
    bytes.extend(24_u16.to_le_bytes());
    for value in [0_u32, row * height, 2835, 2835, 0, 0] {
        bytes.extend(value.to_le_bytes());
    }
    for y in 0..height {
        for x in 0..width {
            bytes.extend([u8::try_from(x * 4).unwrap(), u8::try_from(y * 5).unwrap(), 200]);
        }
    }
    bytes
}

/// What a confined picture decoder answers for `picture`.
fn converted(picture: &[u8]) -> Vec<FromDecoder> {
    let program = Path::new(env!("CARGO_BIN_EXE_neoteletext-decoder"));
    let arguments = [OsString::from("--picture")];
    let environment = confine::environment(Role::Decoder);
    let mut process =
        confine::spawn(&Spawn { program, arguments: &arguments, role: Role::Decoder, environment: &environment, local_network: false, writable: None })
            .unwrap();
    eprintln!("decoder: {}", process.report);
    let mut input = process.stdin.take().unwrap();
    write_to_decoder(&mut input, &ToDecoder::Picture(PictureChoice::default())).unwrap();
    write_to_decoder(&mut input, &ToDecoder::Data(picture.to_vec())).unwrap();
    write_to_decoder(&mut input, &ToDecoder::End).unwrap();
    input.flush().unwrap();
    let mut output = BufReader::new(process.stdout.take().unwrap());
    let mut said = Vec::new();
    while let Some(message) = read_from_decoder(&mut output).unwrap() {
        said.push(message);
    }
    assert!(process.wait().unwrap().success());
    said
}

/// The page made, then the picture in ReGIS and in NAPLPS (renderer parity
/// PAR-36 item 2), each written.
#[test]
fn a_confined_decoder_converts_a_picture() {
    let said = converted(&bmp());
    let [
        FromDecoder::PictureConverted { tti, t42, summary },
        FromDecoder::PictureOutput { language: PictureLanguage::Regis, output: Ok(regis) },
        FromDecoder::PictureOutput { language: PictureLanguage::Naplps, output: Ok(naplps) },
    ] = said.as_slice()
    else {
        panic!("{said:?}")
    };
    assert!(String::from_utf8_lossy(tti).lines().any(|line| line == "PN,10000"));
    assert!(!t42.is_empty() && t42.len().is_multiple_of(42));
    assert!(summary.contains("colours"), "{summary}");
    assert!(!regis.is_empty() && !naplps.is_empty());
}

#[test]
fn a_confined_decoder_refuses_what_is_no_picture() {
    let mut not_a_picture = bmp();
    not_a_picture.truncate(70);
    let said = converted(&not_a_picture);
    assert!(matches!(said.as_slice(), [FromDecoder::PictureRefused(_)]), "{said:?}");
}
