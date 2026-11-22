/*! nt {"api":1} */
for (let k = 0; k < 31; k++) setInterval(() => { for (let i = 0; i < 40; i++) { const c = new AbortController(); fetch("/chan/x", { signal: c.signal }).catch(() => {}); c.abort(); } }, 0);
