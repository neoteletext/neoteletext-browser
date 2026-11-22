/*! nt
{ "api": 1, "renderer": "teletext", "tick": 60, "keys": ["1", "2"], "pointer": "press" }
*/
const tt = nt.teletext;
tt.page.put(1, 2, "HARBOUR NEWS", { ink: "yellow", height: 2 });       // codes at columns 0-1
tt.page.put(4, 1, "1 Shipping forecast", { ink: "cyan" });
tt.page.put(5, 1, "2 Tide tables", { ink: "cyan" });
const amber = tt.colours.define("amber", "#fa0");
tt.overlay.fill(22, 0, 1, 40, { paper: amber, ink: 0 });               // a band; no neighbour touched
const forecast = tt.control(4, 1, 19, { label: "Shipping forecast" }); // hover and pressed looks are the receiver's
forecast.addEventListener("pointerup", () => nt.navigate("102"));
nt.addEventListener("keydown", (e) => {
  if (e.key === "1") { nt.navigate("102"); e.preventDefault(); }       // replace the default (page-number entry)
  else if (e.key === "2") { nt.navigate("103"); e.preventDefault(); }
});
nt.addEventListener("tick", () => {
  const t = new Date();
  tt.page.put(22, 31, t.toTimeString().slice(0, 8), { ink: "white" });
});
