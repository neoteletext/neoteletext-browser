// A bitmap animation (script-api.md section 5.7; TODO-7297's "done"): bars that rise and fall, a ball that bounces across them, a band of a defined colour and a score in text, redrawn whole and presented on every tick. It logs the tick, the cells that changed and the cells that lost detail.
const bm = nt.teletext.bitmap;
const sky = nt.teletext.colours.define("sky", "#48c");
let t = 0;
nt.addEventListener("tick", () => {
  bm.clear(0);
  bm.rect(0, 0, 80, 6, sky);
  for (let bar = 0; bar < 8; bar++) {
    const rise = (bar * 7 + t) % 20;
    bm.rect(bar * 10, 60 - rise, 8, 9 + rise, 1 + (bar % 7));
  }
  const x = 4 + ((t * 3) % 70);
  const y = 10 + Math.abs(20 - (t % 40));
  bm.rect(x, y, 6, 6, 7);
  bm.line(x, y, x + 5, y + 5, 1);
  bm.text(1, 14, "SCORE " + String(t).padStart(3, "0"), { ink: 3, paper: 4 });
  const shown = bm.present();
  console.log(t + " " + shown.cells + " " + shown.lossy);
  t++;
});
