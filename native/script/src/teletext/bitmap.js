
// The sixel bitmap, `nt.teletext.bitmap` (script-api.md section 5.7; work
// package T3): a double-buffered bitmap of TTB.WIDTH by TTB.HEIGHT sixels
// that the host turns into characters. It comes before the teletext API in
// the same function, which takes it into `nt.teletext` before freezing
// that, with the table TTB written in front by the host
// (teletext/bitmap.rs). Every call draws in the back buffer, which is the
// script's own; nothing shows until present(). Every check that matters is
// the host's.
let teletextBitmap;
{
  const optionsOf = (options) => {
    if (options === undefined) return {};
    if (options === null || typeof options !== "object") throw new TypeError("the options are an object");
    return options;
  };
  const number = (value, name) => {
    if (value === undefined) throw new TypeError(name + " is a whole number");
    return whole(value, undefined, name);
  };
  // A text's colour: left out, or a sixel's colour.
  const optional = (value, name) => (value === undefined ? TTB.NONE : whole(value, undefined, name));

  teletextBitmap = Object.freeze({
    width: TTB.WIDTH,
    height: TTB.HEIGHT,
    // The rows the bitmap owns; a row that changes owner is cleared.
    rows(first = TTB.FIRST_ROW, last = TTB.LAST_ROW) {
      // Row 0 is the host's word for "no rows", which is release()'s.
      if (first === TTB.NO_ROWS && last === TTB.NO_ROWS) throw new RangeError("TELETEXT_ROWS: a row is " + TTB.FIRST_ROW + " to " + TTB.LAST_ROW + ", not 0");
      host(OP.BM_ROWS, number(first, "first"), number(last, "last"));
    },
    // Gives every row back to nt.teletext.page.
    release() { host(OP.BM_ROWS, TTB.NO_ROWS, TTB.NO_ROWS); },
    clear(colour = 0) { host(OP.BM_CLEAR, number(colour, "colour")); },
    set(x, y, colour) { host(OP.BM_SET, number(x, "x"), number(y, "y"), number(colour, "colour")); },
    get(x, y) { return host(OP.BM_GET, number(x, "x"), number(y, "y")); },
    line(x0, y0, x1, y1, colour) { host(OP.BM_LINE, number(x0, "x0"), number(y0, "y0"), number(x1, "x1"), number(y1, "y1"), number(colour, "colour")); },
    rect(x, y, width, height, colour, options) {
      const fill = optionsOf(options).fill;
      if (fill !== undefined && typeof fill !== "boolean") throw new TypeError("fill is true or false");
      host(OP.BM_RECT, number(x, "x"), number(y, "y"), number(width, "width"), number(height, "height"), number(colour, "colour"), fill === false ? 0 : 1);
    },
    blit(x, y, width, height, data, options) {
      if (!(data instanceof Uint8Array)) throw new TypeError("data is a Uint8Array");
      const transparent = whole(optionsOf(options).transparent, TTB.TRANSPARENT, "transparent");
      host(OP.BM_BLIT, number(x, "x"), number(y, "y"), number(width, "width"), number(height, "height"), transparent, data);
    },
    text(row, column, text, options) {
      if (typeof text !== "string") throw new TypeError("text is a string");
      const o = optionsOf(options);
      host(OP.BM_TEXT, number(row, "row"), number(column, "column"), optional(o.ink, "ink"), optional(o.paper, "paper"), text.normalize("NFC"));
    },
    // The cells whose look changed, and the cells that could not show
    // exactly what was drawn: what teletext could not show.
    present() {
      const packed = host(OP.BM_PRESENT);
      return { cells: packed % TTB.LOSSY_FACTOR, lossy: Math.floor(packed / TTB.LOSSY_FACTOR) };
    },
  });
}
