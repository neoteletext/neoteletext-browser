/*! nt
{ "api": 1, "renderer": "regis", "pointer": "press", "keys": ["Enter"] }
*/
const g = nt.regis;
const sky = g.colours.define("sky", { h: 0, l: 25, s: 60 });
const ink = g.colours.define("ink", "#fff");
const hot = g.colours.define("hot", "#f80");
g.drawing({ layer: 0 }).draw((p) => {                        // layer 0 is kept as a raster
  p.ink(sky).fill((q) => q.rect(0, 0, 800, 480));
  p.ink(ink).moveTo(60, 40).text("HARBOUR PATROL", { size: 3 });
});
const caption = g.drawing({ x: 60, y: 420, paintRate: 20 }); // 2,000 work units a second
caption.draw((p) => p.ink(ink).text("Press the button or Enter."));
caption.addEventListener("painted", () => nt.sound.play(1));
const face = (c) => g.symbol((p) => { p.ink(c).fill((q) => q.rect(0, 0, 160, 40)); p.ink(0).moveTo(40, 10).text("START"); });
const button = g.drawing({ x: 320, y: 300, source: face(ink), hover: face(hot), pressed: face(sky), detectable: true, label: "Start" });
const start = () => nt.navigate("106");
button.addEventListener("pointerup", start);
nt.addEventListener("keydown", (e) => { if (e.key === "Enter") { e.preventDefault(); start(); } });
