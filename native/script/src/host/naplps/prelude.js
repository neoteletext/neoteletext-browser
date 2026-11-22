
// nt.naplps (script-api.md section 7; work packages N1 and N2) -------------------
//
// The script-side half of NAPLPS's own model: the colour map, the backdrop
// and the painter (sections 7.1 to 7.3), definitions (7.4) and the cast
// (7.5). Like the rest of the prelude it is not trusted: a
// painter only records its calls as whole numbers (a painter program,
// section 10.2), and the host checks every word and writes every byte of
// NAPLPS itself. The tables NP (the painter's calls) and NP_BOUND are
// written in front of the prelude by the host.

const naplpsApi = (function () {
  // The page context of a NAPLPS page has all of it. The channel context
  // has its share of the cast, while the page shown is a NAPLPS page
  // (naplpsNow, below). A page of another renderer has none of it.
  if (isPage && host(OP.INFO, INFO.PAGE_RENDERER) !== "naplps") return undefined;

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


  // Definitions (section 7.4) ---------------------------------------------------

  function handleFor(thing, what) {
    const handle = handleOf.get(thing);
    if (handle === undefined) throw new TypeError(what);
    return handle;
  }

  class NaplpsSymbol {
    #width;
    #height;
    constructor(width, height) { this.#width = width; this.#height = height; }
    get width() { return this.#width; }
    get height() { return this.#height; }
    // Every actor drawn by it changes: actors are drawn by their macros
    // as last defined.
    redefine(fn) { host(OP.NP_SYMBOL, handleFor(this, "the symbol was removed"), this.#width, this.#height, record(fn)); }
    remove() {
      host(OP.NP_REMOVE, handleFor(this, "the symbol was removed"));
      handleOf.delete(this);
    }
  }
  Object.freeze(NaplpsSymbol.prototype);
  class NaplpsDrcs {}
  class NaplpsTexture {}

  function symbol(size, fn) {
    const { width, height } = size || {};
    const made = new NaplpsSymbol(int(width, "a symbol's width"), int(height, "a symbol's height"));
    handleOf.set(made, host(OP.NP_SYMBOL, 0, width, height, record(fn)));
    // The host removes a scope's symbols with the scope.
    core.scopeEnds(() => handleOf.delete(made));
    return Object.freeze(made);
  }
  function drcs(fn) {
    const made = new NaplpsDrcs();
    handleOf.set(made, host(OP.NP_DRCS, record(fn)));
    return Object.freeze(made);
  }
  function texture(size, fn) {
    const { width, height } = size || {};
    const made = new NaplpsTexture();
    handleOf.set(made, host(OP.NP_TEXTURE, int(width, "a texture's width"), int(height, "a texture's height"), record(fn)));
    return Object.freeze(made);
  }

  // The cast (section 7.5) --------------------------------------------------------

  const LOOKS = new Map([["normal", 0], ["hover", 1], ["pressed", 2], ["alternative", 3]]);
  const LOOK_NAMES = ["normal", "hover", "pressed", "alternative"];
  // Sixteen bits, as the record and the receiver hold a place.
  const hold16 = (value) => Math.max(-32768, Math.min(32767, value));

  function symbolOrNone(value, what) {
    return value === undefined || value === null ? 0 : handleFor(value, what + " is a symbol");
  }

  // The whole numbers and the frames NP_ACTOR takes for a definition.
  function actorCall(existing, d) {
    const flags = (d.shown ? 1 : 0) | (d.detectable ? 2 : 0) | (d.wheel ? 4 : 0);
    const ints = [
      existing, d.region ? 2 : 1, int(d.x, "x"), int(d.y, "y"), int(d.width, "width"), int(d.height, "height"),
      int(d.originX, "originX"), int(d.originY, "originY"), int(d.layer, "layer"), int(d.order, "order"), flags, d.region ? entryOf(d.cover) : 0,
      symbolOrNone(d.hover, "hover"), symbolOrNone(d.pressed, "pressed"), symbolOrNone(d.alternative, "alternative"),
    ];
    if (d.region) return host(OP.NP_ACTOR, ...ints);
    if (!Array.isArray(d.frames)) throw new TypeError("an actor's frames are an array of symbols");
    return host(OP.NP_ACTOR, ...ints, Int32Array.from(d.frames, (frame) => handleFor(frame, "a frame is a symbol")));
  }

  // The size of an object: as given, else its first frame's.
  function sized(d) {
    const first = !d.region && Array.isArray(d.frames) ? d.frames[0] : undefined;
    return { width: d.width || (first instanceof NaplpsSymbol ? first.width : 0), height: d.height || (first instanceof NaplpsSymbol ? first.height : 0) };
  }

  class NaplpsThing extends EventTarget {
    #d; // its definition, as last given
    #s; // its state, as the host holds it
    #handle;
    constructor(definition) {
      super();
      this.#d = definition;
      this.#handle = actorCall(0, definition);
      this.#settle();
      handleOf.set(this, this.#handle);
      things.set(this.#handle, this);
      core.scopeEnds(() => this.#forget());
      if (definition.label !== undefined) this.label = definition.label;
    }
    #settle() {
      const d = this.#d;
      this.#s = { x: d.x, y: d.y, ...sized(d), shown: !!d.shown, layer: d.layer, order: d.order, frame: 0, look: 0, label: this.#s ? this.#s.label : "" };
    }
    #forget() {
      things.delete(this.#handle);
      handleOf.delete(this);
      this.#s = null;
    }
    #live() {
      if (!this.#s) throw new TypeError("the actor or region was removed");
      return this.#s;
    }
    #animate(operation, a = 0, b = 0, c = 0, d = 0) {
      this.#live();
      host(OP.NP_ANIMATE, this.#handle, operation, a, b, c, d);
    }
    get x() { return this.#live().x; }
    set x(value) { this.moveTo(value, this.#live().y); }
    get y() { return this.#live().y; }
    set y(value) { this.moveTo(this.#live().x, value); }
    get width() { return this.#live().width; }
    get height() { return this.#live().height; }
    moveTo(x, y) {
      this.#animate(NP_BOUND.ANIMATE_MOVE, 0, int(x, "x"), int(y, "y"));
      Object.assign(this.#s, { x, y });
    }
    moveBy(dx, dy) {
      this.#animate(NP_BOUND.ANIMATE_MOVE, 1, int(dx, "dx"), int(dy, "dy"));
      Object.assign(this.#s, { x: hold16(this.#s.x + dx), y: hold16(this.#s.y + dy) });
    }
    get shown() { return this.#live().shown; }
    set shown(value) {
      this.#animate(value ? NP_BOUND.ANIMATE_SHOW : NP_BOUND.ANIMATE_HIDE);
      this.#s.shown = !!value;
    }
    get layer() { return this.#live().layer; }
    set layer(value) {
      this.#animate(NP_BOUND.ANIMATE_ORDER, int(value, "layer"), this.#live().order);
      this.#s.layer = value;
    }
    get order() { return this.#live().order; }
    set order(value) {
      this.#animate(NP_BOUND.ANIMATE_ORDER, NP_BOUND.KEEP_LAYER, int(value, "order"));
      this.#s.order = value;
    }
    get frame() { return this.#live().frame; }
    set frame(value) {
      this.#animate(NP_BOUND.ANIMATE_FRAME, int(value, "frame"));
      this.#s.frame = value;
    }
    cycle(first, last, period, options) {
      const { once = false } = options || {};
      this.#animate(NP_BOUND.ANIMATE_CYCLE, int(first, "first"), int(last, "last"), int(period, "period"), once ? 1 : 0);
      this.#s.frame = first;
    }
    stopCycle() { this.#animate(NP_BOUND.ANIMATE_STOP_CYCLE); }
    get look() { return LOOK_NAMES[this.#live().look]; }
    set look(value) {
      const look = choice(LOOKS, value, "look");
      this.#animate(NP_BOUND.ANIMATE_LOOK, look);
      this.#s.look = look;
    }
    get label() { return this.#live().label; }
    set label(value) {
      if (typeof value !== "string") throw new TypeError("a label is a string");
      this.#live();
      host(OP.NP_LABEL, this.#handle, value);
      this.#s.label = value;
    }
    get detectable() { return !!this.#d.detectable; }
    // Sends the one object again: the others and their state are kept,
    // and this one's frame, cycle and look start again.
    redefine(options) {
      this.#live();
      const definition = { ...this.#d, x: this.#s.x, y: this.#s.y, shown: this.#s.shown, layer: this.#s.layer, order: this.#s.order, ...(options || {}) };
      actorCall(this.#handle, definition);
      this.#d = definition;
      this.#settle();
      if (options && options.label !== undefined) this.label = options.label;
    }
    remove() {
      this.#live();
      host(OP.NP_REMOVE, this.#handle);
      this.#forget();
    }
    // Its rectangle in dots, or null while it is hidden or once it is removed.
    bounds() {
      const s = this.#s;
      return s && s.shown ? { x: s.x, y: s.y, width: s.width, height: s.height } : null;
    }
    overlaps(other) {
      if (!(other instanceof NaplpsThing)) throw new TypeError("overlaps takes an actor or a region");
      const a = this.bounds(), b = other.bounds();
      return !!a && !!b && a.x < b.x + b.width && b.x < a.x + a.width && a.y < b.y + b.height && b.y < a.y + a.height;
    }
  }
  class NaplpsActor extends NaplpsThing {}
  class NaplpsRegion extends NaplpsThing {}
  for (const made of [NaplpsThing, NaplpsActor, NaplpsRegion]) Object.freeze(made.prototype);

  function actor(options) {
    const {
      frames, x = 0, y = 0, width = 0, height = 0, originX = 0, originY = 0, layer = isPage ? 2 : 3, order = 0, shown = true,
      detectable = false, wheel = false, hover, pressed, alternative, label,
    } = options || {};
    return new NaplpsActor({ region: false, frames, x, y, width, height, originX, originY, layer, order, shown, detectable, wheel, hover, pressed, alternative, label });
  }
  function region(options) {
    const { x = 0, y = 0, width, height, cover, layer = isPage ? 2 : 3, order = 0, shown = true, detectable = true, wheel = false, hover, pressed, label } = options || {};
    return new NaplpsRegion({ region: true, x, y, width, height, originX: 0, originY: 0, layer, order, shown, detectable, wheel, cover, hover, pressed, label });
  }

  // The channel context draws only in the overlay: its share of the cast
  // and the symbols its actors use (section 3.1).
  if (!isPage) return Object.freeze({ NaplpsPainter, symbol, actor, region });

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

  return Object.freeze({ colours, backdrop, measure, NaplpsPainter, symbol, drcs, texture, actor, region });
})();

// What `nt.naplps` is now: in the page context of a NAPLPS page, all of
// it; in the channel context its share, while the page shown is drawn in
// NAPLPS; otherwise undefined, as each renderer's API is off its own pages.
function naplpsNow() {
  if (isPage) return naplpsApi;
  const page = pageNow();
  return page && page.renderer === "naplps" ? naplpsApi : undefined;
}
