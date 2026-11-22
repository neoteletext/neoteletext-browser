/*! nt
{ "api": 1, "renderer": "teletext", "tick": 6, "keys": ["ArrowLeft", "ArrowRight", " "] }
*/
const tt = nt.teletext;
const sea = tt.colours.define("sea", "#024");
tt.overlay.fill(3, 0, 20, 40, { paper: sea });
const hull = tt.block(3, 2).sixels(["011110", "111111", "111111", "100001", "100001", "000000"], { ink: 7 });
const rock = tt.block(2, 1).sixels(["0110", "1111", "1111"], { ink: 3 });
const boat = tt.sprite({ frames: [hull], row: 21, column: 18, layer: 2, label: "Boat" });
const rocks = [];
let score = 0, over = false;
nt.addEventListener("tick", () => {
  if (over) return;
  if (nt.keys.down("ArrowLeft") && boat.column > 0) boat.moveBy(0, -1);
  if (nt.keys.down("ArrowRight") && boat.column < 37) boat.moveBy(0, 1);
  if (Math.random() < 0.2 && rocks.length < 12)
    rocks.push(tt.sprite({ frames: [rock], row: 3, column: Math.floor(Math.random() * 38), layer: 1 }));
  for (const r of rocks.splice(0)) {
    if (r.overlaps(boat)) { over = true; nt.synth.note(0, 36, { ticks: 30 }); tt.colours.set(sea, "#400"); }
    if (r.row >= 22) { r.remove(); score++; } else { r.moveBy(1, 0); rocks.push(r); }
  }
  tt.page.put(1, 30, "SCORE " + String(score).padStart(3, "0"), { ink: "white" });
});
nt.addEventListener("keydown", (e) => { e.preventDefault(); if (e.key === " " && over) nt.navigate(nt.page.number); });
nt.synth.start(0, "sid");
