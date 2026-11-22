//! The input script (script-api.md section 16, "Injects every event of
//! section 4.2"): each kind of line reaches a script as the event the
//! specification names, with every field, through the script library's
//! own prelude. The packing of a pointer event's first number, which the
//! runner writes and the prelude reads, is held together by this test.

mod common;

use common::{CLAIMS, Given, log, message, run, status};
use script_run::Status;
use script_run::input::{Item, parse};

const WATCHER: &str = r#"
const show = (event) => {
  const fields = ["key", "repeat", "shiftKey", "ctrlKey", "altKey", "metaKey", "lost", "data", "pointerId", "pointerType", "isPrimary", "button", "buttons",
                  "x", "y", "localX", "localY", "deltaX", "deltaY", "cause", "tick", "code", "bound"];
  const said = fields.filter((name) => event[name] !== undefined && event[name] !== null).map((name) => name + "=" + event[name]);
  if (event.destination) said.push("to=" + event.destination.url + "|" + event.destination.number);
  console.log(event.type, said.join(" "));
};
for (const type of ["keydown", "keyup", "text", "pointerdown", "pointerup", "pointermove", "pointercancel", "wheel", "focus", "blur",
                    "visibilitychange", "navigate", "fault", "close"]) nt.addEventListener(type, show);
nt.sound.addEventListener("cue", (event) => console.log("cue", event.voice, event.cue));
nt.sound.addEventListener("ended", (event) => console.log("ended", event.voice));
"#;

const INPUT: &str = "\
# every kind of line
1 keydown Enter shift
1 keydown a ctrl alt meta repeat
2 keyup Enter
2 keyup a lost
3 key Enter
4 text h\u{e9}llo\\tthere\\\\
5 pointer down 12 7 button=1 buttons=2 type=touch id=5 local=3,4
6 pointer move 13 8
6 pointer up 13 8 type=pen secondary
7 pointer cancel 0 0
8 wheel 0 -3
9 focus
9 blur
10 navigate link https://channel.test/101
10 navigate browser 200
11 cue 2 9
11 ended 3
11 fault 4 MAX_SPRITES
12 hide
14 show
15 close
";

/// Section 4.2's fields, kind by kind.
#[test]
fn every_kind_of_line_is_the_event_the_specification_names() {
    let page = format!("{CLAIMS}{WATCHER}");
    let made = run(&Given { arguments: &["--ticks", "30"], page: Some(&page), input: Some(INPUT), ..Given::default() });
    assert_eq!(status(&made), Status::Matched, "{}", message(&made));
    let said: Vec<String> =
        log(&made).into_iter().filter(|line| line.contains(" page log ") && !line.contains("tick=")).map(|line| line.replacen(" page log", "", 1)).collect();
    let wanted = [
        "000001 keydown key=Enter repeat=false shiftKey=true ctrlKey=false altKey=false metaKey=false",
        "000001 keydown key=a repeat=true shiftKey=false ctrlKey=true altKey=true metaKey=true",
        "000002 keyup key=Enter repeat=false shiftKey=false ctrlKey=false altKey=false metaKey=false lost=false",
        "000002 keyup key=a repeat=false shiftKey=false ctrlKey=false altKey=false metaKey=false lost=true",
        "000003 keydown key=Enter repeat=false shiftKey=false ctrlKey=false altKey=false metaKey=false",
        "000003 keyup key=Enter repeat=false shiftKey=false ctrlKey=false altKey=false metaKey=false lost=false",
        // The log is plain text: it shows the tab the text carried as a space.
        "000004 text data=h\u{e9}llo there\\",
        "000005 pointerdown pointerId=5 pointerType=touch isPrimary=true button=1 buttons=2 x=12 y=7 localX=3 localY=4",
        "000006 pointermove pointerId=1 pointerType=mouse isPrimary=true button=0 buttons=0 x=13 y=8 localX=0 localY=0",
        "000006 pointerup pointerId=1 pointerType=pen isPrimary=false button=0 buttons=0 x=13 y=8 localX=0 localY=0",
        "000007 pointercancel pointerId=1",
        "000008 wheel x=0 y=0 deltaX=0 deltaY=-3",
        "000009 focus ",
        "000009 blur ",
        "000010 navigate cause=link to=https://channel.test/101|null",
        "000010 navigate cause=browser to=null|200",
        "000011 cue 2 9",
        "000011 ended 3",
        "000011 fault code=4 bound=MAX_SPRITES",
        "000012 visibilitychange ",
        "000014 visibilitychange ",
        "000016 close ",
    ];
    for line in wanted {
        assert!(said.iter().any(|held| held.starts_with(line.trim_end())), "no line `{line}` in {said:#?}");
    }
    // The verdicts: a keydown no handler cancelled was heard and let through.
    assert!(log(&made).iter().any(|line| line == "000001 runner note verdicts hh"), "{:#?}", log(&made));
}

/// The reader: what it refuses, each with its line.
#[test]
fn the_reader_refuses_what_is_not_an_input_script() {
    for (bad, why) in [
        ("x keydown a\n", "not a tick"),
        ("5 keydown a\n4 keydown a\n", "never go down"),
        ("1 keydown\n", "names its key"),
        ("1 keydown a hyper\n", "not a modifier"),
        ("1 keyup a repeat\n", "not a modifier"),
        ("1 pointer press 1 2\n", "not a pointer action"),
        ("1 pointer down 1 2 button=16\n", "is not 0 to 15"),
        ("1 pointer down 1 2 type=finger\n", "not a pointer type"),
        ("1 pointer down 1\n", "pointer ACTION X Y"),
        ("1 wheel 1\n", "wheel DX DY"),
        ("1 text a\\q\n", "followed by"),
        ("1 navigate whim 100\n", "not a cause"),
        ("1 fault 1 lower\n", "not a name in capitals"),
        ("1 focus now\n", "takes no fields"),
        ("36000 focus\n", "not a tick"),
        ("1\n", "TICK kind fields"),
    ] {
        let refused = parse(bad.as_bytes()).expect_err(bad);
        assert!(refused.contains(why) && refused.starts_with("line "), "`{}` was refused with `{refused}`, not for `{why}`", bad.trim_end());
    }
    assert!(parse(&[0xFF, b'\n']).is_err(), "bytes that are not UTF-8");
    let read = parse(b"# a comment\n\n3 key a\r\n3 painted 7\n3 bounds 7 1 2 3 4\n3 cycleend 9\n3 ui 4 2 {\"value\":1}\n").unwrap();
    assert_eq!(read.items.len(), 6);
    assert_eq!(read.at(3).count(), 6);
    assert_eq!(read.at(2).count(), 0);
    assert!(matches!(read.items[2], (3, Item::Painted(7))));
}
