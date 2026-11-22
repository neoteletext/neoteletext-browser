/*! nt {"api":1} */
setTimeout(() => { let n = 0; for (let i = 0; i < 300; i++) { const s = new EventSource("/chan/ev"); s.close(); n++; } nt.log("opened " + n); }, 0);
