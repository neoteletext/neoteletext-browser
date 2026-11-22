/*! nt {"api":1} */
for (let k = 0; k < 31; k++) setInterval(() => { for (let i = 0; i < 60; i++) { const s = new EventSource("/chan/ev"); s.close(); } }, 0);
