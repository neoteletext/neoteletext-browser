// Section 8 through nt.ui on a ReGIS page: ReGIS's own refusals by ReGIS's names, each leaving nothing made, and a component set, moved and removed as drawings (tests/ui_regis.rs).
const ui = nt.ui;
const drawings = () => nt.budget().drawings.used;
check("frozen", Object.isFrozen(ui) && typeof ui.menu === "function");
equal("the vocabulary is the same", Object.keys(ui).sort().join(), "band,button,clock,field,frame,keys,link,menu,paragraph,progress,slider,table,text,toggle");
equal("a menu's keys", ui.keys("menu").join(), "ArrowUp,ArrowDown,Home,End,Enter");
throws("row 0", RangeError, "UI_GRID_ROWS: a component lies on rows 1 to 23", () => ui.text({ at: { row: 0, column: 0, columns: 5 }, text: "x" }));

// ReGIS's own limits, by ReGIS's names; each refusal leaves nothing.
throws("a text longer than its cells", RangeError, "UI_GRID_COLUMNS: the text takes 7 cells and the component has 6; fit: \"shrink\" draws it in narrower cells", () => ui.text({ at: { row: 1, column: 0, columns: 6 }, text: "HARBOUR" }));
throws("a colour-map entry", RangeError, "REGIS_COLOURS: on ReGIS a component's colour is one of the eight names or a colour-map entry, 8 to 15; not 16", () => ui.text({ at: { row: 1, column: 0, columns: 6 }, text: "x", ink: 16 }));
throws("a paper's entry", RangeError, "REGIS_COLOURS: ", () => ui.band({ at: { row: 1, column: 0, columns: 6 }, text: "x", paper: 31 }));
throws("double height's rows", RangeError, "UI_GRID_ROWS: a text of double height takes 2 rows, and the component has 1", () => ui.text({ at: { row: 3, column: 0, columns: 9 }, text: "x", height: 2 }));
throws("a frame's rows", RangeError, "UI_GRID_ROWS: a frame's title, rule, hint and prompts takes 4 rows", () => ui.frame({ at: { row: 1, column: 0, columns: 40, rows: 3 }, title: "t", hint: "h", prompts: ["a"] }));
throws("a toggle's width", RangeError, "UI_GRID_COLUMNS: a toggle is at least 3 columns wide: its switch takes 2", () => ui.toggle({ at: { row: 1, column: 0, columns: 2 }, text: "x" }));
throws("a table's columns", RangeError, "UI_GRID_COLUMNS: the table's columns take 12 cells, and the component has 11", () => ui.table({ at: { row: 1, column: 0, columns: 11, rows: 2 }, columns: [{ title: "a", width: 12 }] }));
throws("an unshown character", TypeError, "", () => ui.text({ at: { row: 1, column: 0, columns: 9 }, text: "☃" }));
throws("a label a ReGIS drawing cannot carry", TypeError, "a label holds only ISO 8859-1's printable characters", () => ui.button({ at: { row: 1, column: 0, columns: 9 }, text: "x", label: "Ж" }));
throws("a menu whose last row cannot be drawn", TypeError, "", () => ui.menu({ at: { row: 1, column: 0, columns: 9, rows: 2 }, items: [{ text: "a" }, { text: "b" }, { text: "☃" }] }));
equal("nothing was made, of any refused component", JSON.stringify([nt.budget().components.used, drawings(), nt.budget().contentBytes.used]), "[0,0,0]");

// No attribute cell: n columns show n characters; and a text is drawn narrower before it is cut.
const exact = ui.text({ id: "exact", at: { row: 1, column: 0, columns: 6 }, text: "HARBOR" });
equal("one drawing for a text", drawings(), 1);
const narrow = ui.text({ at: { row: 2, column: 0, columns: 6 }, text: "HARBOUR WALL", fit: "shrink" });
const cut = ui.text({ at: { row: 3, column: 0, columns: 6 }, text: "HARBOUR WALL AND MORE", fit: "shrink" });
const tall = ui.text({ at: { row: 4, column: 0, columns: 6, rows: 2 }, text: "TALL", height: 2 });
equal("each is one drawing", drawings(), 4);
check("an event target with an id and a kind", exact instanceof EventTarget && exact.id === "exact" && exact.kind === "text" && exact.value === null);
equal("set chains", exact.set({ text: "TIDE", ink: "cyan", id: "tide" }), exact);
throws("set is refused whole", RangeError, "UI_GRID_COLUMNS: ", () => exact.set({ text: "LONGER TEXT" }));
exact.set({ at: { row: 6, column: 3, columns: 6 } });
equal("still one drawing each", drawings(), 4);
for (const made of [narrow, cut, tall]) made.remove();
equal("a removed component's drawings are deleted", drawings(), 1);

// What each kind costs in drawings: the page's own budget says it.
const before = drawings();
const button = ui.button({ at: { row: 8, column: 0, columns: 10 }, text: "Go" });
equal("a button: itself, two looks, its text and its ring", drawings() - before, 5);
const off = ui.button({ at: { row: 9, column: 0, columns: 10 }, text: "No", disabled: true });
equal("a disabled button has no looks", drawings() - before, 5 + 3);
const menu = ui.menu({ at: { row: 10, column: 0, columns: 10, rows: 2 }, items: [{ text: "a", value: "A" }, { text: "b" }, { text: "c" }], selected: 2 });
equal("a menu: itself, four backgrounds, its group, its ring, and two a row", drawings() - before, 8 + 7 + 6);
equal("a menu's value is its item's", JSON.stringify([menu.value, nt.budget().uiNodes.used]), "[2,3]");
menu.set({ selected: 0 });
equal("its own value", menu.value, "A");
menu.set({ items: [{ text: "a" }] });
equal("rows no longer wanted are deleted", drawings() - before, 8 + 7 + 2);
throws("the focus rests on what takes keys", TypeError, "the focus rests only on an enabled component that takes keys", () => off.focus());
button.focus();

// The script's own drawings and the components' share the page's table, and neither is the other's.
const own = nt.regis.drawing({ x: 5 });
const all = drawings();
nt.regis.clear();
equal("clear deletes the script's drawings and leaves the components'", drawings(), all - 1);
throws("the script's drawing is gone", TypeError, "the drawing was removed", () => own.x);
menu.set({ selected: 0, items: [{ text: "still here" }, { text: "and here" }] });
equal("and the components go on", drawings(), all - 1 + 2);

// A scope's components go with it, drawings and all.
const scoped = drawings();
const scope = nt.scope(() => { ui.band({ at: { row: 20, column: 0, columns: 40 }, text: "scoped" }); });
equal("a scope's component", drawings(), scoped + 1);
scope.end();
equal("gone with the scope", drawings(), scoped);
done();
