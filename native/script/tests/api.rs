//! The shared core of the API `nt` as a script sees it (script-api.md
//! sections 3 and 4): each test runs JavaScript in the container and
//! reads what it logged and what its batches hold.
mod common;

use common::{
    Opens as _, all_lines, assert_checks, claim, container, ints_event, key, lines, opening, page, plain_event, run, script, text_event, tick, view,
    with_events,
};
use neoteletext_script::ops::{KeyBit, NavigateCause, PointerAction};
use protocol::script::batch::SoundOperation;
use protocol::script::{ContextId, EventKind, FailReason, FaultReport, SoundReport, Step, StepFlags};

/// Section 10.1: the prelude takes the host's functions and leaves no
/// name of its own in the script's realm; section 3.3: `nt` is frozen and
/// says what it is; departures D13 and D14 and DEC-7282 item 11: no
/// shared memory, no threads; DEC-7287 (this session): no `WeakRef`.
#[test]
fn the_realm_holds_nt_and_none_of_the_hosts_own_names() {
    let mut container = page(&script(
        r#"
        for (const name of ["__host", "__setDispatch", "__setRejection", "SharedArrayBuffer", "Atomics", "WeakRef", "FinalizationRegistry",
                            "OP", "EV", "INFO", "BOUND", "host", "core", "dispatch", "things"]) {
          equal("no " + name, typeof globalThis[name], "undefined");
          let reached = true;
          try { eval(name); } catch (error) { reached = false; }
          check("no binding " + name, !reached);
        }
        check("nt is frozen", Object.isFrozen(nt) && Object.isFrozen(Object.getPrototypeOf(nt)));
        equal("version", nt.version, 1);
        equal("level", nt.level, "page");
        equal("channel", nt.channel.url, "https://example.test/chan/");
        equal("channel title", nt.channel.title, "Example");
        check("channel is frozen", Object.isFrozen(nt.channel));
        equal("page number", nt.page.number, "100");
        equal("page subpage", nt.page.subpage, null);
        equal("page renderer", nt.page.renderer, "teletext");
        equal("page url", nt.page.url, "https://example.test/chan/100.tti");
        nt.page.title = "Scores";
        equal("page title", nt.page.title, "Scores");
        throws("title bound", RangeError, "MAX_TITLE_BYTES: ", () => { nt.page.title = "t".repeat(65); });
        equal("hidden", nt.hidden, false);
        equal("now", nt.now(), 0);
        check("nt is an EventTarget", nt instanceof EventTarget);
        equal("strict", (function () { return this; })(), undefined);
        throws("strict assignment", ReferenceError, "", () => { undeclared = 1; });
        throws("nt cannot be replaced", TypeError, "", () => { globalThis.nt = {}; });
        equal("atob", atob(btoa("teletext")), "teletext");
        check("queueMicrotask", typeof queueMicrotask === "function");
        done();
        "#,
    ));
    let batch = container.step(&tick(1, 0)).unwrap();
    assert_checks(&lines(&batch), 50);
    assert_eq!(batch.title.as_deref(), Some("Scores"));
}

/// Section 3.3 and DOM: listeners in order, once, removal, duplicates,
/// `stopImmediatePropagation`, `handleEvent`, `signal`; a script's own
/// events dispatch, the host's cannot be forged.
#[test]
fn event_targets_follow_dom() {
    let mut container = page(&script(
        r#"
        const target = new EventTarget();
        const seen = [];
        const first = (event) => seen.push("first:" + event.type);
        target.addEventListener("go", first);
        target.addEventListener("go", first);
        target.addEventListener("go", { handleEvent(event) { seen.push("object"); } });
        target.addEventListener("go", () => seen.push("once"), { once: true });
        const event = new CustomEvent("go", { detail: 7, cancelable: true });
        equal("not cancelled", target.dispatchEvent(event), true);
        target.dispatchEvent(new Event("go"));
        equal("order", seen.join(","), "first:go,object,once,first:go,object");
        equal("detail", event.detail, 7);
        equal("isTrusted", event.isTrusted, false);
        equal("target", event.target, target);
        target.removeEventListener("go", first);
        seen.length = 0;
        target.addEventListener("go", (e) => { seen.push("stop"); e.stopImmediatePropagation(); e.preventDefault(); });
        target.addEventListener("go", () => seen.push("never"));
        equal("cancelled", target.dispatchEvent(new Event("go", { cancelable: true })), false);
        equal("not cancelable", target.dispatchEvent(new Event("go")), true);
        equal("stopped", seen.join(","), "object,stop,object,stop");
        const controller = new AbortController();
        let heard = 0;
        target.addEventListener("abortable", () => heard++, { signal: controller.signal });
        target.dispatchEvent(new Event("abortable"));
        controller.abort();
        target.dispatchEvent(new Event("abortable"));
        equal("signal removes", heard, 1);
        equal("aborted", controller.signal.aborted, true);
        check("abort reason", controller.signal.reason instanceof DOMException && controller.signal.reason.name === "AbortError");
        throws("throwIfAborted", DOMException, "", () => controller.signal.throwIfAborted());
        equal("AbortSignal.abort", AbortSignal.abort("why").reason, "why");
        equal("any", AbortSignal.any([new AbortController().signal, AbortSignal.abort(3)]).reason, 3);
        throws("no signal by hand", TypeError, "", () => new AbortSignal());
        // A script cannot forge the host's events (section 3.3).
        for (const type of ["keydown", "pointerdown", "tick", "navigate", "close", "error"]) {
          throws("forged " + type, TypeError, "'" + type + "' is an event only the host raises", () => nt.dispatchEvent(new Event(type)));
        }
        let own = 0;
        nt.addEventListener("score", (e) => own += e.detail);
        nt.dispatchEvent(new CustomEvent("score", { detail: 5 }));
        equal("own event on nt", own, 5);
        throws("not an event", TypeError, "", () => target.dispatchEvent({ type: "go" }));
        done();
        "#,
    ));
    assert_checks(&lines(&container.step(&tick(1, 0)).unwrap()), 23);
}

/// Section 3.3 and departures D12 to D14: timers are rounded up to ticks,
/// the least is one tick; all time is given to the tick.
#[test]
fn timers_and_clocks_keep_to_the_tick() {
    let mut container = page(&script(
        r#"
        const fired = [];
        const at = (name) => () => fired.push(name + "@" + nt.now());
        setTimeout(at("zero"), 0);
        setTimeout(at("sixteen"), 16);
        setTimeout(at("seventeen"), 17);
        setTimeout(at("fifty"), 50);
        setTimeout((a, b) => fired.push("args:" + a + b), 1, "x", "y");
        const cancelled = setTimeout(at("cancelled"), 1);
        clearTimeout(cancelled);
        clearTimeout(cancelled);
        clearTimeout(123456);
        clearTimeout("not a timer");
        let count = 0;
        const every = setInterval(() => { fired.push("every@" + nt.now()); if (++count === 3) clearInterval(every); }, 20);
        throws("string handler", TypeError, "a timer's handler is a function", () => setTimeout("fired.push('evaluated')", 1));
        throws("too long", RangeError, "MAX_TIMER_TICKS: ", () => setTimeout(() => {}, 3600001));
        const before = [Date.now(), performance.now()];
        let spin = 0; for (let i = 0; i < 20000; i++) spin += i;
        equal("Date does not move in a callback", Date.now(), before[0]);
        equal("performance.now does not move in a callback", performance.now(), before[1]);
        equal("performance.now starts at 0", before[1], 0);
        equal("the epoch", before[0], 1800000000000);
        equal("timeOrigin", performance.timeOrigin, 1800000000000);
        equal("the zone", new Date().getTimezoneOffset(), -600);
        equal("local hours", new Date(0).getHours(), 10);
        setTimeout(() => {
          equal("fired", fired.join(" "), "zero@1 sixteen@1 args:xy seventeen@2 every@2 fifty@3 every@4 every@6");
          equal("Date at tick 9", Date.now(), 1800000000150);
          equal("performance at tick 9", performance.now(), 150);
          const held = [];
          for (let i = 0; i < 32; i++) held.push(setTimeout(() => {}, 1000));
          throws("too many", RangeError, "MAX_TIMERS: ", () => setTimeout(() => {}, 1));
          held.forEach(clearTimeout);
          done();
        }, 150);
        "#,
    ));
    let batches = run(&mut container, 0, 11);
    assert_checks(&all_lines(&batches), 13);
    // The state says when the decoder must wake: the earliest timer.
    let first = batches[0].as_ref().unwrap();
    assert_eq!(view(first).states[0].next_due, Some(1));
}

/// Sections 4.1 to 4.3: host events arrive as DOM events with their
/// fields; the verdict says whether the default was cancelled and whether
/// a listener heard it; only what the manifest claimed is ever given.
#[test]
fn input_events_reach_their_listeners_and_answer_with_verdicts() {
    let mut container = page(&script(
        r#"
        const seen = [];
        nt.addEventListener("keydown", (e) => {
          seen.push(["keydown", e.key, e.repeat, e.shiftKey, e.ctrlKey, e.altKey, e.metaKey, e.isTrusted, e.cancelable, nt.keys.down(e.key), nt.keys.pressed(e.key), e instanceof KeyboardEvent, "code" in e].join());
          if (e.key === "Enter") e.preventDefault();
        });
        nt.addEventListener("keyup", (e) => { seen.push(["keyup", e.key, e.lost, e.cancelable, nt.keys.down(e.key)].join()); e.preventDefault(); });
        nt.addEventListener("text", (e) => seen.push(["text", e.data, e instanceof NtTextEvent].join()));
        nt.addEventListener("pointerdown", (e) => {
          const p = nt.pointer;
          seen.push(["pointerdown", e.x, e.y, e.button, e.buttons, e.pointerId, e.pointerType, e.isPrimary, e.localX, e.localY, e.things.length, e.target === nt, p.x, p.y, p.buttons, p.inside, p.target, "pressure" in e].join());
        });
        nt.addEventListener("pointermove", (e) => seen.push(["pointermove", e.x, e.y].join()));
        nt.addEventListener("wheel", (e) => seen.push(["wheel", e.deltaX, e.deltaY].join()));
        nt.addEventListener("focus", () => seen.push("focus"));
        nt.addEventListener("blur", () => seen.push("blur"));
        let ticks = 0;
        nt.addEventListener("tick", (e) => { if (ticks++ === 0) seen.push(["tick", e.tick, e.missed, e instanceof NtTickEvent].join()); });
        setTimeout(() => {
          equal("events", seen.join(" | "), [
            "tick,0,0,true",
            "keydown,Enter,false,true,false,false,false,true,true,true,true,true,false",
            "keydown,a,true,false,true,false,false,true,true,true,false,true,false",
            "keyup,Enter,false,false,false",
            "text,héllo,true",
            "pointerdown,12,7,1,2,5,touch,true,3,4,0,true,12,7,2,true,,false",
            "pointermove,13,8",
            "wheel,0,-3",
            "focus", "blur",
          ].join(" | "));
          done();
        }, 30);
        "#,
    ));
    assert_eq!(container.step(&tick(1, 0)).unwrap().verdicts, None);
    let pointer = |action: PointerAction, extra: i32, x, y| ints_event(EventKind::Pointer, [action.number() | extra, x, y, 0], &[5, 2, 3, 4]);
    let events = vec![
        key(EventKind::KeyDown, "Enter", KeyBit::SHIFT.number()),
        key(EventKind::KeyDown, "a", KeyBit::CTRL.number() | KeyBit::REPEAT_OR_LOST.number()),
        // A key the manifest did not claim is never given to the script.
        key(EventKind::KeyDown, "q", 0),
        key(EventKind::KeyUp, "Enter", 0),
        text_event(EventKind::Text, [0; 4], "héllo"),
        // Button 1, a touch (type 1 in bits 8 and 9), the primary pointer (bit 10).
        pointer(PointerAction::DOWN, (1 << 4) | (1 << 8) | (1 << 10), 12, 7),
        pointer(PointerAction::MOVE, 0, 13, 8),
        plain_event(EventKind::Wheel, [0, -3, 0, 0]),
        plain_event(EventKind::Focus, [0; 4]),
        plain_event(EventKind::Blur, [0; 4]),
    ];
    let batch = container.step(&with_events(2, 1, events)).unwrap();
    // Bit 0: the default was cancelled; bit 1: a listener heard it. A
    // keyup's default cannot be cancelled whatever its listener calls.
    assert_eq!(view(&batch).verdicts, vec![3, 2, 0, 2, 2, 2, 2, 2, 2, 2]);
    let batches = run(&mut container, 2, 40);
    assert_checks(&all_lines(&batches), 1);
}

/// Section 4.5: an input event goes to the page context and then the
/// channel context; neither can blind the other; either can cancel.
/// Section 3.1: two realms that share nothing a script can reach.
/// Section 3.6: the shared store, and `sharedchange` in the same step.
#[test]
fn two_contexts_share_only_the_store() {
    let mut container = container();
    let mut channel = opening(ContextId::Channel);
    claim(&mut channel, &["Enter", "a"]);
    container
        .open_with(
            channel,
            script(
                r#"
                globalThis.channelOnly = 1;
                Array.prototype.poisoned = "by the channel";
                equal("level", nt.level, "channel");
                equal("no page before pageshow", nt.page, null);
                nt.addEventListener("pageshow", (e) => {
                  console.log("channel pageshow " + e.page.number + " " + e.page.subpage + " " + e.page.renderer + " " + nt.page.number);
                  try { nt.page.title = "x"; console.error("FAIL the channel set the title"); } catch (error) { check("title refused", error instanceof TypeError); }
                });
                nt.addEventListener("keydown", (e) => { console.log("channel keydown " + e.key + " " + e.target); if (e.key === "a") e.preventDefault(); });
                nt.addEventListener("sharedchange", (e) => console.log("channel sharedchange " + e.key + "=" + JSON.stringify(nt.shared.get(e.key))));
                nt.addEventListener("pagehide", (e) => { console.log("channel pagehide " + e.page.number); done(); });
                "#,
            )
            .as_bytes(),
        )
        .unwrap();
    let mut page = opening(ContextId::Page);
    claim(&mut page, &["Enter"]);
    container
        .open_with(
            page,
            script(
                r#"
                equal("the channel's globals are not here", typeof channelOnly, "undefined");
                equal("the channel's prototypes are not here", [].poisoned, undefined);
                nt.addEventListener("keydown", (e) => {
                  console.log("page keydown " + e.key);
                  e.stopPropagation(); e.stopImmediatePropagation();
                  nt.shared.set("score", { n: 3, list: [1, "two"] });
                  equal("read back", JSON.stringify(nt.shared.get("score")), '{"n":3,"list":[1,"two"]}');
                  check("a fresh copy each time", nt.shared.get("score") !== nt.shared.get("score"));
                  equal("keys", nt.shared.keys().join(), "score");
                  throws("not JSON", TypeError, "a shared value", () => nt.shared.set("f", () => 1));
                  throws("key bound", RangeError, "MAX_SHARED_KEY_BYTES: ", () => nt.shared.set("é".repeat(33), 1));
                  throws("value bound", RangeError, "MAX_SHARED_VALUE_BYTES: ", () => nt.shared.set("big", "x".repeat(16384)));
                  equal("missing", nt.shared.get("missing"), undefined);
                  done();
                });
                nt.addEventListener("sharedchange", () => console.error("FAIL the writer was told of its own change"));
                "#,
            )
            .as_bytes(),
        )
        .unwrap();
    let show = text_event(EventKind::PageShow, [0x100, 0, 1, 0], "https://example.test/chan/100.tti");
    let hide = plain_event(EventKind::PageHide, [0x100, 0, 0, 0]);
    let batch = container.step(&with_events(1, 0, vec![show, key(EventKind::KeyDown, "Enter", 0), key(EventKind::KeyDown, "a", 0), hide])).unwrap();
    let log = lines(&batch);
    let order: Vec<&str> = log.iter().map(String::as_str).filter(|line| line.starts_with("page ") || line.starts_with("channel ")).collect();
    assert_eq!(
        order,
        vec![
            "channel pageshow 100 null teletext 100",
            "page keydown Enter",
            // In the same step, after the callback that made the change.
            "channel sharedchange score={\"n\":3,\"list\":[1,\"two\"]}",
            // The page stopped propagation; the channel context still hears it, with no target.
            "channel keydown Enter null",
            // The page did not claim "a"; the channel did, and cancels it.
            "channel keydown a null",
            "channel pagehide 100",
        ]
    );
    // Enter: heard, not cancelled. "a": cancelled by the channel context.
    assert_eq!(view(&batch).verdicts, vec![2, 2, 3, 2]);
    assert_checks(&log, 12);
}

/// Section 3.1's table: a page context ends with one handling of `close`,
/// its timers and listeners go with it, its realm is freed, and the next
/// page starts with nothing carried across; the channel context lives on.
#[test]
fn a_page_context_ends_and_another_begins_with_nothing_carried() {
    let mut container = container();
    let mut channel = opening(ContextId::Channel);
    claim(&mut channel, &[]);
    container.open_with(channel, b"let ticks = 0; nt.addEventListener('tick', () => { if (++ticks === 5) console.log('channel still ticking'); });").unwrap();
    let mut first = opening(ContextId::Page);
    claim(&mut first, &[]);
    container
        .open_with(
            first,
            br#"globalThis.big = new Uint8Array(8 * 1024 * 1024).fill(1);
            globalThis.carried = "from the first page";
            setInterval(() => console.error("FAIL a closed page's timer fired"), 2);
            nt.addEventListener("tick", () => console.error("FAIL a closed page's listener ran"));
            let closes = 0;
            nt.addEventListener("close", (e) => { console.log("closing " + (++closes) + " " + e.cancelable); nt.shared.set("left", nt.page.number); });"#,
        )
        .unwrap();
    assert!(matches!(container.open_with(opening(ContextId::Page), b";"), Err(neoteletext_script::LinkError::Context(_))), "a context opened twice");
    let (_, heap_with_page) = container.memory();
    assert!(heap_with_page > 8 * 1024 * 1024, "the page's 8 MiB are not in the heap: {heap_with_page}");
    container.close(ContextId::Page);
    assert!(!container.is_open(ContextId::Page) && container.is_open(ContextId::Channel));
    let (_, heap_after) = container.memory();
    assert!(heap_after < heap_with_page - 8 * 1024 * 1024 + 64 * 1024, "the realm was not freed: {heap_with_page} then {heap_after}");
    let mut second = opening(ContextId::Page);
    second.page = 0x200;
    second.seed = [7; 32];
    container
        .open_with(
            second,
            b"console.log('second page sees ' + typeof carried + ' ' + typeof big + ' ' + nt.page.number + ' ' + nt.now() + ' ' + nt.shared.get('left'));",
        )
        .unwrap();
    let log = all_lines(&run(&mut container, 0, 8));
    assert_eq!(log.iter().filter(|line| line.contains("FAIL")).count(), 0, "{log:#?}");
    assert_eq!(log.iter().filter(|line| line.starts_with("closing")).collect::<Vec<_>>(), vec!["closing 1 false"]);
    assert!(log.contains(&"second page sees undefined undefined 200 0 100".to_owned()), "{log:#?}");
    assert!(log.contains(&"channel still ticking".to_owned()), "{log:#?}");
}

/// Sections 3.1, 3.8 and 4.2, and departure D26: what is thrown and not
/// caught is logged with its stack and counted; `error` and
/// `unhandledrejection` are raised on `globalThis`; changes made before a
/// throw stand; sixteen failures in a row stop the context alone.
#[test]
fn failures_are_logged_counted_and_stop_a_context_after_sixteen() {
    let mut container = container();
    let mut channel = opening(ContextId::Channel);
    claim(&mut channel, &[]);
    container.open_with(channel, b"let n = 0; nt.addEventListener('tick', () => { n++; if (n === 30) console.log('channel alive'); });").unwrap();
    let mut page = opening(ContextId::Page);
    claim(&mut page, &[]);
    container
        .open_with(
            page,
            br#"let n = 0;
            addEventListener("error", (e) => { if (n === 2) { console.log("error event: " + e.message.split("\n")[0] + " " + (e.error instanceof RangeError)); e.preventDefault(); } });
            addEventListener("unhandledrejection", (e) => { console.log("rejection event: " + e.reason.message + " " + (e.promise instanceof Promise)); });
            nt.addEventListener("tick", () => {
              n++;
              if (n === 1) { nt.page.title = "set before the throw"; null.property; }
              if (n === 2) throw new RangeError("quiet");
              if (n === 3) Promise.reject(new Error("nobody caught this"));
              if (n === 4) { const p = Promise.reject(new Error("caught later")); p.catch(() => console.log("caught")); }
              if (n === 5) reportError(new Error("reported"));
              if (n >= 7) throw new Error("again " + n);
            });
            throw new Error("at the top level");"#,
        )
        .unwrap();
    let batches = run(&mut container, 0, 40);
    let log = all_lines(&batches);
    let has = |needle: &str| log.iter().any(|line| line.contains(needle));
    assert!(has("uncaught: Error: at the top level"), "{log:#?}");
    assert!(log.iter().any(|line| line.contains("at the top level") && line.contains("page.js")), "the stack names the script: {log:#?}");
    assert!(has("uncaught: TypeError: cannot read property 'property' of null"), "{log:#?}");
    // Changes made before the throw stand, as on the web.
    assert_eq!(batches[0].as_ref().unwrap().title.as_deref(), Some("set before the throw"));
    assert!(has("error event: RangeError: quiet true") && !has("uncaught: RangeError: quiet"), "{log:#?}");
    assert!(has("rejection event: nobody caught this true") && has("unhandled rejection: Error: nobody caught this"), "{log:#?}");
    assert!(has("caught") && !has("caught later"), "{log:#?}");
    assert!(has("uncaught: Error: reported"), "{log:#?}");
    // Tick 6 succeeded, so the count began again at tick 7: sixteen
    // failures later, at tick 22, the page context is stopped.
    assert!(has("uncaught: Error: again 22") && !has("again 23"), "{log:#?}");
    assert!(has("MAX_CONSECUTIVE_FAILURES"), "{log:#?}");
    let last = batches.last().unwrap().as_ref().unwrap();
    let states = view(last).states;
    let state = |context| states.iter().find(|state| state.context == context).unwrap();
    assert_eq!((state(ContextId::Page).stopped, state(ContextId::Page).failures), (Some(FailReason::Failures), 16));
    assert_eq!(state(ContextId::Channel).stopped, None);
    assert!(has("channel alive"), "{log:#?}");
    assert!(container.stopped().is_none(), "a context's failures stopped the whole container");
}

/// Section 3.3: everything made in a scope goes when the scope ends.
#[test]
fn a_scope_ends_with_its_listeners_and_timers() {
    let mut container = page(&script(
        r#"
        let heard = 0, fired = 0, outer = 0;
        nt.addEventListener("tick", () => outer++);
        const scope = nt.scope(() => {
          nt.addEventListener("tick", () => heard++);
          setInterval(() => fired++, 1);
          nt.scope(() => { nt.addEventListener("tick", () => heard += 100); }).end();
        });
        throws("a function", TypeError, "scope takes a function", () => nt.scope(1));
        setTimeout(() => { scope.end(); scope.end(); }, 50);
        setTimeout(() => {
          equal("the scope's listener stopped", heard, 3);
          equal("the scope's timer stopped", fired, 3);
          equal("what was outside it did not", outer, 9);
          equal("timers left", nt.budget().timers.used, 0);
          let depth = 0;
          const nest = () => { depth++; nt.scope(nest); };
          throws("depth", RangeError, "MAX_SCOPE_DEPTH: ", nest);
          equal("depth reached", depth, 9);
          done();
        }, 150);
        "#,
    ));
    assert_checks(&all_lines(&run(&mut container, 0, 12)), 7);
}

/// Section 3.3's standard globals: `crypto`, `structuredClone` (departure
/// D17), `console`, and the budget report (section 3.7).
#[test]
fn the_standard_globals_work_within_their_bounds() {
    let mut container = page(&script(
        r#"
        const bytes = new Uint8Array(64);
        equal("returns its array", crypto.getRandomValues(bytes), bytes);
        check("filled", bytes.some((byte) => byte !== 0));
        const words = new Uint32Array(4);
        crypto.getRandomValues(words);
        check("words filled", words.some((word) => word > 0xffff));
        const view = new Uint8Array(new ArrayBuffer(16), 4, 4);
        crypto.getRandomValues(view);
        check("only the view is written", new Uint8Array(view.buffer).every((byte, index) => (index >= 4 && index < 8) || byte === 0));
        try { crypto.getRandomValues(new Float32Array(4)); check("float refused", false); } catch (error) { equal("float refused", error.name, "TypeMismatchError"); }
        try { crypto.getRandomValues(new Uint8Array(65537)); check("quota", false); } catch (error) { equal("quota", error.name, "QuotaExceededError"); }
        equal("the quota itself is allowed", crypto.getRandomValues(new Uint8Array(65536)).length, 65536);
        const uuid = crypto.randomUUID();
        check("uuid " + uuid, /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/.test(uuid));
        check("uuids differ", uuid !== crypto.randomUUID());
        const original = { list: [1, { deep: true }], when: new Date(5), map: new Map([["k", new Set([1])]]), bytes: new Uint8Array([1, 2, 3]) };
        original.self = original;
        const copy = structuredClone(original);
        check("a copy", copy !== original && copy.list !== original.list && copy.list[1] !== original.list[1]);
        check("cycles kept", copy.self === copy);
        check("values kept", copy.list[1].deep === true && copy.when.getTime() === 5 && copy.map.get("k").has(1) && copy.bytes[2] === 3 && copy.bytes.buffer !== original.bytes.buffer);
        for (const bad of [() => 1, Symbol("s"), new (class Thing {})(), /x/]) {
          try { structuredClone(bad); check("refused", false); } catch (error) { equal("refused", error.name, "DataCloneError"); }
        }
        try { structuredClone(1, { transfer: [new ArrayBuffer(1)] }); check("transfer", false); } catch (error) { equal("transfer", error.name, "DataCloneError"); }
        console.debug("d", 1); console.info("i", { a: 1 }); console.warn("w", [1, 2]); console.error("e", null, undefined); nt.log("l", new Error("in a log").message);
        console.log("x".repeat(1000));
        const budget = nt.budget();
        equal("budget entries", Object.keys(budget).join(), "scriptBytes,heapBytes,timers,listeners,requests,sockets,soundOperations,callbackPercent,callbackPercentMax,stepPercentMax,secondPercent,batchBytes,hostBytes,failures,triplets,colours,controls,sprites,spriteFrames,objectPageTriplets,bitmapCellsLossy");
        check("heap is counted", budget.heapBytes.used > 100000 && budget.heapBytes.limit === 48 * 1024 * 1024);
        check("script bytes", budget.scriptBytes.used > 1000 && budget.scriptBytes.limit === 1048576);
        check("host bytes", budget.hostBytes.used > 0 && budget.hostBytes.limit === 8388608);
        check("a fresh object each call", nt.budget() !== nt.budget());
        done();
        "#,
    ));
    let batch = container.step(&tick(1, 0)).unwrap();
    assert_checks(&lines(&batch), 22);
    let log = view(&batch).log;
    let logged: Vec<(u8, &str)> = log.iter().map(|line| (line.level, line.text.as_str())).collect();
    assert_eq!(&logged[..5], &[(0, "d 1"), (2, "i {\"a\":1}"), (3, "w [1,2]"), (4, "e null undefined"), (1, "l in a log")]);
    assert_eq!(logged[5].1.len(), 400, "a log line is cut to MAX_LOG_CHARS");
}

/// Departure D15 and the security review's finding S-13 (TODO-7279): two
/// contexts started in the same second have different `Math.random`
/// sequences, because each is seeded from its own seed and not from the
/// time; the same seed gives the same sequence (the runner's repeatable
/// runs); and `crypto` is not `Math.random`.
#[test]
fn math_random_follows_the_seed_and_not_the_second() {
    let sequence = |seed: [u8; 32]| {
        let mut container = container();
        let mut open = opening(ContextId::Page);
        open.seed = seed;
        container.open_with(open, b"console.log([Math.random(), Math.random(), Math.random()].join()); console.log(Array.from(crypto.getRandomValues(new Uint8Array(8))).join());").unwrap();
        lines(&container.step(&tick(1, 0)).unwrap())
    };
    let (first, same_second, same_seed) = (sequence([1; 32]), sequence([2; 32]), sequence([1; 32]));
    assert_ne!(first[0], same_second[0], "two contexts started in the same second share Math.random");
    assert_ne!(first[1], same_second[1]);
    assert_eq!(first, same_seed);
    let numbers: Vec<f64> = first[0].split(',').map(|number| number.parse().unwrap()).collect();
    assert!(numbers.iter().all(|number| (0.0..1.0).contains(number)) && (numbers[0] - numbers[1]).abs() > f64::EPSILON);
    // The two contexts of one container have different seeds too.
    let mut container = container();
    let mut channel = opening(ContextId::Channel);
    channel.seed = [3; 32];
    container.open_with(channel, b"console.log(Math.random())").unwrap();
    let mut page = opening(ContextId::Page);
    page.seed = [4; 32];
    container.open_with(page, b"console.log(Math.random())").unwrap();
    let both = lines(&container.step(&tick(1, 0)).unwrap());
    assert_ne!(both[0], both[1]);
}

/// Sections 3.3 and 4.2: `nt.navigate` raises `navigate` in its own
/// context first, which may cancel it; a navigation the browser's own
/// controls caused cannot be cancelled by any script.
#[test]
fn navigation_is_heard_first_and_the_browsers_own_cannot_be_cancelled() {
    let mut container = page(&script(
        r#"
        const heard = [];
        nt.addEventListener("navigate", (e) => {
          heard.push([e.cause, e.cancelable, e.destination.number, e.destination.subpage, e.destination.url].join());
          if (e.destination.number === "300" || e.cause === "browser" || e.cause === "link") e.preventDefault();
        });
        nt.navigate("300");
        throws("a string", TypeError, "a navigation's target is a string", () => nt.navigate(100));
        throws("scheme", TypeError, "a navigation's target is a page number", () => nt.navigate("javascript:alert(1)"));
        nt.navigate("1a0/00ff");
        // The refused one was counted: this is the third in a second.
        throws("rate", RangeError, "MAX_NAVIGATIONS_PER_SECOND: ", () => nt.navigate("200"));
        setTimeout(() => {
          equal("heard", heard.join(" | "), "script,true,300,, | script,true,,,javascript:alert(1) | script,true,1A0,00FF, | script,true,200,, | browser,false,100,, | link,true,,,https://example.test/chan/x");
          done();
        }, 30);
        "#,
    ));
    // The cancelled navigation to 300 left nothing; the last allowed one is staged.
    let first = container.step(&tick(1, 0)).unwrap();
    assert_eq!(first.navigate.as_deref(), Some("1a0/00ff"));
    let navigate = |cause: NavigateCause, to: &str| text_event(EventKind::Navigate, [cause.number(), 0, 0, 0], to);
    let batch = container
        .step(&with_events(2, 1, vec![navigate(NavigateCause::BROWSER, "100"), navigate(NavigateCause::LINK, "https://example.test/chan/x")]))
        .unwrap();
    // The script called preventDefault on both; only the link's counts.
    assert_eq!(view(&batch).verdicts, vec![2, 3]);
    let mut log = lines(&first);
    log.extend(all_lines(&run(&mut container, 2, 5)));
    assert_checks(&log, 4);
}

/// Section 3.5: sound and synthesizers leave as the batch's typed
/// operations; nothing sounds before the viewer has acted, and then an
/// `allowed` event says so; a hidden tab's contexts are told, and their
/// sound released (section 3.1's table).
#[test]
fn sound_leaves_as_typed_operations_and_is_released_when_hidden() {
    let mut container = container();
    let mut open = opening(ContextId::Page);
    open.flags.sound_allowed = false;
    claim(&mut open, &[]);
    container
        .open_with(
            open,
            script(
                r#"
                equal("not yet allowed", nt.sound.allowed, false);
                nt.sound.play(1);
                nt.sound.addEventListener("allowed", () => {
                  equal("allowed", nt.sound.allowed, true);
                  nt.sound.play(2, { voice: 3, gain: 40 });
                  nt.synth.start(1, "dx7", { variant: 2 });
                  nt.synth.note(1, 60, { velocity: 90, ticks: 2 });
                  nt.synth.noteOn(1, 64, 80); nt.synth.noteOff(1, 64); nt.synth.control(1, 7, 100); nt.synth.program(1, 5); nt.synth.bend(1, 8192);
                  throws("family", TypeError, "family is one of sid, dx7, juno, synthd, synth53, synth8, synth80", () => nt.synth.start(0, "theremin"));
                  throws("voice", RangeError, "AUDIO_CHANNELS: ", () => nt.sound.stop(4));
                  throws("unannounced", TypeError, "the channel announced no such sound", () => nt.sound.play(9));
                  throws("bend", RangeError, "MAX_MIDI_BEND: ", () => nt.synth.bend(1, 16384));
                  throws("whole", TypeError, "voice is a whole number", () => nt.sound.stop(1.5));
                });
                nt.sound.addEventListener("cue", (e) => console.log("cue " + e.voice + " " + e.cue));
                nt.sound.addEventListener("ended", (e) => console.log("ended " + e.voice));
                nt.addEventListener("visibilitychange", () => { console.log("hidden " + nt.hidden); if (!nt.hidden) done(); });
                nt.addEventListener("fault", (e) => console.log("fault " + e.code + " " + e.bound));
                "#,
            )
            .as_bytes(),
        )
        .unwrap();
    let quiet = container.step(&Step { sequence: 1, tick: 0, flags: StepFlags { tick: true, ..StepFlags::default() }, ..Step::default() }).unwrap();
    assert_eq!(view(&quiet).sound, Vec::new());
    assert_eq!(view(&quiet).log.iter().filter(|line| line.text.contains("sound does nothing until")).count(), 1);
    let batch = container.step(&tick(2, 1)).unwrap();
    assert_eq!(
        view(&batch).sound,
        vec![
            SoundOperation::Play { voice: 3, sound: 2, gain: 40 },
            SoundOperation::SynthStart { part: 1, family: 13, variant: 2, gain: 100 },
            SoundOperation::Midi { part: 1, status: 0x90, data1: 60, data2: 90 },
            SoundOperation::Midi { part: 1, status: 0x90, data1: 64, data2: 80 },
            SoundOperation::Midi { part: 1, status: 0x80, data1: 64, data2: 0 },
            SoundOperation::Midi { part: 1, status: 0xB0, data1: 7, data2: 100 },
            SoundOperation::Midi { part: 1, status: 0xC0, data1: 5, data2: 0 },
            SoundOperation::Midi { part: 1, status: 0xE0, data1: 0, data2: 64 },
        ]
    );
    // The held note is released by the host on its tick.
    assert_eq!(view(&batch).states[0].next_due, Some(3));
    assert_eq!(container.step(&tick(3, 2)).unwrap().sound, None);
    let reports = Step {
        sound: vec![SoundReport::Cue { voice: 3, cue: 9 }, SoundReport::Ended { voice: 3 }],
        faults: vec![FaultReport { code: 4, bound: "MAX_OBJECTS".into() }],
        ..tick(4, 3)
    };
    let released = container.step(&reports).unwrap();
    assert_eq!(view(&released).sound, vec![SoundOperation::Midi { part: 1, status: 0x80, data1: 60, data2: 0 }]);
    assert_eq!(lines(&released), vec!["cue 3 9", "ended 3", "fault 4 MAX_OBJECTS"]);
    // Hidden: told first, then everything it plays is stopped.
    let hidden = container.step(&Step { flags: StepFlags { pausing: true, ..tick(5, 4).flags }, ..tick(5, 4) }).unwrap();
    assert_eq!(lines(&hidden), vec!["hidden true"]);
    assert_eq!(view(&hidden).sound, vec![SoundOperation::SynthStop { part: 1 }, SoundOperation::Stop { voice: 3 }]);
    let shown = container.step(&Step { flags: StepFlags { resumed: true, ..tick(6, 5).flags }, ..tick(6, 5) }).unwrap();
    assert_checks(&lines(&shown), 7);
    assert!(lines(&shown).contains(&"hidden false".to_owned()));
}
