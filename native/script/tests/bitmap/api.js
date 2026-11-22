// Section 5.7 through nt.teletext.bitmap: each call's result and each refusal's kind and bound (tests/bitmap.rs).
const bm = nt.teletext.bitmap;
const page = nt.teletext.page;
check("frozen", Object.isFrozen(bm) && Object.isFrozen(nt.teletext));
equal("width", bm.width, 80);
equal("height", bm.height, 69);

// The back buffer is the script's own: set and get.
equal("black at first", bm.get(0, 0), 0);
bm.set(3, 4, 5);
equal("set then get", bm.get(3, 4), 5);
bm.set(-1, 0, 5); bm.set(80, 0, 5); bm.set(0, 69, 5);
equal("outside there is no sixel", bm.get(80, 0), undefined);
bm.set(0, 0, 16); bm.set(0, 0, 31); bm.set(0, 0, 0);
throws("colour 8", RangeError, "BITMAP_COLOURS: a sixel's colour is 0 to 7 or a Colour Map entry 16 to 31, not 8", () => bm.set(0, 0, 8));
throws("colour 15", RangeError, "BITMAP_COLOURS: ", () => bm.set(0, 0, 15));
throws("colour 32", RangeError, "BITMAP_COLOURS: ", () => bm.set(0, 0, 32));
throws("colour -1", RangeError, "BITMAP_COLOURS: ", () => bm.clear(-1));
throws("x is whole", TypeError, "x is a whole number", () => bm.set(0.5, 0, 1));
throws("colour is given", TypeError, "colour is a whole number", () => bm.set(0, 0));
throws("a string is no number", TypeError, "y is a whole number", () => bm.get(0, "1"));

// clear, line and rect.
bm.clear(2);
equal("clear fills", bm.get(79, 68), 2);
bm.clear();
equal("clear is black by default", bm.get(79, 68), 0);
bm.line(0, 0, 5, 5, 3);
equal("a line draws both ends", bm.get(0, 0) + bm.get(3, 3) + bm.get(5, 5), 9);
equal("and no further", bm.get(6, 6), 0);
bm.line(-10, 2, 200, 2, 4);
equal("a line is clipped", bm.get(0, 2) + bm.get(79, 2), 8);
throws("a line too far", RangeError, "MAX_BITMAP_COORDINATE: x1 is within 1024 of the bitmap's origin, not 1025", () => bm.line(0, 0, 1025, 0, 1));
bm.rect(10, 10, 4, 3, 6);
equal("rect fills", bm.get(10, 10) + bm.get(13, 12), 12);
equal("rect's edge", bm.get(14, 12) + bm.get(13, 13), 0);
bm.rect(20, 20, 5, 5, 1, { fill: false });
equal("an outline", bm.get(20, 20) + bm.get(24, 24) + bm.get(22, 20), 3);
equal("is hollow", bm.get(22, 22), 0);
bm.rect(70, 60, 100, 100, 2);
equal("rect is clipped", bm.get(79, 68), 2);
throws("negative width", RangeError, "MAX_BITMAP_COORDINATE: ", () => bm.rect(0, 0, -1, 1, 1));
throws("fill is a boolean", TypeError, "fill is true or false", () => bm.rect(0, 0, 1, 1, 1, { fill: 1 }));
throws("options are an object", TypeError, "the options are an object", () => bm.rect(0, 0, 1, 1, 1, 7));

// blit: one byte a sixel, the transparent byte leaves the sixel, the data is copied.
bm.blit(30, 30, 2, 2, new Uint8Array([1, 255, 255, 2]));
equal("blit", bm.get(30, 30) * 10 + bm.get(31, 31), 12);
equal("transparent leaves", bm.get(31, 30), 0);
bm.blit(30, 30, 2, 1, new Uint8Array([9, 3]), { transparent: 9 });
equal("another transparent byte", bm.get(30, 30) * 10 + bm.get(31, 30), 13);
const data = new Uint8Array([4]);
bm.blit(40, 40, 1, 1, data);
data[0] = 5;
equal("blit copies", bm.get(40, 40), 4);
bm.blit(-1, -1, 2, 2, new Uint8Array([7, 7, 7, 6]));
equal("blit is clipped", bm.get(0, 0), 6);
throws("blit's length", RangeError, "BITMAP_BLIT: 2 by 2 sixels are exactly 4 bytes, not 3", () => bm.blit(0, 0, 2, 2, new Uint8Array(3)));
throws("blit's colours", RangeError, "BITMAP_COLOURS: ", () => bm.blit(50, 50, 2, 1, new Uint8Array([1, 8])));
equal("a refused blit drew nothing", bm.get(50, 50), 0);
throws("blit's data", TypeError, "data is a Uint8Array", () => bm.blit(0, 0, 1, 1, [1]));
throws("a blit past the host's bytes", RangeError, "", () => bm.blit(0, 0, 1024, 1024, new Uint8Array(1024 * 1024)));

// text reserves cells.
bm.text(2, 3, "HI", { ink: 3, paper: 4 });
bm.text(23, 39, "!");
throws("text's row", RangeError, "TELETEXT_ROWS: ", () => bm.text(0, 0, "x"));
throws("text's column", RangeError, "COLUMNS: ", () => bm.text(1, 40, "x"));
throws("text past the row", RangeError, "COLUMNS: ", () => bm.text(1, 38, "abc"));
throws("text's ink", RangeError, "BITMAP_COLOURS: a text's ink", () => bm.text(1, 0, "x", { ink: 9 }));
throws("text is a string", TypeError, "text is a string", () => bm.text(1, 0, 7));
throws("a mark is no G0 character", TypeError, "a bitmap's text is characters of the page's own G0 set", () => bm.text(1, 0, "é"));
throws("unshown", TypeError, "the page's character sets cannot show", () => bm.text(1, 0, "☃"));

// Nothing has been shown, and every row is still the page's.
equal("the page writes", page.put(5, 0, "page"), 4);

// The first present takes rows 1 to 23.
const first = bm.present();
check("present gives cells and lossy", Number.isInteger(first.cells) && first.cells > 0 && Number.isInteger(first.lossy) && first.lossy >= 0);
equal("the bitmap wrote row 5", page.at(5, 0) === 0x70, false);
throws("put on a bitmap row", TypeError, "rows 1 to 23 are the bitmap's", () => page.put(5, 0, "x"));
throws("codes on a bitmap row", TypeError, "rows 1 to 23 are the bitmap's", () => page.codes(23, 0, new Uint8Array(1)));
throws("blit over bitmap rows", TypeError, "rows 1 to 23 are the bitmap's", () => page.blit(new Uint8Array(920).fill(32)));
throws("clear over bitmap rows", TypeError, "rows 1 to 23 are the bitmap's", () => page.clear());
throws("scroll over bitmap rows", TypeError, "rows 1 to 23 are the bitmap's", () => page.scroll(1, 3, 1));
throws("substitution", TypeError, "black background colour substitution stays off while the bitmap owns rows 1 to 23", () => page.settings({ substitution: true }));
page.settings({ screen: 4 });
const second = bm.present();
equal("a second present changes nothing", second.cells, 0);
equal("and loses what the first lost", second.lossy, first.lossy);
equal("the budget's lossy cells", nt.budget().bitmapCellsLossy.used, first.lossy);
equal("of the bitmap's cells", nt.budget().bitmapCellsLossy.limit, 920);

// rows and release.
bm.rows(10, 12);
equal("the page has row 5 again", page.put(5, 0, "free"), 4);
throws("row 10 is the bitmap's", TypeError, "rows 10 to 12 are the bitmap's", () => page.codes(10, 0, new Uint8Array(1)));
throws("double height into the bitmap", TypeError, "rows 10 to 12 are the bitmap's", () => page.put(9, 2, "x", { height: 2 }));
throws("rows in order", RangeError, "TELETEXT_ROWS: the first row, 5, is after the last, 4", () => bm.rows(5, 4));
throws("no row 0", RangeError, "TELETEXT_ROWS: ", () => bm.rows(0, 0));
throws("no row 24", RangeError, "TELETEXT_ROWS: ", () => bm.rows(1, 24));
throws("rows are whole", TypeError, "first is a whole number", () => bm.rows("1", 2));
bm.release();
equal("released", page.put(10, 0, "back"), 4);
bm.rows();
throws("rows() is 1 to 23", TypeError, "rows 1 to 23 are the bitmap's", () => page.put(10, 0, "x"));
bm.release();
page.settings({ substitution: true });
throws("no rows under substitution", TypeError, "the bitmap owns no row while black background colour substitution is on", () => bm.rows(1, 2));
throws("no present under substitution", TypeError, "the bitmap owns no row while black background colour substitution is on", () => bm.present());
done();
