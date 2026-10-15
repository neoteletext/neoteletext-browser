import {ASPECT_HEIGHT, ASPECT_WIDTH} from './thumbnailSize';

// Sample picture data handed to each egui image. Half of 1080p keeps twelve
// pictures at 2 MB each; egui's zoom shows real texels up to 2× at the
// slider's widest setting.
export const FRAME_WIDTH = 960;
export const FRAME_HEIGHT = FRAME_WIDTH * ASPECT_HEIGHT / ASPECT_WIDTH;

type Painter = (context: CanvasRenderingContext2D, width: number, height: number, random: () => number) => void;

// mulberry32: small seeded PRNG so every visit draws the same frames.
const seededRandom = (seed: number) => () => {
    seed = (seed + 0x6d2b79f5) | 0;
    let t = Math.imul(seed ^ (seed >>> 15), 1 | seed);
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
};

const TAU = Math.PI * 2;

const truchet: Painter = (context, width, height, random) => {
    const cell = width / 24;
    context.fillStyle = '#10243f';
    context.fillRect(0, 0, width, height);
    context.lineWidth = cell * 0.22;
    context.lineCap = 'round';
    for (let y = 0; y < height; y += cell) {
        for (let x = 0; x < width; x += cell) {
            context.strokeStyle = `hsl(${30 + (x / width) * 30}, 95%, ${55 + (y / height) * 15}%)`;
            context.beginPath();
            if (random() < 0.5) {
                context.arc(x, y, cell / 2, 0, TAU / 4);
                context.moveTo(x + cell, y + cell / 2);
                context.arc(x + cell, y + cell, cell / 2, -TAU / 4, -TAU / 2, true);
            } else {
                context.arc(x + cell, y, cell / 2, TAU / 4, TAU / 2);
                context.moveTo(x + cell / 2, y + cell);
                context.arc(x, y + cell, cell / 2, -TAU / 2, -TAU / 4, true);
            }
            context.stroke();
        }
    }
};

// Plasma is computed on a small grid and smoothed up, which is both fast
// and gives the soft, cellular look of the original shader.
const PLASMA_COLUMNS = 192;
const primordialSoup: Painter = (context, width, height) => {
    const rows = PLASMA_COLUMNS * height / width;
    const small = document.createElement('canvas');
    small.width = PLASMA_COLUMNS;
    small.height = rows;
    const smallContext = small.getContext('2d');
    if (!smallContext) return;
    const image = smallContext.createImageData(PLASMA_COLUMNS, rows);
    for (let y = 0; y < rows; y++) {
        for (let x = 0; x < PLASMA_COLUMNS; x++) {
            const u = x / PLASMA_COLUMNS * 6;
            const v = y / rows * 6;
            const value = Math.sin(u * 1.3 + Math.sin(v * 1.7)) + Math.sin(v * 1.1 - Math.cos(u * 0.9)) + Math.sin(Math.hypot(u - 3, v - 3) * 2.2);
            const shade = (value + 3) / 6;
            const index = (y * PLASMA_COLUMNS + x) * 4;
            image.data[index] = 40 + shade * 60;
            image.data[index + 1] = 120 + shade * 120;
            image.data[index + 2] = 90 + Math.pow(shade, 3) * 140;
            image.data[index + 3] = 255;
        }
    }
    smallContext.putImageData(image, 0, 0);
    context.imageSmoothingQuality = 'high';
    context.drawImage(small, 0, 0, width, height);
};

const ziggurat: Painter = (context, width, height) => {
    const steps = 14;
    for (let step = 0; step < steps; step++) {
        const inset = step / steps;
        context.fillStyle = `hsl(${350 - step * 9}, 70%, ${18 + step * 4.5}%)`;
        const stepWidth = width * (1 - inset * 0.85);
        const stepHeight = height * (1 - inset * 0.95);
        context.fillRect((width - stepWidth) / 2, height - stepHeight, stepWidth, stepHeight);
    }
};

const tunnel: Painter = (context, width, height) => {
    const rings = 28;
    const centerX = width * 0.58;
    const centerY = height * 0.46;
    for (let ring = rings; ring > 0; ring--) {
        const depth = ring / rings;
        const radius = Math.pow(depth, 2.2) * width * 0.8;
        context.fillStyle = ring % 2 === 0 ? `hsl(265, 60%, ${8 + depth * 30}%)` : `hsl(190, 80%, ${10 + depth * 42}%)`;
        context.beginPath();
        context.arc(centerX + (width / 2 - centerX) * depth, centerY + (height / 2 - centerY) * depth, radius, 0, TAU);
        context.fill();
    }
};

const terrain: Painter = (context, width, height, random) => {
    const sky = context.createLinearGradient(0, 0, 0, height);
    sky.addColorStop(0, '#f6c28b');
    sky.addColorStop(1, '#f3e3cf');
    context.fillStyle = sky;
    context.fillRect(0, 0, width, height);
    const ridges = 6;
    for (let ridge = 0; ridge < ridges; ridge++) {
        const baseline = height * (0.42 + ridge * 0.1);
        const phase = random() * TAU;
        context.fillStyle = `hsl(${215 + ridge * 4}, ${30 + ridge * 6}%, ${62 - ridge * 9}%)`;
        context.beginPath();
        context.moveTo(0, height);
        for (let x = 0; x <= width; x += width / 160) {
            const t = x / width;
            const y = baseline - Math.sin(t * 7 + phase) * height * 0.05 - Math.sin(t * 19 + phase * 2) * height * 0.018;
            context.lineTo(x, y);
        }
        context.lineTo(width, height);
        context.fill();
    }
};

const particles: Painter = (context, width, height, random) => {
    context.fillStyle = '#05070d';
    context.fillRect(0, 0, width, height);
    const count = 1400;
    for (let particle = 0; particle < count; particle++) {
        const angle = random() * TAU;
        const distance = Math.pow(random(), 0.6) * width * 0.45;
        const x = width / 2 + Math.cos(angle + distance / 260) * distance;
        const y = height / 2 + Math.sin(angle + distance / 260) * distance * 0.55;
        context.fillStyle = `hsla(${200 + distance / 6}, 90%, 70%, ${1 - distance / (width * 0.5)})`;
        context.beginPath();
        context.arc(x, y, 1 + random() * 3.5, 0, TAU);
        context.fill();
    }
};

// SMPTE ECR 1-1978 colour bars: seven 75% bars over the reverse-blue row.
const SMPTE_TOP = ['#c0c0c0', '#c0c000', '#00c0c0', '#00c000', '#c000c0', '#c00000', '#0000c0'];
const SMPTE_MIDDLE = ['#0000c0', '#131313', '#c000c0', '#131313', '#00c0c0', '#131313', '#c0c0c0'];
const crtMonitor: Painter = (context, width, height) => {
    const bar = width / SMPTE_TOP.length;
    SMPTE_TOP.forEach((color, index) => {
        context.fillStyle = color;
        context.fillRect(index * bar, 0, bar + 1, height * 0.67);
    });
    SMPTE_MIDDLE.forEach((color, index) => {
        context.fillStyle = color;
        context.fillRect(index * bar, height * 0.67, bar + 1, height * 0.08);
    });
    context.fillStyle = '#101010';
    context.fillRect(0, height * 0.75, width, height * 0.25);
    context.fillStyle = 'rgba(0, 0, 0, 0.18)';
    for (let line = 0; line < height; line += 6) context.fillRect(0, line, width, 2);
};

const errorScreen: Painter = (context, width, height, random) => {
    context.fillStyle = '#1b1020';
    context.fillRect(0, 0, width, height);
    context.font = `800 ${height * 0.55}px system-ui, sans-serif`;
    context.textAlign = 'center';
    context.textBaseline = 'middle';
    const slices = 18;
    const sliceHeight = height / slices;
    for (let slice = 0; slice < slices; slice++) {
        context.save();
        context.beginPath();
        context.rect(0, slice * sliceHeight, width, sliceHeight);
        context.clip();
        const shift = (random() - 0.5) * width * 0.06;
        context.fillStyle = '#ff3d7f';
        context.fillText('408', width / 2 + shift - 12, height / 2);
        context.fillStyle = '#3df2ff';
        context.fillText('408', width / 2 + shift + 12, height / 2);
        context.fillStyle = '#f6f0ff';
        context.fillText('408', width / 2 + shift, height / 2);
        context.restore();
    }
};

const waveform: Painter = (context, width, height, random) => {
    context.fillStyle = '#0d1a1a';
    context.fillRect(0, 0, width, height);
    const bars = 96;
    const barWidth = width / bars;
    for (let bar = 0; bar < bars; bar++) {
        const t = bar / bars;
        const amplitude = (0.25 + 0.75 * Math.abs(Math.sin(t * 9) * Math.sin(t * 2.3))) * (0.6 + random() * 0.4);
        const barHeight = amplitude * height * 0.7;
        context.fillStyle = `hsl(${150 + t * 50}, 70%, 58%)`;
        context.fillRect(bar * barWidth + barWidth * 0.2, (height - barHeight) / 2, barWidth * 0.6, barHeight);
    }
};

const spirograph: Painter = (context, width, height) => {
    context.fillStyle = '#f4efe6';
    context.fillRect(0, 0, width, height);
    const squares = 60;
    context.lineWidth = 3;
    context.translate(width / 2, height / 2);
    for (let square = 0; square < squares; square++) {
        const size = height * 0.9 * (1 - square / squares);
        context.strokeStyle = `hsl(${square * 6}, 65%, 45%)`;
        context.rotate(TAU / 96);
        context.strokeRect(-size / 2, -size / 2, size, size);
    }
    context.setTransform(1, 0, 0, 1, 0, 0);
};

const hyperspace: Painter = (context, width, height, random) => {
    context.fillStyle = '#020412';
    context.fillRect(0, 0, width, height);
    const streaks = 500;
    context.lineCap = 'round';
    for (let streak = 0; streak < streaks; streak++) {
        const angle = random() * TAU;
        const start = random() * width * 0.1;
        const length = random() * width * 0.4;
        context.strokeStyle = `hsla(${210 + random() * 40}, 100%, ${70 + random() * 30}%, ${0.3 + random() * 0.7})`;
        context.lineWidth = 1 + random() * 3;
        context.beginPath();
        context.moveTo(width / 2 + Math.cos(angle) * start, height / 2 + Math.sin(angle) * start);
        context.lineTo(width / 2 + Math.cos(angle) * (start + length), height / 2 + Math.sin(angle) * (start + length));
        context.stroke();
    }
};

const dusk: Painter = (context, width, height) => {
    const sky = context.createLinearGradient(0, 0, 0, height);
    sky.addColorStop(0, '#1d2b64');
    sky.addColorStop(0.55, '#c0587e');
    sky.addColorStop(1, '#f8b26a');
    context.fillStyle = sky;
    context.fillRect(0, 0, width, height);
    context.fillStyle = '#fff3d6';
    context.beginPath();
    context.arc(width * 0.68, height * 0.7, height * 0.16, 0, TAU);
    context.fill();
    context.fillStyle = '#151a3a';
    context.fillRect(0, height * 0.78, width, height * 0.22);
    context.fillStyle = 'rgba(255, 243, 214, 0.5)';
    for (let line = 0; line < 9; line++) {
        const lineWidth = height * (0.28 - line * 0.025);
        context.fillRect(width * 0.68 - lineWidth / 2, height * (0.8 + line * 0.022), lineWidth, height * 0.006);
    }
};

export interface Frame {
    id: string;
    title: string;
    template: string;
    durationSeconds: number;
    seed: number;
    paint: Painter;
}

// Titles are the template names from the original gallery.
export const FRAMES: Frame[] = [
    {id: 'truchet', title: 'Truchet 3', template: 'canvas', durationSeconds: 14, seed: 11, paint: truchet},
    {id: 'soup', title: 'Primordial Soup', template: 'glsl', durationSeconds: 30, seed: 2, paint: primordialSoup},
    {id: 'ziggurat', title: 'Ziggurat', template: 'canvas', durationSeconds: 9, seed: 3, paint: ziggurat},
    {id: 'tunnel', title: 'Tunnel', template: 'three.js', durationSeconds: 21, seed: 4, paint: tunnel},
    {id: 'terrain', title: 'Terrain', template: 'three.js', durationSeconds: 45, seed: 5, paint: terrain},
    {id: 'particles', title: 'Particles', template: 'three.js', durationSeconds: 18, seed: 6, paint: particles},
    {id: 'crt', title: 'CRT Monitor', template: 'webgl', durationSeconds: 12, seed: 7, paint: crtMonitor},
    {id: 'error', title: 'Challenge 408 Error', template: 'canvas', durationSeconds: 6, seed: 8, paint: errorScreen},
    {id: 'waveform', title: 'Waveform', template: 'canvas', durationSeconds: 64, seed: 9, paint: waveform},
    {id: 'spiral', title: 'Square Spiral', template: 'svg', durationSeconds: 15, seed: 10, paint: spirograph},
    {id: 'hyperspace', title: 'Hyperspace', template: 'webgl', durationSeconds: 8, seed: 12, paint: hyperspace},
    {id: 'dusk', title: 'Dusk', template: 'svg', durationSeconds: 27, seed: 13, paint: dusk},
];

export interface Pixels {
    width: number;
    height: number;
    rgba: Uint8Array;
}

// Paints a frame and returns its raw RGBA, the form egui uploads as a texture.
export const renderPixels = (frame: Frame): Pixels => {
    const canvas = document.createElement('canvas');
    canvas.width = FRAME_WIDTH;
    canvas.height = FRAME_HEIGHT;
    const context = canvas.getContext('2d', {willReadFrequently: true});
    if (!context) throw new Error('Canvas 2D is unavailable');
    frame.paint(context, FRAME_WIDTH, FRAME_HEIGHT, seededRandom(frame.seed));
    const image = context.getImageData(0, 0, FRAME_WIDTH, FRAME_HEIGHT);
    return {width: FRAME_WIDTH, height: FRAME_HEIGHT, rgba: new Uint8Array(image.data.buffer)};
};

export const formatDuration = (seconds: number) =>
    `${String(Math.floor(seconds / 60)).padStart(2, '0')}:${String(seconds % 60).padStart(2, '0')}`;
