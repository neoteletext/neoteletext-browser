/*! nt {"api":1} */
nt.renderer("regis");
const d = nt.regis.drawing({});
function art(p) { for (let i = 0; i < 1000; i++) p.rect(i % 700, i % 400, 30000, 30000, true); }
for (let t = 0; t < 31; t++) setInterval(() => { d.draw(art); }, 0);
