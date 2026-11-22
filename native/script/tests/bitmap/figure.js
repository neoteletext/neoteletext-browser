// A figure every cell of which holds at most two colours, so that a receiver of the level it was drawn for shows it exactly (tests/bitmap.rs). LEVEL_1 is set in front by the test.
const bm = nt.teletext.bitmap;
const amber = LEVEL_1 ? 3 : nt.teletext.colours.define("amber", "#fa0");
bm.rect(4, 6, 20, 12, 1);
bm.line(2, 68, 79, 68, 7);
if (!LEVEL_1) {
  bm.rect(40, 30, 17, 10, amber);
  bm.rect(60, 3, 6, 6, 4);
  bm.rect(62, 5, 2, 2, 6);
}
bm.text(12, 10, "SIXELS", LEVEL_1 ? { ink: 2 } : { ink: 3, paper: 4 });
let ticks = 0;
nt.addEventListener("tick", () => {
  ticks++;
  if (ticks === 1) {
    const shown = bm.present();
    console.log("first " + shown.cells + " " + shown.lossy + " " + nt.budget().triplets.used);
  } else if (!LEVEL_1 && ticks === 3) {
    // The overlay lies over the bitmap: two cells of row 4 inside the red block.
    nt.teletext.overlay.put(4, 5, "OK", { ink: 7, paper: 2 });
    console.log("overlay " + nt.budget().triplets.used);
  }
});
