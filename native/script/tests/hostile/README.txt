The security review's hostile scripts (rdm TODO-7267 and its findings
TODO-7270 to TODO-7279; the review's probe, 22 November 2026), each as the
review wrote it but for the API's names: the spike's nt.after(1, f) is
setTimeout(f, 0) and its nt.every(1, f) is setInterval(f, 0), both one tick.
tests/hostile.rs runs every one and holds each to its bound.
regismem.js and regisstage.js attack the ReGIS API: the spike's
nt.renderer("regis") is the manifest's "renderer" member, and they are
held to their bounds by tests/regis.rs (work package R1, TODO-7272).
