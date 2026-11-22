// Section 5.3 through nt.teletext: each call's result and each refusal's kind and bound, as JavaScript is given them (tests/teletext_sprites.rs).
const tt = nt.teletext;
check("frozen", Object.isFrozen(tt) && typeof tt.block === "function" && typeof tt.sprite === "function");

// Blocks.
const hull = tt.block(3, 2);
equal("a block's size", hull.columns * 10 + hull.rows, 32);
equal("sixels chains", hull.sixels(["011110", "111111", "111111", "100001", "100001", "000000"], { ink: 7 }), hull);
throws("block columns", RangeError, "COLUMNS: a block is 1 to 40 columns", () => tt.block(41, 1));
throws("block rows", RangeError, "TELETEXT_ROWS: a block is 1 to 23 rows", () => tt.block(1, 24));
throws("block size type", TypeError, "columns is a whole number", () => tt.block(1.5, 1));
throws("sixel lines", TypeError, "a block of 2 rows is drawn by 6 lines", () => hull.sixels(["011110"]));
throws("sixel characters", TypeError, "a line of sixels for a block 3 wide is 6 characters", () => hull.sixels(["01111x", "", "", "", "", ""]));
throws("cell row", RangeError, "TELETEXT_ROWS: a row of this block is 0 to 1", () => hull.cell(2, 0, { sixels: 1 }));
throws("cell column", RangeError, "COLUMNS: a column of this block is 0 to 2", () => hull.cell(0, 3, { sixels: 1 }));
throws("cell sixels", RangeError, "MOSAIC_PATTERNS: ", () => hull.cell(0, 0, { sixels: 64 }));
throws("cell both", TypeError, "a cell is a character or a mosaic", () => hull.cell(0, 0, { sixels: 1, char: "a" }));
throws("cell char", TypeError, "char is one character", () => hull.cell(0, 0, { char: "ab" }));
throws("cell ink", RangeError, "COLOUR_MAP_ENTRIES: ", () => hull.cell(0, 0, { sixels: 1, ink: -1 }));
throws("text too wide", RangeError, "COLUMNS: the text takes 4 cells from column 0 of a block 3 wide", () => hull.text(0, 0, "abcd"));
throws("text unshown", TypeError, "the page's character sets cannot show", () => hull.text(0, 0, "☃"));
throws("not a block", TypeError, "not a block", () => tt.sprite({ frames: [{}], row: 1, column: 1 }));
const word = tt.block(4, 1).text(0, 0, "café", { ink: 3, paper: 4 });
const flag = tt.block(2, 2).cell(0, 0, { char: "A", ink: 1 }).cell(0, 1, { sixels: 63, separated: true, ink: 2 }).cell(1, 0, { paper: 6 });

// Sprites.
const boat = tt.sprite({ frames: [hull], row: 21, column: 18, label: "Boat" });
equal("defaults", [boat.row, boat.column, boat.layer, boat.order, boat.shown, boat.detectable, boat.wheel, boat.frame, boat.frames, boat.label].join(), "21,18,2,0,true,false,false,0,1,Boat");
check("a sprite is a thing", boat instanceof EventTarget && Object.isFrozen(Object.getPrototypeOf(boat)));
equal("bounds", JSON.stringify(boat.bounds()), '{"x":18,"y":21,"width":3,"height":2}');
boat.moveBy(0, -1);
boat.moveTo(20, 5);
boat.row = 19;
boat.column += 2;
equal("moved", [boat.row, boat.column].join(), "19,7");
throws("column reach", RangeError, "MAX_SPRITE_COLUMN: a sprite's column is -39 to 39, not 40", () => boat.moveTo(1, 40));
throws("row reach", RangeError, "MAX_SPRITE_ROW: a sprite's row is -24 to 24, not -25", () => boat.moveBy(-44, 0));
equal("a refused move moves nothing", [boat.row, boat.column].join(), "19,7");
boat.moveTo(-1, -2);
equal("cut to the page", JSON.stringify(boat.bounds()), '{"x":0,"y":0,"width":1,"height":1}');
boat.moveTo(24, 39);
equal("a rectangle on row 24", JSON.stringify(boat.bounds()), '{"x":39,"y":24,"width":1,"height":1}');
boat.moveTo(-24, 0);
equal("wholly off the page", boat.bounds(), null);
boat.moveTo(19, 7);
boat.shown = false;
equal("hidden has no bounds", boat.bounds(), null);
boat.shown = true;
boat.layer = 3;
boat.order = -128;
equal("ordered", [boat.layer, boat.order].join(), "3,-128");
throws("layer 0", RangeError, "LAST_LAYER: a sprite's layer is 1 to 3, not 0", () => { boat.layer = 0; });
throws("layer 4", RangeError, "LAST_LAYER: ", () => tt.sprite({ frames: [hull], row: 1, column: 1, layer: 4 }));
throws("order", RangeError, "SPRITE_ORDER: a sprite's order is -128 to 127, not 128", () => { boat.order = 128; });
throws("shown type", TypeError, "shown is true or false", () => { boat.shown = 1; });
throws("frame beyond", RangeError, "MAX_OBJECT_FRAMES: a frame of this sprite is 0 to 0, not 1", () => { boat.frame = 1; });
throws("no row", TypeError, "a sprite has a row and a column", () => tt.sprite({ frames: [hull] }));
throws("no frames", TypeError, "frames is an array of blocks", () => tt.sprite({ frames: [], row: 1, column: 1 }));
throws("seventeen frames", RangeError, "MAX_OBJECT_FRAMES: a sprite has at most 16 frames, not 17", () => tt.sprite({ frames: new Array(17).fill(hull), row: 1, column: 1 }));
throws("frames of two sizes", TypeError, "the frames of a sprite are blocks of one size", () => tt.sprite({ frames: [hull, word], row: 1, column: 1 }));
throws("label bytes", RangeError, "MAX_LABEL_BYTES: ", () => { boat.label = "x".repeat(65); });

// A frame that needs more than an object holds: 23 rows of 40 cells.
const big = tt.block(40, 23);
for (let row = 0; row < 23; row++) for (let column = 0; column < 40; column++) big.cell(row, column, { sixels: 1 + ((row + column) & 31), ink: column & 31 });
const held = () => { const budget = nt.budget(); return JSON.stringify([budget.sprites, budget.spriteFrames, budget.objectPageTriplets]); };
const before = held();
throws("a frame over an object's triplets", RangeError, "OBJECT_BODY_TRIPLETS: a frame's object is at most 507 triplets", () => tt.sprite({ frames: [big], row: 1, column: 0 }));
equal("a refused sprite leaves nothing", held(), before);

// Frames and cycles.
const two = tt.block(2, 2).cell(0, 0, { sixels: 21 });
const flagship = tt.sprite({ frames: [flag, two, word.rows === 1 ? two : flag], row: 5, column: 5, detectable: true, wheel: true, shown: false, order: 3 });
flagship.frame = 2;
equal("frame", flagship.frame, 2);
flagship.cycle(0, 2, 10);
check("cycling", flagship.cycling === true && flagship.frame === 0);
throws("cycle order", RangeError, "MAX_OBJECT_FRAMES: a cycle's first frame, 2, is after its last, 1", () => flagship.cycle(2, 1, 10));
throws("cycle period", RangeError, "MAX_CYCLE_PERIOD: a cycle's period is 1 to 3600 ticks a frame, not 3601", () => flagship.cycle(0, 1, 3601));
throws("cycle period 0", RangeError, "MAX_CYCLE_PERIOD: ", () => flagship.cycle(0, 1, 0));
throws("cycle last", RangeError, "MAX_OBJECT_FRAMES: a frame of this sprite is 0 to 2, not 3", () => flagship.cycle(0, 3, 10));
flagship.stopCycle();
check("stopped", flagship.cycling === false);
flagship.cycle(1, 2, 4, { once: true });
check("overlaps needs both shown", flagship.overlaps(boat) === false);
flagship.shown = true;
boat.moveTo(6, 6);
check("overlaps", flagship.overlaps(boat) && boat.overlaps(flagship) && !boat.overlaps(null));

// The budget report (3.7).
const budget = nt.budget();
equal("sprites", JSON.stringify(budget.sprites), '{"used":2,"limit":32}');
equal("sprite frames", JSON.stringify(budget.spriteFrames), '{"used":3,"limit":512}');
equal("the Object page", budget.objectPageTriplets.limit, 8112);
check("sprites take no local triplet", budget.triplets.used === 0);

// Events: a sprite is heard, then nt.
let heard = "";
flagship.addEventListener("pointerup", () => { heard += "sprite "; });
nt.addEventListener("pointerup", (e) => { heard += "nt " + (e.target === flagship) + " " + e.things.length; console.log(heard); });
flagship.addEventListener("cycleend", () => { console.log("cycle ended at frame " + flagship.frame + ", cycling " + flagship.cycling); });

// Removal: the handle is dead, and removal twice is nothing.
const gone = tt.sprite({ frames: [two], row: 2, column: 2 });
gone.remove();
gone.remove();
throws("removed", TypeError, "the sprite was removed", () => gone.moveBy(1, 0));
equal("removed has no bounds", gone.bounds(), null);

// The page's share of the sprites: 28, the channel context's four kept.
const many = [];
for (let n = 2; n < 28; n++) many.push(tt.sprite({ frames: [two], row: 1, column: n }));
throws("the page's share", RangeError, "MAX_SPRITES: a page has at most 32 sprites, of which 4 are kept for the channel context", () => tt.sprite({ frames: [two], row: 1, column: 1 }));
equal("frames that are the same are one object", nt.budget().spriteFrames.used, 3);
for (const one of many) one.remove();
equal("released", nt.budget().sprites.used, 2);
done();
