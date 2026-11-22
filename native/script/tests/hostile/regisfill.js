/*! nt {"api":1,"renderer":"regis"} */
// Work package R1's own, beside the review's regismem.js: it does what
// that script does and does not give up. Its drawings are hidden, so that
// no commit is refused for the work of painting them, and it catches what
// is thrown, so that it is not stopped for failing: it goes on until
// every bound of the ReGIS tables has refused it, and says which did.
const points = [];
for (let i = 0; i < 1024; i++) points.push([(i * 37) % 700, (i * 91) % 400]);
function art(p) { for (let i = 0; i < 6; i++) p.polyline(points); }
const refused = new Set();
let made = 0, defined = 0;
setInterval(() => {
  for (let n = 0; n < 40; n++) {
    let d;
    try { d = nt.regis.drawing({ visible: false, label: "L".repeat(64) }); made++; }
    catch (error) { refused.add(error.message.split(":")[0]); break; }
    if (refused.has("REGIS_MAX_TOTAL_CONTENTS_BYTES")) continue;
    try { d.draw(art); defined++; }
    catch (error) { refused.add(error.message.split(":")[0]); }
  }
  if (refused.has("REGIS_MAX_DRAWINGS") && made >= 0) { nt.log("made " + made + " defined " + defined + " refused by " + [...refused].sort().join(",")); made = -1; }
}, 0);
