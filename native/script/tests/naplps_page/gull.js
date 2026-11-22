/*! nt
{ "api": 1, "renderer": "naplps", "tick": 2, "keys": ["ArrowUp", "ArrowDown"] }
*/
const n = nt.naplps;
const sky = n.colours.define("sky", "#136");
n.backdrop.paint((p) => p.ink(sky).fillRect(0, 0, 480, 460));
const wing = (dy) => n.symbol({ width: 24, height: 12 }, (p) => p.ink("white").moveBy(0, 6 + dy).polylineBy([[6, -2 * dy], [6, 2 * dy], [6, -2 * dy], [6, 2 * dy]]));
const gull = n.actor({ frames: [wing(4), wing(-4)], x: 60, y: 220, label: "Gull" });
gull.cycle(0, 1, 15);                                    // the receiver flaps the wings
const kiteShape = n.symbol({ width: 16, height: 24 }, (p) => p.ink("red").moveBy(8, 0).fillPolygonBy([[8, 10], [-8, 14], [-8, -14]]));
let kites = [], passed = 0;
nt.addEventListener("tick", (e) => {
  if (nt.keys.down("ArrowUp")) gull.moveBy(0, -6);
  if (nt.keys.down("ArrowDown")) gull.moveBy(0, 6);
  if (e.tick % 60 === 0 && kites.length < 8)
    kites.push(n.actor({ frames: [kiteShape], x: 470, y: 20 + Math.floor(Math.random() * 400), layer: 1 }));
  kites = kites.filter((k) => {
    k.moveBy(-5, 0);
    if (k.overlaps(gull)) { nt.synth.note(0, 40, { ticks: 20 }); passed = 0; }
    if (k.x < -16) { k.remove(); passed++; return false; }
    return true;
  });
  nt.page.title = "Gull: " + passed;
});
nt.synth.start(0, "synth8");
