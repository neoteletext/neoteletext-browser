//! The picture of a run (script-api.md section 16, "Snapshots pictures
//! and sound"): a batch applied to the page by the decoder's own code and
//! painted as the decoder paints it, for each renderer.

mod common;

use common::png_size;
use protocol::script::Renderer;
use protocol::script::batch::{Batch, NaplpsAppend, TeletextRow};
use script_run::screen::{Place, Screen};

const PLACE: Place = Place { page: 0x100, subcode: 0 };

/// A JavaScript page, opened in `renderer`.
fn blank(renderer: Renderer) -> Screen {
    let mut screen = Screen::new(PLACE);
    screen.settle_subpage();
    screen.open(renderer, "en");
    screen
}

/// A JavaScript page opens blank (section 12.3): a teletext one is a
/// page of no rows, 480 by 500, whatever it is to be drawn in: the
/// decoder carries a ReGIS or NAPLPS page by a teletext page of its
/// number, and paints it in its renderer's size once its script draws.
#[test]
fn a_javascript_page_is_drawn_in_its_renderer() {
    let mut teletext = blank(Renderer::Teletext);
    assert_eq!(teletext.held_renderer(), Some(Renderer::Teletext));
    assert_eq!(png_size(&teletext.snapshot().unwrap()), (480, 500));
    for (renderer, batch, size) in [
        (Renderer::Regis, Batch { regis: Some(b"S(E)P[100,100]W(I3)V[300,300]C[+50]".to_vec()), ..Batch::default() }, (800, 480)),
        (Renderer::Naplps, Batch { naplps: Some(NaplpsAppend { new_generation: true, bytes: b"\x1b\x22\x40".to_vec() }), ..Batch::default() }, (480, 500)),
    ] {
        let mut screen = blank(renderer);
        // Carried by a blank teletext page until its script draws.
        assert_eq!(screen.held_renderer(), Some(Renderer::Teletext));
        assert_eq!(png_size(&screen.snapshot().unwrap()), (480, 500));
        screen.set_tick(1);
        screen.apply(1, &batch, true).unwrap();
        screen.paint();
        assert_eq!(screen.held_renderer(), Some(renderer));
        assert_eq!(png_size(&screen.snapshot().unwrap()), size, "{renderer:?}");
    }
}

/// A batch changes the picture, the same way every time; a batch of
/// another renderer than the page's is refused whole, as the decoder
/// refuses it.
#[test]
fn a_batch_is_applied_by_the_decoder() {
    let mut row = [0x20_u8; 40];
    row[..5].copy_from_slice(b"HELLO");
    let rows = Batch { teletext_rows: Some(vec![TeletextRow { row: 3, codes: row }]), ..Batch::default() };
    let drawn = |batch: &Batch| {
        let mut screen = blank(Renderer::Teletext);
        let before = screen.snapshot().unwrap();
        screen.set_tick(1);
        screen.apply(1, batch, true).unwrap();
        screen.paint();
        (before, screen.snapshot().unwrap(), screen.output_bytes)
    };
    let (before, after, bytes) = drawn(&rows);
    assert_ne!(before, after, "the batch drew nothing");
    assert!(bytes > 0);
    assert_eq!(drawn(&rows).1, after, "the same batch, the same picture");
    // A close's batch draws nothing.
    let mut closing = blank(Renderer::Teletext);
    closing.apply(1, &rows, false).unwrap();
    closing.paint();
    assert_eq!(closing.snapshot().unwrap(), before);
    let mut teletext = blank(Renderer::Teletext);
    let refused = teletext.apply(1, &Batch { regis: Some(b"S(E)".to_vec()), ..Batch::default() }, true).unwrap_err();
    assert!(refused.contains("another renderer"), "{refused}");
}

/// A page file is read by the engine's own readers and drawn as the
/// browser draws it, and its `JS` line is what the decoder says it is.
#[test]
fn a_page_file_is_shown_before_any_script() {
    let mut with_text = Screen::new(Place { page: 0x100, subcode: 0x3F7F });
    with_text.page_file(b"PN,10000\r\nSC,0000\r\nJS,page.js\r\nOL,5,HELLO FROM A PAGE FILE\r\n");
    with_text.settle_subpage();
    assert_eq!(with_text.place().subcode, 0, "the sub-page the file holds");
    assert_eq!(with_text.script_names(), [(0x100, 0, Some("page.js".to_owned()))]);
    assert_eq!(with_text.held_renderer(), Some(Renderer::Teletext));
    with_text.open(Renderer::Teletext, "en");
    let drawn = with_text.snapshot().unwrap();
    assert_eq!(png_size(&drawn), (480, 500));
    assert_ne!(drawn, blank(Renderer::Teletext).snapshot().unwrap());
}
