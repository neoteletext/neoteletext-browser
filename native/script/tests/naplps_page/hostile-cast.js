// A page that defines, casts and animates as much as it can on every tick
// (the security review's finding S-06, TODO-7272, for NAPLPS definitions
// and the cast): the longest symbols, DRCS characters and textures, more
// of each than there are codes, more actors than a page may have, each
// with the longest label and the wheel, floods of moves and cycles,
// redefinitions, removals and repaints. Every call past a bound must
// throw; nothing here may stop the page, make the host hold or send more
// than its bounds, or make a batch the link refuses.
const n = nt.naplps;
const steps = [];
// 678 steps: with what a definition writes before them, just under the longest contents.
for (let index = 0; index < 678; index++) steps.push(index % 2 ? [-1, 0] : [1, 0]);
let syms = [], actors = [], thrown = 0, made = 0;
function attempt(fn) {
  try { fn(); made++; } catch (error) {
    if (!(error instanceof RangeError) && !(error instanceof TypeError)) throw error;
    thrown++;
  }
}
function flood() {
  for (const actor of actors.splice(0, 20)) attempt(() => actor.remove());
  for (let index = 0; index < 100; index++) attempt(() => syms.push(n.symbol({ width: 24, height: 12 }, (p) => p.polylineBy(steps))));
  for (let index = 0; index < 100; index++) attempt(() => n.drcs((p) => p.polylineBy(steps)));
  for (let index = 0; index < 6; index++) attempt(() => n.texture({ width: 128, height: 128 }, (p) => p.polylineBy(steps)));
  for (let index = 0; index < 80; index++)
    attempt(() => actors.push(n.actor({ frames: new Array(16).fill(syms[index % syms.length]), x: index, y: index, wheel: index % 3 === 0, detectable: true, label: "x".repeat(64) })));
  for (const actor of actors) {
    for (let index = 0; index < 3; index++) attempt(() => actor.moveBy(1, 1));
    attempt(() => actor.cycle(0, 15, 1));
    attempt(() => { actor.label = "y".repeat(64); });
  }
  for (const symbol of syms.slice(0, 8)) attempt(() => symbol.redefine((p) => p.polylineBy(steps)));
  attempt(() => n.backdrop.repaint((p) => p.fillRect(0, 0, 480, 460)));
  for (const symbol of syms.splice(40)) attempt(() => symbol.remove());
  console.log("flood " + made + " " + thrown);
}
flood();
nt.addEventListener("tick", flood);
