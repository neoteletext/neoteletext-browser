/*! nt
{ "api": 1, "renderer": "regis", "tick": 2, "keys": ["ArrowLeft", "ArrowRight"] }
*/
const g = nt.regis;
const sea = g.colours.define("sea", "#036"), wood = g.colours.define("wood", "#a62"), ink = g.colours.define("ink", "#fff");
g.drawing({ layer: 0 }).draw((p) => p.ink(sea).fill((q) => q.rect(0, 400, 800, 80)));
const boatShape = g.symbol((p) => p.ink(wood).fill((q) => q.polygon([[0, 0], [80, 0], [64, 20], [16, 20]])));
const crateShape = g.symbol((p) => p.ink(wood).pattern(4).fill((q) => q.rect(0, 0, 16, 16)));
const boat = g.drawing({ source: boatShape, layer: 2, x: 360, y: 380, track: true });
const score = g.drawing({ x: 640, y: 16, layer: 3 });
let crates = [], caught = 0;
const show = () => score.draw((p) => p.ink(ink).text("CAUGHT " + caught, { size: 2 }));
show();
nt.addEventListener("tick", (e) => {
  if (nt.keys.down("ArrowLeft")) boat.x = Math.max(0, boat.x - 8);
  if (nt.keys.down("ArrowRight")) boat.x = Math.min(720, boat.x + 8);
  if (e.tick % 40 === 0 && crates.length < 16)
    crates.push(g.drawing({ source: crateShape, layer: 2, x: Math.floor(Math.random() * 780), y: 0 }));
  crates = crates.filter((c) => {
    c.y += 4;
    const hit = c.y >= 364 && c.y <= 396 && c.x + 16 > boat.x && c.x < boat.x + 80;
    if (hit) { caught++; show(); nt.synth.note(0, 72); }
    if (hit || c.y > 480) { c.remove(); return false; }
    return true;
  });
});
nt.synth.start(0, "juno");
