// Section 8 through nt.ui: each call's result and each refusal's kind and bound (tests/ui.rs).
const ui = nt.ui;
check("frozen", Object.isFrozen(ui) && typeof ui.menu === "function");
equal("thirteen kinds and keys", Object.keys(ui).sort().join(), "band,button,clock,field,frame,keys,link,menu,paragraph,progress,slider,table,text,toggle");
equal("a menu's keys", ui.keys("menu").join(), "ArrowUp,ArrowDown,Home,End,Enter");
equal("a field's keys", ui.keys("field").join(), "Enter,Escape,Backspace,ArrowLeft,ArrowRight");
equal("a band's keys", ui.keys("band").length, 0);
throws("no such kind", TypeError, "no such kind of component", () => ui.keys("carousel"));

// Placement: the grid of 40 columns by rows 1 to 23, on every renderer.
throws("no at", TypeError, "a component has at:", () => ui.text({ text: "x" }));
throws("at type", TypeError, "at.row is a whole number", () => ui.text({ at: { row: "1", column: 0, columns: 5 }, text: "x" }));
throws("row 0", RangeError, "UI_GRID_ROWS: a component lies on rows 1 to 23", () => ui.text({ at: { row: 0, column: 0, columns: 5 }, text: "x" }));
throws("rows past 23", RangeError, "UI_GRID_ROWS: ", () => ui.text({ at: { row: 23, column: 0, columns: 5, rows: 2 }, text: "x" }));
throws("columns past 39", RangeError, "UI_GRID_COLUMNS: a component lies on columns 0 to 39; 6 columns from column 35 do not", () => ui.text({ at: { row: 1, column: 35, columns: 6 }, text: "x" }));

// Options: strict, by kind.
throws("an option of another kind", TypeError, "a text has no option \"items\"", () => ui.text({ at: { row: 1, column: 0, columns: 5 }, items: [] }));
throws("a wrong type", TypeError, "text is a string", () => ui.text({ at: { row: 1, column: 0, columns: 5 }, text: 5 }));
throws("a fraction", TypeError, "a component's options hold a number that is not a whole number", () => ui.slider({ at: { row: 1, column: 0, columns: 5 }, value: 1.5 }));
throws("an item's member", TypeError, "a menu's item has no member \"colour\"", () => ui.menu({ at: { row: 1, column: 0, columns: 9 }, items: [{ text: "a", colour: 1 }] }));
throws("a colour's name", TypeError, "ink is an entry of the renderer's colours or one of black, red", () => ui.text({ at: { row: 1, column: 0, columns: 5 }, text: "x", ink: "amber" }));
throws("an id's length", RangeError, "MAX_UI_ID_BYTES: ", () => ui.text({ at: { row: 1, column: 0, columns: 5 }, text: "x", id: "i".repeat(65) }));
throws("a control character", TypeError, "text holds no control character", () => ui.text({ at: { row: 1, column: 0, columns: 5 }, text: "a\nb" }));
throws("min and max", RangeError, "MAX_UI_NUMBER: max, 1, is greater than min, 1", () => ui.slider({ at: { row: 1, column: 0, columns: 9 }, min: 1, max: 1 }));
throws("a field's longest", RangeError, "MAX_UI_FIELD_CHARS: the field's value is at most its longest, 3 characters", () => ui.field({ at: { row: 1, column: 0, columns: 9 }, value: "abcd", longest: 3 }));
throws("a row's cells", TypeError, "a table's row has no more cells than the table has columns", () => ui.table({ at: { row: 1, column: 0, columns: 9, rows: 2 }, columns: [{ title: "a", width: 3 }], rows: [["1", "2"]] }));
throws("a format", TypeError, "format is \"HH:MM\" or \"HH:MM:SS\"", () => ui.clock({ at: { row: 1, column: 0, columns: 9 }, format: "H" }));
equal("nothing was made", nt.budget().components.used, 0);

// Teletext's own limits, by teletext's names.
throws("the attribute cell", RangeError, "TELETEXT_ATTRIBUTE_CELL: a row of a component begins with the cell of its colour attribute, so a text is at least 2 columns wide", () => ui.text({ at: { row: 1, column: 0, columns: 1 }, text: "x" }));
throws("one size of character", RangeError, "COLUMNS: the text is 6 characters and the component shows 5 after its attribute cells; teletext has one size of character", () => ui.text({ at: { row: 1, column: 0, columns: 6 }, text: "HARBOR" }));
throws("a spacing colour", RangeError, "COLOURS: on teletext a component's ink is a Level 1 colour, 0 to 7", () => ui.text({ at: { row: 1, column: 0, columns: 6 }, text: "x", ink: 16 }));
throws("a Colour Map entry", RangeError, "COLOUR_MAP_ENTRIES: ", () => ui.text({ at: { row: 1, column: 0, columns: 6 }, text: "x", paper: 32 }));
throws("double height's rows", RangeError, "TELETEXT_ROWS: a text of double height takes 2 rows, and the component has 1", () => ui.text({ at: { row: 3, column: 0, columns: 9 }, text: "x", height: 2 }));
throws("a frame's rows", RangeError, "TELETEXT_ROWS: a frame's title, rule, hint and prompts takes 4 rows", () => ui.frame({ at: { row: 1, column: 0, columns: 40, rows: 3 }, title: "t", hint: "h", prompts: ["a"] }));
throws("a toggle's width", RangeError, "COLUMNS: a toggle is at least 6 columns wide", () => ui.toggle({ at: { row: 1, column: 0, columns: 5 }, text: "x" }));
throws("a table's columns", RangeError, "COLUMNS: the table's columns take 12 cells with the cell of the row's colour, and the component has 11", () => ui.table({ at: { row: 1, column: 0, columns: 11, rows: 2 }, columns: [{ title: "a", width: 11 }] }));
throws("an unshown character", TypeError, "the page's character sets cannot show", () => ui.text({ at: { row: 1, column: 0, columns: 9 }, text: "☃" }));
equal("still nothing was made", JSON.stringify([nt.budget().components.used, nt.budget().triplets.used, nt.budget().controls.used]), "[0,0,0]");

// A component: set, value, focus, remove.
const cut = ui.text({ id: "cut", at: { row: 1, column: 0, columns: 6 }, text: "HARBOUR", fit: "cut" });
equal("cut to its cells", String.fromCharCode(nt.teletext.page.at(1, 1), nt.teletext.page.at(1, 5)), "HO");
equal("the colour's cell", nt.teletext.page.at(1, 0), 7);
check("an event target with an id and a kind", cut instanceof EventTarget && cut.id === "cut" && cut.kind === "text" && cut.value === null);
equal("set chains", cut.set({ text: "TIDE", ink: "cyan", id: "tide" }), cut);
equal("drawn again", [nt.teletext.page.at(1, 0), nt.teletext.page.at(1, 4), nt.teletext.page.at(1, 5), cut.id].join(), "6,69,32,tide");
throws("set is checked too", TypeError, "a text has no option \"on\"", () => cut.set({ on: true }));
throws("and changes nothing", RangeError, "COLUMNS: ", () => cut.set({ text: "LONGER TEXT", fit: "refuse" }));
equal("as it was", nt.teletext.page.at(1, 4), 69);
cut.set({ at: { row: 2, column: 3, columns: 6 } });
equal("moved", [nt.teletext.page.at(1, 1), nt.teletext.page.at(2, 4)].join(), "32,84");
throws("the focus rests on what takes keys", TypeError, "the focus rests only on an enabled component that takes keys", () => cut.focus());
const menu = ui.menu({ at: { row: 4, column: 0, columns: 10, rows: 2 }, items: [{ text: "a", value: "A" }, { text: "b" }, { text: "c" }], selected: 2 });
equal("a menu's value is its item's", JSON.stringify([menu.value, nt.budget().uiNodes.used, nt.budget().controls.used]), '["c"===0?0:2,3,2]'.replace('"c"===0?0:2', "2"));
menu.set({ selected: 0 });
equal("its own value", menu.value, "A");
const off = ui.toggle({ at: { row: 7, column: 0, columns: 10 }, text: "x", disabled: true });
throws("a disabled component takes no focus", TypeError, "the focus rests only", () => off.focus());
equal("and no control", nt.budget().controls.used, 2);
off.set({ disabled: false });
off.focus();
equal("a control each", nt.budget().controls.used, 3);
const before = nt.budget().triplets.used;
menu.remove();
menu.remove();
throws("removed", TypeError, "the component was removed", () => menu.set({ selected: 1 }));
check("its triplets and controls are free", nt.budget().triplets.used < before && nt.budget().controls.used === 1 && nt.teletext.page.at(4, 1) === 32);
const scope = nt.scope(() => { ui.band({ at: { row: 9, column: 0, columns: 40 }, text: "scoped" }); });
equal("a scope's component", nt.budget().components.used, 3);
scope.end();
equal("gone with the scope", [nt.budget().components.used, nt.teletext.page.at(9, 18)].join(), "2,32");
done();
