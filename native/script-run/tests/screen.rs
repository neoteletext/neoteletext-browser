//! The picture of a run (script-api.md section 16, "Snapshots pictures
//! and sound"): the decoder's own snapshot of the page, for each
//! renderer, and what the runner's stand-in for the decoder's side of a
//! step (work package F4) applies of a batch and what it leaves out.

mod common;

use common::png_size;
use protocol::script::Renderer;
use protocol::script::batch::{Batch, NaplpsAppend, TeletextRow, Triplet};
use script_run::screen::{Place, Screen, StandIn};

const PLACE: Place = Place { page: 0x100, subcode: 0 };

fn blank(renderer: Renderer) -> Screen {
    let mut screen = Screen::new(PLACE, Box::new(StandIn::default()));
    screen.blank(renderer);
    screen
}

/// A JavaScript page opens blank in its renderer (section 12.3), and a
/// snapshot has that renderer's size: teletext's 480 by 500, ReGIS's 800
/// by 480.
#[test]
fn a_blank_page_is_drawn_in_its_renderer() {
    let mut teletext = blank(Renderer::Teletext);
    assert_eq!(teletext.renderer(), Renderer::Teletext);
    assert_eq!(png_size(&teletext.snapshot().unwrap()), (480, 500));
    let mut regis = blank(Renderer::Regis);
    assert_eq!(regis.renderer(), Renderer::Regis);
    assert_eq!(png_size(&regis.snapshot().unwrap()), (800, 480));
    let mut naplps = blank(Renderer::Naplps);
    assert_eq!(naplps.renderer(), Renderer::Naplps);
    let size = png_size(&naplps.snapshot().unwrap());
    assert_eq!(size.0, 480, "{size:?}");
}

/// A batch's teletext rows, ReGIS text and NAPLPS bytes change the
/// picture, the same way every time; the sections the stand-in cannot
/// apply are named, never dropped silently.
#[test]
fn a_batch_changes_the_picture_and_what_is_left_out_is_named() {
    let mut row = [0x20_u8; 40];
    row[..5].copy_from_slice(b"HELLO");
    let rows = Batch { teletext_rows: Some(vec![TeletextRow { row: 3, codes: row }]), ..Batch::default() };
    let regis = Batch { regis: Some(b"S(E)P[100,100]W(I3)V[300,300]C[+50]".to_vec()), ..Batch::default() };
    for (renderer, batch) in [(Renderer::Teletext, rows), (Renderer::Regis, regis)] {
        let mut screen = blank(renderer);
        let before = screen.snapshot().unwrap();
        screen.set_tick(1);
        screen.apply(&batch);
        let after = screen.snapshot().unwrap();
        assert_ne!(before, after, "{renderer:?}: the batch drew nothing");
        assert!(screen.output_bytes > 0 && screen.left_out.is_empty());
        let mut again = blank(renderer);
        let _ = again.snapshot().unwrap();
        again.set_tick(1);
        again.apply(&batch);
        assert_eq!(again.snapshot().unwrap(), after, "{renderer:?}: the same batch, the same picture");
        // A snapshot with nothing new is the same picture again.
        assert_eq!(screen.snapshot().unwrap(), after);
    }
    let mut naplps = blank(Renderer::Naplps);
    naplps.apply(&Batch { naplps: Some(NaplpsAppend { new_generation: false, bytes: b"\x1b\x22\x40".to_vec() }), ..Batch::default() });
    assert!(naplps.output_bytes > 0);
    assert!(naplps.snapshot().is_ok());

    let mut teletext = blank(Renderer::Teletext);
    teletext.apply(&Batch { teletext_enhancement: Some(vec![Triplet { address: 1, mode: 2, data: 3 }]), sprites: Some(Vec::new()), ..Batch::default() });
    assert_eq!(teletext.left_out, ["teletext enhancement", "teletext sprites"]);
}

/// A page file is read by the engine's own readers and drawn as the
/// browser draws it: a TTI page's text is in the picture.
#[test]
fn a_page_file_is_shown_before_any_script() {
    let mut with_text = Screen::new(Place { page: 0x100, subcode: 0x3F7F }, Box::new(StandIn::default()));
    with_text.page_file(b"PN,10000\r\nSC,0000\r\nOL,5,HELLO FROM A PAGE FILE\r\n");
    with_text.settle_subpage();
    assert_eq!(with_text.place().subcode, 0, "the first sub-page the file holds");
    assert_eq!(with_text.renderer(), Renderer::Teletext);
    let drawn = with_text.snapshot().unwrap();
    assert_eq!(png_size(&drawn), (480, 500));
    assert_ne!(drawn, blank(Renderer::Teletext).snapshot().unwrap());
}
