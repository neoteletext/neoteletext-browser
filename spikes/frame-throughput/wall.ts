// Proof C page: a wall of page images. Each message is
// [page: u16][encode microseconds: u32][image bytes]. A frame is decoded
// off-screen and swapped in; if a newer frame for the same page arrives
// while one is decoding, the older one is dropped (newest wins).

interface TauriChannel { onmessage: (data: ArrayBuffer) => void }
declare global {
    interface Window {
        __TAURI__: {
            core: {
                invoke: (command: string, args?: Record<string, unknown>) => Promise<unknown>;
                Channel: new () => TauriChannel;
            };
        };
    }
}

const HEADER_BYTES = 6;
// A wave is the set of frames one tick produces; frames arriving within
// this gap of each other belong to the same wave.
const WAVE_GAP_MS = 100;
const IDLE_FINISH_MS = 1500;

const core = window.__TAURI__.core;
const wall = document.getElementById('wall') as HTMLDivElement;

const run = async () => {
    const [pages, ticks, tickMilliseconds] = (await core.invoke('parameters')) as [number, number, number];
    const images = Array.from({length: pages}, () => {
        const image = document.createElement('img');
        wall.appendChild(image);
        return image;
    });
    const urls: (string | null)[] = new Array(pages).fill(null);
    const pendingVersion: number[] = new Array(pages).fill(0);
    let received = 0, displayed = 0, dropped = 0, firstArrival = 0, lastDisplay = 0, lastArrival = 0;
    let waveStart = 0, waveLastDisplay = 0;
    const waveDurations: number[] = [];

    const channel = new core.Channel();
    channel.onmessage = (data: ArrayBuffer) => {
        const now = performance.now();
        if (!firstArrival) firstArrival = now;
        if (now - lastArrival > WAVE_GAP_MS) {
            if (waveStart) waveDurations.push(waveLastDisplay - waveStart);
            waveStart = now;
        }
        lastArrival = now;
        received++;
        const view = new DataView(data);
        const page = view.getUint16(0);
        const version = ++pendingVersion[page];
        const url = URL.createObjectURL(new Blob([data.slice(HEADER_BYTES)], {type: 'image/png'}));
        const next = new Image();
        next.src = url;
        next.decode().then(() => {
            if (version !== pendingVersion[page]) {
                dropped++;
                URL.revokeObjectURL(url);
                return;
            }
            images[page].src = url;
            const previous = urls[page];
            urls[page] = url;
            if (previous) URL.revokeObjectURL(previous);
            displayed++;
            lastDisplay = waveLastDisplay = performance.now();
        }).catch(() => URL.revokeObjectURL(url));
    };
    await core.invoke('start', {frames: channel, pages, ticks, tickMilliseconds});

    // Finish once frames have stopped arriving.
    await new Promise<void>(resolve => {
        const check = () => (lastArrival && performance.now() - lastArrival > IDLE_FINISH_MS ? resolve() : setTimeout(check, 200));
        setTimeout(check, 500);
    });
    if (waveStart) waveDurations.push(waveLastDisplay - waveStart);
    const seconds = (lastDisplay - firstArrival) / 1000;
    const sorted = [...waveDurations].sort((a, b) => a - b);
    const median = sorted[Math.floor(sorted.length / 2)] ?? 0;
    const worst = sorted[sorted.length - 1] ?? 0;
    await core.invoke('report', {lines: [
        `pages ${pages}, ticks ${ticks}, tick interval ${tickMilliseconds} ms`,
        `received ${received}, displayed ${displayed}, dropped as superseded ${dropped}`,
        `displayed ${(displayed / seconds).toFixed(0)} frames per second over ${seconds.toFixed(2)} s`,
        `time to show a whole wave of ${pages} pages: median ${median.toFixed(0)} ms, worst ${worst.toFixed(0)} ms (${sorted.length} waves)`,
    ]});
};

void run();
export {};
