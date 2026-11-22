// A page that fights for the page's triplets and for its rows on every tick: sixteen defined colours in every cell of the bitmap, the overlay and controls asked for on every row, the bitmap's rows moved and released, the page plane written under it, substitution switched. The host must stage no more than the page's 208 local triplets, hold no more than a page, and never show a row half the bitmap's and half the page's.
const tt = nt.teletext, bm = tt.bitmap;
for (let entry = 0; entry < 16; entry++) tt.colours.define("c" + entry, "#" + (entry * 273).toString(16).padStart(3, "0"));
const frame = new Uint8Array(80 * 69);
let t = 0, refused = 0, lossy = 0;
const attempt = (work) => { try { work(); } catch (error) { if (error instanceof RangeError || error instanceof TypeError) refused++; else throw error; } };
nt.addEventListener("tick", () => {
  for (let at = 0; at < frame.length; at++) frame[at] = 16 + ((at * 7 + (at >> 3) + t) % 16);
  bm.blit(0, 0, 80, 69, frame);
  for (let row = 1; row <= 23; row++) {
    attempt(() => tt.overlay.put(row, (row + t) % 38, "é", { ink: (row + t) % 32, paper: (row * 3) % 32, flash: true }));
    attempt(() => tt.control(row, (row * 5 + t) % 40, 1));
    attempt(() => tt.page.put(row, 2, "page", { ink: "red" }));
    attempt(() => tt.page.codes(row, 0, new Uint8Array(40).fill(0x7f)));
  }
  attempt(() => tt.page.settings({ substitution: t % 3 === 0 }));
  attempt(() => bm.rows(1 + (t % 23), 1 + ((t * 5) % 23)));
  attempt(() => { lossy += bm.present().lossy; });
  if (t % 4 === 3) attempt(() => bm.release());
  attempt(() => tt.page.clear());
  attempt(() => { lossy += bm.present().lossy; });
  console.log("tick " + t + " used " + tt.overlay.triplets.used + " refused " + (refused > 0) + " lossy " + (lossy > 0));
  t++;
});
