
// The teletext API, `nt.teletext` (script-api.md section 5; work package
// T1): the page plane, the overlay, controls, the Colour Map, and text's
// measure and languages. It follows the shared core in the same function,
// with the table TT written in front by the host (teletext.rs). Like the
// core it only turns objects into flat host calls: every check that
// matters is the host's.
let teletextApi;
// What the sprites' part, which follows, takes from this one and gives it.
let teletextHelpers;
let teletextSprites;
{
  const COLOUR8 = Object.freeze({ black: 0, red: 1, green: 2, yellow: 3, blue: 4, magenta: 5, cyan: 6, white: 7 });
  const LOOKS = Object.freeze([null, "hover", "pressed"]);

  const optionsOf = (options) => {
    if (options === undefined) return {};
    if (options === null || typeof options !== "object") throw new TypeError("the options are an object");
    return options;
  };
  // A Level 1 colour: 0 to 7 or its name (section 5.1).
  const colour8 = (value, name) => {
    if (value === undefined) return TT.NONE;
    if (typeof value === "string") {
      if (!Object.prototype.hasOwnProperty.call(COLOUR8, value)) throw new TypeError(name + " is 0 to 7 or one of " + Object.keys(COLOUR8).join(", "));
      return COLOUR8[value];
    }
    const colour = whole(value, undefined, name);
    if (colour < 0) throw new RangeError("COLOURS: " + name + " is 0 to 7, not " + colour);
    return colour;
  };
  // A Colour Map entry, 0 to 31 (section 5.2).
  const entryOf = (value, name) => {
    if (value === undefined) return TT.NONE;
    const entry = whole(value, undefined, name);
    if (entry < 0) throw new RangeError("COLOUR_MAP_ENTRIES: " + name + " is a Colour Map entry, 0 to 31, not " + entry);
    return entry;
  };
  const onOff = (value, shift, name) => {
    if (value === undefined) return TT.NOT_GIVEN;
    if (typeof value !== "boolean") throw new TypeError(name + " is true or false");
    return (value ? TT.ON : TT.OFF) << shift;
  };
  const mosaicOf = (value) => {
    if (value === undefined) return TT.NOT_GIVEN;
    if (value === false) return TT.OFF << TT.MOSAIC_SHIFT;
    if (value === "contiguous") return TT.ON << TT.MOSAIC_SHIFT;
    if (value === "separated") return TT.SEPARATED << TT.MOSAIC_SHIFT;
    throw new TypeError("mosaic is false, \"contiguous\" or \"separated\"");
  };
  const heightOf = (value) => {
    if (value === undefined) return 0;
    if (value !== 1 && value !== 2) throw new TypeError("height is 1 or 2");
    return value;
  };
  // Text is given to the encoder in normalisation form C, as it asks.
  const textOf = (text) => {
    if (typeof text !== "string") throw new TypeError("text is a string");
    return text.normalize("NFC");
  };
  const codesOf = (codes) => {
    if (!(codes instanceof Uint8Array)) throw new TypeError("codes is a Uint8Array");
    return codes;
  };
  // The engine's languages, asked for once: a call names one by its place.
  let languageList = null;
  const languageTags = () => {
    if (!languageList) languageList = JSON.parse(host(OP.TT_LANGUAGES));
    return languageList;
  };
  const languageOf = (tag) => {
    if (tag === undefined) return TT.NONE;
    const index = languageTags().findIndex((language) => language.tag === tag);
    if (index < 0) throw new TypeError("language is a tag of nt.teletext.languages()");
    return index;
  };
  // "#rgb", teletext's twelve bits exactly, or "#rrggbb", the top four
  // bits of each primary (section 5.5).
  const rgb12 = (colour) => {
    const short = typeof colour === "string" ? /^#([0-9a-fA-F]{3})$/.exec(colour) : null;
    if (short) return parseInt(short[1], 16);
    const long = typeof colour === "string" ? /^#([0-9a-fA-F])[0-9a-fA-F]([0-9a-fA-F])[0-9a-fA-F]([0-9a-fA-F])[0-9a-fA-F]$/.exec(colour) : null;
    if (long) return parseInt(long[1] + long[2] + long[3], 16);
    throw new TypeError("a colour is \"#rgb\" or \"#rrggbb\"");
  };

  const page = Object.freeze({
    put(row, column, text, style) {
      const o = optionsOf(style);
      const flags = onOff(o.flash, TT.FLASH_SHIFT, "flash") | onOff(o.conceal, TT.CONCEAL_SHIFT, "conceal") | onOff(o.box, TT.BOX_SHIFT, "box")
        | onOff(o.hold, TT.HOLD_SHIFT, "hold") | mosaicOf(o.mosaic);
      return host(OP.TT_PUT, row, column, colour8(o.ink, "ink"), colour8(o.paper, "paper"), heightOf(o.height), flags, languageOf(o.language), textOf(text));
    },
    codes(row, column, codes) { host(OP.TT_CODES, row, column, codesOf(codes)); },
    blit(codes) { host(OP.TT_BLIT, codesOf(codes)); },
    at(row, column) { return host(OP.TT_AT, row, column); },
    clear(firstRow = TT.FIRST_ROW, lastRow = TT.LAST_ROW) { host(OP.TT_CLEAR, firstRow, lastRow); },
    scroll(firstRow, lastRow, rows) { host(OP.TT_SCROLL, firstRow, lastRow, rows); },
    settings(options) {
      const o = optionsOf(options);
      if (o.substitution !== undefined && typeof o.substitution !== "boolean") throw new TypeError("substitution is true or false");
      host(OP.TT_SETTINGS, entryOf(o.screen, "screen"), entryOf(o.rowColour, "rowColour"), o.substitution === undefined ? TT.NONE : o.substitution ? 1 : 0);
    },
  });

  const overlay = Object.freeze({
    put(row, column, text, look) {
      const o = optionsOf(look);
      host(OP.TT_OVERLAY_PUT, row, column, entryOf(o.ink, "ink"), entryOf(o.paper, "paper"), onOff(o.flash, TT.FLASH_SHIFT, "flash"), languageOf(o.language), textOf(text));
    },
    fill(row, column, rows, columns, look) {
      const o = optionsOf(look);
      host(OP.TT_OVERLAY_FILL, row, column, rows, columns, entryOf(o.ink, "ink"), entryOf(o.paper, "paper"));
    },
    clear(row, column, rows, columns) {
      // No rows is the host's word for the whole overlay.
      if (row === undefined && column === undefined && rows === undefined && columns === undefined) host(OP.TT_OVERLAY_CLEAR, 0, 0, 0, 0);
      else if (rows === 0) throw new RangeError("TELETEXT_ROWS: a rectangle has at least one row");
      else host(OP.TT_OVERLAY_CLEAR, row, column, rows, columns);
    },
    // Every overlay cell, mark, G2 character and control is paid for from
    // the page's local triplets (section 5.2).
    get triplets() { return nt.budget().triplets; },
  });

  const colours = Object.freeze({
    define(name, colour) {
      if (typeof name !== "string") throw new TypeError("a colour's name is a string");
      return host(OP.TT_COLOUR_DEFINE, rgb12(colour), name);
    },
    set(entry, colour) { host(OP.TT_COLOUR_SET, whole(entry, undefined, "entry"), rgb12(colour)); },
  });

  // Controls (section 5.4): things, whose pointer events go on to nt.
  const controlStates = new WeakMap();
  const stateOf = (control) => {
    const state = controlStates.get(control);
    if (!state) throw new TypeError("not a control");
    return state;
  };
  const liveOf = (control) => {
    const state = stateOf(control);
    if (state.removed) throw new TypeError("the control was removed");
    return state;
  };
  // Forgets a control here: its listeners first, while the host still
  // knows its handle, so that the host is told each is gone.
  const forget = (control) => {
    const state = controlStates.get(control);
    if (!state || state.removed) return false;
    const byType = listenersOf.get(control);
    if (byType) for (const [type, list] of Array.from(byType)) for (const record of list.slice()) removeRecord(control, type, record);
    state.removed = true;
    things.delete(state.handle);
    handleOf.delete(control);
    parentOf.delete(control);
    return true;
  };
  class Control extends EventTarget {
    get row() { return stateOf(this).row; }
    get column() { return stateOf(this).column; }
    get columns() { return stateOf(this).columns; }
    get rows() { return stateOf(this).rows; }
    get detectable() { return true; }
    get label() { return stateOf(this).label; }
    set label(text) {
      const state = liveOf(this);
      if (typeof text !== "string") throw new TypeError("a label is a string");
      host(OP.TT_LABEL, state.handle, text);
      state.label = text;
    }
    get look() { return LOOKS[stateOf(this).look]; }
    set look(look) {
      const state = liveOf(this);
      const number = LOOKS.indexOf(look);
      if (number < 0) throw new TypeError("look is \"hover\", \"pressed\" or null");
      host(OP.TT_ANIMATE, state.handle, TT.LOOK, number, 0, 0, 0);
      state.look = number;
    }
    remove() {
      const state = stateOf(this);
      if (state.removed) return;
      const handle = state.handle;
      forget(this);
      host(OP.TT_REMOVE, handle);
    }
    // In cells (section 3.9).
    bounds() {
      const state = stateOf(this);
      return state.removed ? null : { x: state.column, y: state.row, width: state.columns, height: state.rows };
    }
    overlaps(other) {
      const a = this.bounds();
      const b = other && typeof other.bounds === "function" ? other.bounds() : null;
      return !!a && !!b && a.x < b.x + b.width && b.x < a.x + a.width && a.y < b.y + b.height && b.y < a.y + a.height;
    }
  }
  Object.freeze(Control.prototype);
  const control = (row, column, columns, options) => {
    const o = optionsOf(options);
    if (o.joined !== undefined && typeof o.joined !== "boolean") throw new TypeError("joined is true or false");
    if (o.label !== undefined && typeof o.label !== "string") throw new TypeError("a label is a string");
    const handle = host(OP.TT_CONTROL, row, column, columns, o.joined ? 1 : 0);
    // A joined run is one more row of the control above, whose handle the
    // host gives again.
    let thing = things.get(handle);
    if (thing) {
      stateOf(thing).rows += 1;
    } else {
      thing = new Control();
      controlStates.set(thing, { handle, row, column, columns, rows: 1, look: 0, label: "", removed: false });
      things.set(handle, thing);
      handleOf.set(thing, handle);
      parentOf.set(thing, nt);
      const made = thing;
      core.scopeEnds(() => { forget(made); });
    }
    if (o.label !== undefined) thing.label = o.label;
    return thing;
  };

  teletextHelpers = Object.freeze({ optionsOf, entryOf, textOf, languageOf });
  const api = {
    page, overlay, colours, control,
    block(columns, rows) { return teletextSprites.block(columns, rows); },
    sprite(options) { return teletextSprites.sprite(options); },
    bitmap: teletextBitmap,
    get level() { return host(OP.TT_LEVEL); },
    measure(text, options) { return host(OP.TT_MEASURE, languageOf(optionsOf(options).language), textOf(text)); },
    languages() { return languageTags().map((language) => ({ tag: language.tag, name: language.name, direction: language.direction })); },
  };
  // A page context has the API of its own page's renderer and no other;
  // a channel context has it for the teletext pages it is shown, where
  // the host lets it ask the level, the languages and a text's measure
  // (section 3.1: renderer state belongs to the page).
  if (!isPage || host(OP.INFO, INFO.PAGE_RENDERER) === "teletext") teletextApi = Object.freeze(api);
}
