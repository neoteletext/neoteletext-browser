//! Measurements, which print their numbers and assert nothing: how long a
//! slice of fuel takes across kinds of work (what `FUEL_SLICE` was chosen
//! by), what the engine costs to compile and a context to start, and what
//! a step of a trivial page costs. Run by hand, in a release build, on a
//! machine with processors to spare:
//!   cargo test --release -p neoteletext-script --test measure -- --ignored --nocapture
mod common;

use std::time::Instant;

use common::{Opens as _, claim, limited as container, opening, tick};
use protocol::script::ContextId;

const WORK: &[(&str, &str)] = &[
    ("empty loop", "for(;;){}"),
    ("arithmetic", "let x=0; for(let i=0;;i++){ x+=i*i%7; }"),
    ("copyWithin 40MiB", "const a=new Uint8Array(40*1024*1024); for(;;){ a.copyWithin(0,1); }"),
    ("copyWithin back", "const a=new Uint8Array(40*1024*1024); for(;;){ a.copyWithin(1,0); }"),
    ("fill 40MiB", "const a=new Uint8Array(40*1024*1024); for(;;){ a.fill(7); }"),
    ("int32 fill", "const a=new Int32Array(8*1024*1024); for(;;){ a.fill(7); }"),
    ("float64 sort", "const a=new Float64Array(2*1024*1024); for(let i=0;i<a.length;i++)a[i]=(i*7919)%10007; for(;;){ a.sort(); a.reverse(); }"),
    ("typed set", "const a=new Uint8Array(20*1024*1024), b=new Uint8Array(20*1024*1024); for(;;){ a.set(b); }"),
    ("typed indexOf", "const a=new Uint8Array(40*1024*1024); for(;;){ a.indexOf(9); }"),
    ("string double", "let s='x'; for(;;){ s=s+s; if(s.length>1<<24) s='x'; }"),
    ("string indexOf", "const s='ab'.repeat(8*1024*1024); for(;;){ s.indexOf('zz'); }"),
    ("string repeat", "for(;;){ 'abc'.repeat(4*1024*1024); }"),
    ("regex backtrack", "const s='a'.repeat(40); for(;;){ /(a+)+b/.test(s); }"),
    ("regex scan", "const s='ab'.repeat(4*1024*1024); for(;;){ /zz/.test(s); }"),
    ("array sort", "const a=[]; for(let i=0;i<200000;i++)a.push((i*7919)%10007); for(;;){ a.sort((x,y)=>x-y); a.reverse(); }"),
    ("array sort default", "const a=[]; for(let i=0;i<200000;i++)a.push((i*7919)%10007); for(;;){ a.sort(); a.reverse(); }"),
    ("json", "const a=[]; for(let i=0;i<100000;i++)a.push({i,s:'x'+i}); for(;;){ JSON.parse(JSON.stringify(a)); }"),
    ("array join", "const a=new Array(1000000).fill('ab'); for(;;){ a.join(','); }"),
    ("array fill/includes", "const a=new Array(4000000).fill(1); for(;;){ a.includes(2); a.indexOf(3); }"),
    ("map set", "const m=new Map(); for(let i=0;;i++){ m.set(i&0xfffff,i); }"),
    ("alloc", "const keep=[]; for(;;){ keep.push(new Array(10000).fill(1)); }"),
    ("bigint", "let b=1n; for(;;){ b=b*b+3n; if(b>1n<<200000n) b=3n; }"),
    ("bigint tostring", "let b=(1n<<200000n)-1n; for(;;){ b.toString(); }"),
    ("normalize", "const s='e\\u0301'.repeat(1000000); for(;;){ s.normalize('NFC'); }"),
    ("toUpperCase", "const s='abcdefgh'.repeat(1000000); for(;;){ s.toUpperCase(); }"),
    ("localeCompare", "const s='abcdefgh'.repeat(1000000), t=s+'x'; for(;;){ s.localeCompare(t); }"),
    ("recursion", "function f(n){ return n? f(n-1)+1:0 } for(;;){ try { f(1e9) } catch(e){} }"),
    ("budget", "for(;;){ nt.budget(); }"),
    ("log", "for(;;){ console.log('x'.repeat(390)); }"),
    ("shared", "const v='x'.repeat(16000); for(;;){ nt.shared.set('k',v); nt.shared.get('k'); }"),
    ("random", "const a=new Uint8Array(65536); for(;;){ crypto.getRandomValues(a); }"),
    ("promises", "function spin(){ return Promise.resolve().then(spin); } spin();"),
    ("structuredClone", "const a=[]; for(let i=0;i<10000;i++)a.push({i,s:'x'+i}); for(;;){ structuredClone(a); }"),
    ("gc churn", "for(;;){ let a=[]; for(let i=0;i<10000;i++) a.push({i}); }"),
    ("date", "for(;;){ new Date().toString(); }"),
    ("atob", "const s=btoa('x'.repeat(3000000)); for(;;){ atob(s); }"),
    ("escape", "const s='é'.repeat(1000000); for(;;){ encodeURIComponent(s); }"),
];

#[test]
#[ignore = "a measurement that prints its numbers and asserts nothing; run by hand in a release build on a quiet machine"]
fn compile() {
    for _ in 0..3 {
        let began = Instant::now();
        let engine = neoteletext_script::Engine::new().unwrap();
        let rss = std::fs::read_to_string("/proc/self/statm").unwrap();
        println!("compile {:?} ({:?}) statm {}", began.elapsed(), engine.compile_time, rss.trim());
    }
}

#[test]
#[ignore = "a measurement that prints its numbers and asserts nothing; run by hand in a release build on a quiet machine"]
fn throughput() {
    for (name, source) in [
        ("nothing", ""),
        ("loop 3e5, globals", "let x=0; for(let i=0;i<3e5;i++){ x+=i*i%7; }"),
        ("loop 3e5, locals", "(function(){ let x=0; for(let i=0;i<3e5;i++){ x+=i*i%7; } })();"),
        ("objects 3e4", "(function(){ const a=[]; for(let i=0;i<3e4;i++){ a.push({i, s: 'k'+i}); } a.sort((p,q)=>q.i-p.i); })();"),
        ("copy 16x8MiB", "const a=new Uint8Array(8*1024*1024); for(let i=0;i<16;i++) a.copyWithin(0,1);"),
        ("int32 fill 8x32MiB", "const a=new Int32Array(8*1024*1024); for(let i=0;i<8;i++) a.fill(i);"),
        ("string search", "const s='ab'.repeat(4*1024*1024); for(let i=0;i<16;i++) s.indexOf('zz');"),
    ] {
        let best = (0..5)
            .map(|_| {
                let mut container = container();
                let (opened, took) = common::timed(|| container.open_with(opening(ContextId::Page), source.as_bytes()));
                (took.running, container.costs().slices, opened.is_ok())
            })
            .min()
            .unwrap();
        let fuel_a_millisecond = u128::from(best.1) * u128::from(neoteletext_script::limits::FUEL_SLICE) * 1000 / best.0.as_micros().max(1);
        println!("{name:20} ran {:?} in {} slices ({fuel_a_millisecond} fuel a millisecond; finished: {})", best.0, best.1, best.2);
    }
}

#[test]
#[ignore = "a measurement that prints its numbers and asserts nothing; run by hand in a release build on a quiet machine"]
fn slices() {
    for (name, source) in WORK {
        let mut container = container();
        let began = Instant::now();
        let result = container.open_with(opening(ContextId::Page), source.as_bytes());
        let ran = began.elapsed();
        let costs = container.costs();
        println!("{name:22} ran {:>9.3?} longest slice {:>10.3?} stop {:?}", ran, costs.longest_slice, result.err().map(|error| error.to_string()));
    }
}

#[test]
#[ignore = "a measurement that prints its numbers and asserts nothing; run by hand in a release build on a quiet machine"]
fn trivial_page() {
    println!("module {} bytes", neoteletext_script::machine::MODULE.len());
    for _ in 0..3 {
        let mut container = container();
        let mut open = opening(ContextId::Page);
        claim(&mut open, &[]);
        let began = Instant::now();
        container.open_with(open, b"let n = 0; nt.addEventListener('tick', () => { n++; });").unwrap();
        let opened = began.elapsed();
        let mut steps: Vec<std::time::Duration> = (0..600_u32)
            .map(|at| {
                let began = Instant::now();
                container.step(&tick(at, u64::from(at))).unwrap();
                began.elapsed()
            })
            .collect();
        steps.sort();
        let (linear, heap) = container.memory();
        println!(
            "container {:?}, context with prelude and script {opened:?}; a step with one tick listener: median {:?}, ninth decile {:?}, worst {:?}; memory {linear} bytes, heap {heap}, host {}",
            container.costs().container,
            steps[300],
            steps[540],
            steps[599],
            container.host_bytes()
        );
        let mut idle = container;
        idle.close(ContextId::Page);
        idle.open_with(opening(ContextId::Page), b";").unwrap();
        let mut steps: Vec<std::time::Duration> = (0..600_u32)
            .map(|at| {
                let began = Instant::now();
                idle.step(&tick(at + 1000, u64::from(at) + 1000)).unwrap();
                began.elapsed()
            })
            .collect();
        steps.sort();
        println!("a step with no listener: median {:?}, worst {:?}", steps[300], steps[599]);
    }
}
