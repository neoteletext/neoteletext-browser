/*! nt {"api":1} */
nt.renderer("regis");
let made = 0;
function art(p) { for (let i = 0; i < 1000; i++) p.rect(i % 700, i % 400, 30000, 30000, true); }
for (let t = 0; t < 31; t++) setInterval(() => { if (made < 2048) { nt.regis.drawing({ x: 1, y: 1 }).draw(art); made++; } else if (made === 2048) { nt.log(nt.budget()); made++; } }, 0);
