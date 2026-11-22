// Sections 5.1 to 5.6 through nt.teletext: each call's result and each refusal's kind and bound (tests/teletext.rs).
const tt = nt.teletext;
check("frozen", Object.isFrozen(tt) && Object.isFrozen(tt.page) && Object.isFrozen(tt.overlay) && Object.isFrozen(tt.colours));
equal("level", tt.level, 25);
equal("regis", nt.regis, undefined);

// The page plane (5.1).
equal("put gives its cells", tt.page.put(1, 2, "HARBOUR NEWS", { ink: "yellow", height: 2 }), 14);
equal("at", tt.page.at(1, 1), 3);
equal("named colour", tt.page.put(2, 1, "a", { ink: 6 }), tt.page.put(3, 1, "a", { ink: "cyan" }));
throws("attribute cell", RangeError, "TELETEXT_ATTRIBUTE_CELL: row 1, column 13 holds the code 83", () => tt.page.put(1, 14, "x", { ink: "red" }));
throws("row 0", RangeError, "TELETEXT_ROWS: ", () => tt.page.put(0, 0, "x"));
throws("row 24", RangeError, "TELETEXT_ROWS: ", () => tt.page.codes(24, 0, new Uint8Array(1)));
throws("colour name", TypeError, "ink is 0 to 7 or one of black, red", () => tt.page.put(2, 5, "x", { ink: "amber" }));
throws("colour 8", RangeError, "COLOURS: ", () => tt.page.put(2, 5, "x", { ink: 8 }));
throws("height", TypeError, "height is 1 or 2", () => tt.page.put(2, 5, "x", { height: 3 }));
throws("mosaic", TypeError, "mosaic is false", () => tt.page.put(2, 5, "x", { mosaic: true }));
throws("text", TypeError, "text is a string", () => tt.page.put(2, 5, 7));
throws("unshown", TypeError, "the page's character sets cannot show '☃' (U+2603)", () => tt.page.put(2, 5, "☃"));
equal("nothing was written", tt.page.at(2, 5), 32);
equal("mosaics", tt.page.put(6, 3, "", { mosaic: "separated", ink: "red", hold: true }), 3);
equal("mosaic colour", tt.page.at(6, 2), 0x11);
tt.page.codes(6, 3, new Uint8Array([0x7f, 0x35]));
throws("code", RangeError, "MAX_ROW_CODE: ", () => tt.page.codes(6, 3, new Uint8Array([0x80])));
throws("codes type", TypeError, "codes is a Uint8Array", () => tt.page.codes(6, 3, [0x41]));
throws("blit length", RangeError, "BLIT_CODES: ", () => tt.page.blit(new Uint8Array(919)));
const plane = new Uint8Array(920).fill(0x20);
plane[0] = 0x41;
tt.page.blit(plane);
plane[0] = 0x42;
equal("blit copies", tt.page.at(1, 0), 0x41);
tt.page.scroll(1, 3, 2);
equal("scroll", tt.page.at(3, 0), 0x41);
tt.page.clear(3, 3);
equal("clear rows", tt.page.at(3, 0), 32);
tt.page.codes(23, 39, new Uint8Array([0x5a]));
tt.page.clear();
equal("clear", tt.page.at(23, 39), 32);
tt.page.settings({ screen: 4, rowColour: 4, substitution: true });
tt.page.settings({});
throws("settings", RangeError, "COLOUR_MAP_ENTRIES: ", () => tt.page.settings({ screen: 32 }));
tt.page.settings({ screen: 0, rowColour: 0, substitution: false });

// The Colour Map (5.5).
const amber = tt.colours.define("amber", "#fa0");
equal("first entry", amber, 16);
equal("long form", tt.colours.define("amber too", "#FfAa00"), 17);
tt.colours.set(amber, "#400");
throws("colour form", TypeError, "a colour is", () => tt.colours.define("x", "red"));
throws("entry 15", RangeError, "TELETEXT_DEFINABLE_COLOURS: ", () => tt.colours.set(15, "#000"));
throws("undefined entry", TypeError, "the entry 20 is not one", () => tt.colours.set(20, "#000"));

// The overlay and its triplets (5.2).
equal("no triplets yet", JSON.stringify(tt.overlay.triplets), '{"used":0,"limit":208}');
tt.overlay.fill(22, 0, 1, 40, { paper: amber, ink: 0 });
equal("a band is four triplets", tt.overlay.triplets.used, 4);
tt.overlay.put(10, 5, "café", { ink: 3, flash: true });
tt.overlay.clear(10, 5, 1, 4);
equal("a rectangle cleared", tt.overlay.triplets.used, 4);
throws("entry 32", RangeError, "COLOUR_MAP_ENTRIES: ", () => tt.overlay.fill(1, 0, 1, 1, { ink: 32 }));
throws("no rows", RangeError, "TELETEXT_ROWS: ", () => tt.overlay.clear(1, 0, 0, 1));
throws("budget", RangeError, "LOCAL_TRIPLETS: ", () => { for (let row = 1; row <= 23; row++) for (let column = 0; column < 40; column += 2) tt.overlay.fill(row, column, 1, 1, { ink: 1 }); });
check("the budget held", tt.overlay.triplets.used <= 208);
tt.overlay.clear();
equal("cleared", tt.overlay.triplets.used, 0);

// Text (5.6).
equal("measure", tt.measure("café"), 4);
equal("measure past a row", tt.measure("w".repeat(45)), 45);
equal("decomposed text is composed first", tt.measure("cafe\u0301"), 4);
const languages = tt.languages();
equal("languages", JSON.stringify(languages[0]), '{"tag":"en","name":"English","direction":"ltr"}');
check("a right-to-left language", languages.some((language) => language.tag === "he" && language.direction === "rtl"));
equal("German", tt.page.put(8, 0, "Grüße", { language: "de" }), 5);
equal("its national option", tt.page.at(8, 2), 0x7d);
throws("language", TypeError, "language is a tag", () => tt.page.put(8, 0, "x", { language: "tlh" }));

// Controls (5.4).
const forecast = tt.control(4, 1, 19, { label: "Shipping forecast" });
check("a control is a thing", forecast instanceof EventTarget && Object.isFrozen(Object.getPrototypeOf(forecast)));
equal("label", forecast.label, "Shipping forecast");
equal("bounds", JSON.stringify(forecast.bounds()), '{"x":1,"y":4,"width":19,"height":1}');
equal("joined is the same control", tt.control(5, 1, 19, { joined: true }), forecast);
equal("rows", forecast.rows, 2);
const tides = tt.control(5, 30, 5);
check("overlaps", !forecast.overlaps(tides) && forecast.overlaps(forecast));
forecast.look = "pressed";
equal("look", forecast.look, "pressed");
forecast.look = null;
throws("look", TypeError, "look is", () => { forecast.look = "alternative"; });
throws("over another", TypeError, "a control already holds", () => tt.control(4, 10, 3));
throws("label bound", RangeError, "MAX_LABEL_BYTES: ", () => { tides.label = "l".repeat(65); });
let heard = [];
forecast.addEventListener("pointerup", (event) => heard.push("control " + (event.target === forecast)));
nt.addEventListener("pointerup", (event) => { heard.push("nt " + (event.target === forecast)); console.log(heard.join(", ")); });
tides.remove();
tides.remove();
equal("removed", tides.bounds(), null);
throws("removed look", TypeError, "the control was removed", () => { tides.look = "hover"; });
const scope = nt.scope(() => { tt.control(9, 0, 4); });
equal("a scope's control", tt.overlay.triplets.used, 9);
scope.end();
equal("gone with the scope", tt.overlay.triplets.used, 7);
equal("budget", Object.keys(nt.budget()).slice(-9).join(), "triplets,colours,controls,sprites,spriteFrames,objectPageTriplets,bitmapCellsLossy,components,uiNodes");
equal("controls", nt.budget().controls.used, 1);
done();
