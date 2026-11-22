// A full-page animation through the script API (TODO-7297 step 4): eight frames made once, each of 80 by 69 sixels in eight colours with a different colour in every cell and its neighbours, one blitted whole and presented on every tick, so that every cell of every row changes on every frame.
const bm = nt.teletext.bitmap;
const frames = [];
for (let n = 0; n < 8; n++) {
  const frame = new Uint8Array(80 * 69);
  for (let y = 0; y < 69; y++) for (let x = 0; x < 80; x++) frame[y * 80 + x] = (Math.floor(x / 3) + Math.floor(y / 2) + n) % 8;
  frames.push(frame);
}
let t = 0, cells = 0, lossy = 0;
nt.addEventListener("tick", () => {
  bm.blit(0, 0, 80, 69, frames[t % 8]);
  const shown = bm.present();
  cells += shown.cells;
  lossy += shown.lossy;
  t++;
  if (t % 60 === 0) console.log("frames " + t + " cells " + cells + " lossy " + lossy);
});
