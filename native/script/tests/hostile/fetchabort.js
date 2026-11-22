/*! nt {"api":1} */
setTimeout(() => { let n = 0; for (let i = 0; i < 300; i++) { const c = new AbortController(); fetch("/chan/x", { signal: c.signal }).catch(() => {}); c.abort(); n++; } nt.log("issued " + n); }, 0);
