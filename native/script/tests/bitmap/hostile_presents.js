// A page that asks for the bitmap to be converted as often as it can: a full page of sixteen colours redrawn and presented in a loop that never ends by itself. Each present is bounded work, so the callback's time limit ends the loop as it ends any other; the page is not shown half a frame, and later ticks still run.
const bm = nt.teletext.bitmap;
const frame = new Uint8Array(80 * 69);
let presents = 0;
nt.addEventListener("tick", () => {
  for (;;) {
    for (let at = 0; at < frame.length; at += 97) frame[at] = (frame[at] + 1) % 8;
    bm.blit(0, 0, 80, 69, frame);
    bm.present();
    presents++;
  }
});
