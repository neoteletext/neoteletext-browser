
// nt.naplps (script-api.md sections 7.1 to 7.3; work package N1) ----------------
//
// The script-side half of NAPLPS's own model: the colour map, the backdrop
// and the painter. Like the rest of the prelude it is not trusted: a
// painter only records its calls as whole numbers (a painter program,
// section 10.2), and the host checks every word and writes every byte of
// NAPLPS itself. The tables NP (the painter's calls) and NP_BOUND are
// written in front of the prelude by the host.

const naplpsApi = (function () {
  if (!isPage || host(OP.INFO, INFO.PAGE_RENDERER) !== "naplps") return undefined;

  // Teletext's eight colours are entries 0 to 7 (section 7.1).
  const colourNames = new Map([["black", 0], ["red", 1], ["green", 2], ["yellow", 3], ["blue", 4], ["magenta", 5], ["cyan", 6], ["white", 7]]);

  function int(value, name) {
    if (typeof value !== "number" || !Number.isInteger(value)) throw new TypeError(name + " is a whole number");
    return value;
  }

  // "#rgb" or "#rrggbb" as 24 bits.
  function rgb24(colour) {
    if (typeof colour !== "string") throw new TypeError("a colour is \"#rgb\" or \"#rrggbb\"");
    const short = /^#([0-9a-f])([0-9a-f])([0-9a-f])$/i.exec(colour);
    if (short) return parseInt(short[1] + short[1] + short[2] + short[2] + short[3] + short[3], 16);
    const long = /^#([0-9a-f]{6})$/i.exec(colour);
    if (long) return parseInt(long[1], 16);
    throw new TypeError("a colour is \"#rgb\" or \"#rrggbb\"");
  }

  // A colour-map entry: its number, or the name it was defined by.
  function entryOf(value) {
    if (typeof value === "string") {
      const entry = colourNames.get(value);
      if (entry === undefined) throw new TypeError("no colour is named " + value);
      return entry;
    }
    return int(value, "a colour-map entry");
  }

  function choice(table, value, what) {
    const code = table.get(value);
    if (code === undefined) throw new TypeError("no such " + what + ": " + String(value));
    return code;
  }

  const STROKES = new Map([["solid", 0], ["dotted", 1], ["dashed", 2], ["dotdash", 3]]);
  const FILLS = new Map([["solid", 0], ["vertical", 1], ["horizontal", 2], ["cross", 3]]);
  const SIZES = new Map([["normal", 0], ["double-height", 1], ["double-size", 2]]);
  const ROTATIONS = new Map([[0, 0], [90, 1], [180, 2], [270, 3]]);
  const PATHS = new Map([["right", 0], ["left", 1], ["up", 2], ["down", 3]]);
  const SPACINGS = new Map([[1, 0], [1.25, 1], [1.5, 2], ["proportional", 3]]);
  const ROW_SPACINGS = new Map([[1, 0], [1.25, 1], [1.5, 2], [2, 3]]);

  // A painter records calls while its function runs and at no other time.
  const recording = new WeakMap(); // painter -> array of words, while open

  function wordsOf(painter) {
    const words = recording.get(painter);
    if (!words) throw new TypeError("a painter is used only inside the function it was given to");
    if (words.length > NP_BOUND.MAX_PAINTER_WORDS) throw new RangeError("MAX_PAINTER_WORDS: the painting has too many calls");
    return words;
  }

  // A string as its length and code points, in normalisation form C,
  // which the engine's text encoder takes.
  function pushString(words, text, what) {
    if (typeof text !== "string") throw new TypeError(what + " is a string");
    const points = Array.from(text.normalize("NFC"), (character) => character.codePointAt(0));
    words.push(points.length);
    for (const point of points) words.push(point);
  }

  function pushPairs(words, pairs, what) {
    if (!Array.isArray(pairs)) throw new TypeError(what + " is an array of [x, y] pairs");
    words.push(pairs.length);
    for (const pair of pairs) {
      if (!Array.isArray(pair) || pair.length !== 2) throw new TypeError(what + " is an array of [x, y] pairs");
      words.push(int(pair[0], "a coordinate"), int(pair[1], "a coordinate"));
    }
  }

  class NaplpsPainter {
    #call(number, ...ints) {
      const words = wordsOf(this);
      words.push(number);
      for (const value of ints) words.push(int(value, "a painter's argument"));
      return this;
    }
    #points(number, points, what) {
      const words = wordsOf(this);
      words.push(number);
      pushPairs(words, points, what);
      return this;
    }
    moveTo(x, y) { return this.#call(NP.MOVE_TO, x, y); }
    moveBy(dx, dy) { return this.#call(NP.MOVE_BY, dx, dy); }
    pointAt(x, y) { return this.#call(NP.POINT_AT, x, y); }
    pointBy(dx, dy) { return this.#call(NP.POINT_BY, dx, dy); }
    lineTo(x, y) { return this.#call(NP.LINE_TO, x, y); }
    lineBy(dx, dy) { return this.#call(NP.LINE_BY, dx, dy); }
    polyline(points) { return this.#points(NP.POLYLINE, points, "a polyline"); }
    polylineBy(steps) { return this.#points(NP.POLYLINE_BY, steps, "a polyline"); }
    rect(x, y, width, height) { return this.#call(NP.RECT, x, y, width, height); }
    fillRect(x, y, width, height) { return this.#call(NP.FILL_RECT, x, y, width, height); }
    rectBy(width, height) { return this.#call(NP.RECT_BY, width, height); }
    fillRectBy(width, height) { return this.#call(NP.FILL_RECT_BY, width, height); }
    polygon(points) { return this.#points(NP.POLYGON, points, "a polygon"); }
    fillPolygon(points) { return this.#points(NP.FILL_POLYGON, points, "a polygon"); }
    polygonBy(steps) { return this.#points(NP.POLYGON_BY, steps, "a polygon"); }
    fillPolygonBy(steps) { return this.#points(NP.FILL_POLYGON_BY, steps, "a polygon"); }
    arc(mx, my, ex, ey) { return this.#call(NP.ARC, mx, my, ex, ey); }
    fillArc(mx, my, ex, ey) { return this.#call(NP.FILL_ARC, mx, my, ex, ey); }
    circle(cx, cy, radius) { return this.#call(NP.CIRCLE, cx, cy, radius); }
    disc(cx, cy, radius) { return this.#call(NP.DISC, cx, cy, radius); }
    trace(dx, dy, moves) {
      this.#call(NP.TRACE, dx, dy);
      pushString(wordsOf(this), moves, "a trace's moves");
      return this;
    }
    fillTrace(dx, dy, moves) {
      this.#call(NP.FILL_TRACE, dx, dy);
      pushString(wordsOf(this), moves, "a trace's moves");
      return this;
    }
    image(x, y, width, height, pixels) {
      if (!(pixels instanceof Uint8Array)) throw new TypeError("an image's pixels are a Uint8Array of colour-map entries");
      this.#call(NP.IMAGE, x, y, width, height);
      const words = wordsOf(this);
      words.push(pixels.length);
      for (let index = 0; index < pixels.length; index++) words.push(pixels[index]);
      return this;
    }
    ink(entry) { return this.#call(NP.INK, entryOf(entry)); }
    inkOn(entry, background) { return this.#call(NP.INK_ON, entryOf(entry), entryOf(background)); }
    pen(width, height) { return this.#call(NP.PEN, width, height); }
    stroke(style) { return this.#call(NP.STROKE, choice(STROKES, style, "line texture")); }
    fillStyle(style) {
      if (typeof style === "string") return this.#call(NP.FILL_STYLE, choice(FILLS, style, "fill style"));
      // A Texture (section 7.4) is a thing the host knows by its handle.
      const handle = handleOf.get(style);
      if (handle === undefined) throw new TypeError("a fill style is a name or a Texture");
      return this.#call(NP.FILL_STYLE, NP_BOUND.FILL_TEXTURE, handle);
    }
    highlight(on) { return this.#call(NP.HIGHLIGHT, on ? 1 : 0); }
    mask(width, height) { return this.#call(NP.MASK, width, height); }
    text(row, column, string, options = {}) {
      const { size = "normal", language = "" } = options || {};
      this.#call(NP.TEXT, row, column, choice(SIZES, size, "text size"));
      pushString(wordsOf(this), language, "a language");
      pushString(wordsOf(this), string, "a text");
      return this;
    }
    textAt(x, y, string, options = {}) {
      const {
        width = NP_BOUND.CELL_DOTS, height = NP_BOUND.CELL_LINES, rotation = 0, path = "right", spacing = 1, rowSpacing = 1,
        underline = false, reverse = false, language = "",
      } = options || {};
      this.#call(
        NP.TEXT_AT, x, y, width, height, choice(ROTATIONS, rotation, "rotation"), choice(PATHS, path, "text path"),
        choice(SPACINGS, spacing, "character spacing"), choice(ROW_SPACINGS, rowSpacing, "row spacing"), underline ? 1 : 0, reverse ? 1 : 0,
      );
      pushString(wordsOf(this), language, "a language");
      pushString(wordsOf(this), string, "a text");
      return this;
    }
    glyph(drcs) {
      const handle = handleOf.get(drcs);
      if (handle === undefined) throw new TypeError("a glyph is a DRCS character");
      return this.#call(NP.GLYPH, handle);
    }
    stamp(symbol, x, y) {
      const handle = handleOf.get(symbol);
      if (handle === undefined) throw new TypeError("a stamp is a symbol");
      return this.#call(NP.STAMP, handle, x, y);
    }
    wait(tenths) { return this.#call(NP.WAIT, tenths); }
  }
  Object.freeze(NaplpsPainter.prototype);

  // Runs `fn` with a painter and gives the program it recorded. What a
  // later part of the API (definitions, section 7.4) records with too.
  function record(fn) {
    if (typeof fn !== "function") throw new TypeError("a painting is a function of a painter");
    const painter = new NaplpsPainter();
    const words = [];
    recording.set(painter, words);
    try {
      fn(painter);
    } finally {
      recording.delete(painter);
    }
    if (words.length > NP_BOUND.MAX_PAINTER_WORDS) throw new RangeError("MAX_PAINTER_WORDS: the painting has too many calls");
    return Int32Array.from(words);
  }

  const colours = Object.freeze({
    define(name, colour) {
      if (typeof name !== "string") throw new TypeError("a colour's name is a string");
      const entry = host(OP.NP_COLOUR_DEFINE, rgb24(colour), name);
      colourNames.set(name, entry);
      return entry;
    },
    set(entry, colour) { host(OP.NP_COLOUR_SET, entryOf(entry), rgb24(colour)); },
    blink(entry, options) {
      const { to, on = 5, off = 5, delay = 0 } = options || {};
      host(OP.NP_BLINK, entryOf(entry), entryOf(to), int(on, "on"), int(off, "off"), int(delay, "delay"));
    },
    steady(entry) { host(OP.NP_BLINK, entryOf(entry), NP_BOUND.STEADY, 0, 0, 0); },
  });

  class NaplpsBackdrop extends EventTarget {
    paint(fn) { host(OP.NP_PAINT, record(fn)); }
    repaint(fn) { host(OP.NP_REPAINT, record(fn)); }
  }
  const backdrop = new NaplpsBackdrop();
  handleOf.set(backdrop, NP_BOUND.BACKDROP_HANDLE);
  things.set(NP_BOUND.BACKDROP_HANDLE, backdrop);
  Object.freeze(NaplpsBackdrop.prototype);
  Object.freeze(backdrop);

  function measure(string, options = {}) {
    const { width = NP_BOUND.CELL_DOTS, height = NP_BOUND.CELL_LINES, spacing = 1, language = "" } = options || {};
    if (typeof string !== "string" || typeof language !== "string") throw new TypeError("a text and its language are strings");
    const answer = host(
      OP.NP_MEASURE, int(width, "width"), int(height, "height"), choice(SPACINGS, spacing, "character spacing"), utf8Length(language),
      language + string.normalize("NFC"),
    ).split(",").map(Number);
    return { width: answer[0], height: answer[1] };
  }

  return Object.freeze({ colours, backdrop, measure, NaplpsPainter });
})();
