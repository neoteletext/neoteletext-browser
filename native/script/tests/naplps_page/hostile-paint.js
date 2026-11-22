// A page that paints as much as it can, every way it can, on every tick
// (the security review's finding S-06, TODO-7272, for NAPLPS): the
// largest polylines, polygons and images, programs past the word bound,
// repaints, colours and blinks without end. Every call past a bound must
// throw; nothing here may stop the page or make the host hold or send
// more than its bounds.
const n = nt.naplps;
const points = [];
for (let index = 0; index < 1024; index++) points.push([index % 480, index % 460]);
const corners = [];
for (let index = 0; index < 4096; index++) corners.push([(index * 7) % 480, (index * 13) % 460]);
const pixels = new Uint8Array(16384).fill(5);
let thrown = 0, painted = 0;
function attempt(fn) {
  try { fn(); painted++; } catch (error) {
    if (!(error instanceof RangeError) && !(error instanceof TypeError)) throw error;
    thrown++;
  }
}
function flood() {
  for (let index = 0; index < 8; index++) attempt(() => n.colours.define("c" + index + "-" + nt.now(), "#fff"));
  for (let index = 0; index < 40; index++) attempt(() => n.colours.blink(8 + (index % 8), { to: index % 16, on: 1, off: 1 }));
  for (let index = 0; index < 20; index++) attempt(() => n.colours.set(8, "#" + (index % 10) + "00"));
  attempt(() => n.backdrop.repaint((p) => { for (let index = 0; index < 10; index++) p.polyline(points); }));
  attempt(() => n.backdrop.paint((p) => { for (let index = 0; index < 10; index++) p.polyline(points); }));
  attempt(() => n.backdrop.paint((p) => p.fillPolygon(corners).polygon(corners)));
  attempt(() => n.backdrop.paint((p) => p.image(0, 0, 128, 128, pixels).image(128, 0, 128, 128, pixels)));
  attempt(() => n.backdrop.paint((p) => { for (let index = 0; index < 40; index++) p.polyline(points); }));
  attempt(() => n.backdrop.paint((p) => { for (let index = 0; index < 30; index++) p.text(1, 0, "x".repeat(40)); p.wait(600).wait(600); }));
  attempt(() => n.backdrop.paint((p) => p.textAt(0, 0, "y".repeat(1000), { width: 1, height: 1 })));
  console.log("flood " + painted + " " + thrown);
}
flood();
nt.addEventListener("tick", flood);
setInterval(flood, 17);
