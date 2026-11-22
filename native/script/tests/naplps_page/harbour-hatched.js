/*! nt
{ "api": 1, "renderer": "naplps", "keys": ["l", "b"] }
*/
const n = nt.naplps;
const sky = n.colours.define("sky", "#136"), sea = n.colours.define("sea", "#024");
const lamp = n.colours.define("lamp", "#000"), foam = n.colours.define("foam", "#9cf");
// The texture is a definition of section 7.4 (work package N2): until it is built the sea is hatched.
n.backdrop.paint((p) => {
  p.ink(sky).fillRect(0, 0, 480, 300);
  p.wait(5);                                             // half a second, played by the receiver
  p.inkOn(foam, sea).fillStyle("horizontal").fillRect(0, 300, 480, 160);
  p.wait(5);
  p.ink("white").text(2, 4, "HARBOUR LIGHT", { size: "double-height" });
  p.ink(lamp).fillStyle("solid").disc(420, 90, 10);      // painted dark; lit later by the map alone
  p.ink("white").stroke("dashed").moveTo(420, 100).lineBy(0, 200);
});
nt.addEventListener("keydown", (e) => {
  e.preventDefault();
  if (e.key === "l") n.colours.set(lamp, "#fe8");
  else n.colours.blink(lamp, { to: sky, on: 5, off: 5 });
});
