// Display helpers for .t42 addresses. Validation lives in Rust (t42.rs);
// these only shape what the chrome and tab page show.

export const SAMPLE_SRC = 'https://example.com/samples/thumbnail-wall.t42';

const EXTENSION = /\.t42$/i;

export const fileTitle = (src: string) => {
    try {
        const name = decodeURIComponent(new URL(src).pathname.split('/').pop() ?? '');
        return name.replace(EXTENSION, '') || src;
    } catch {
        return src;
    }
};

// A stable 2×2 colour mosaic per address, used as the tab's icon.
const MOSAIC_CELLS = 4;
const HUE_STEP = 47;
const FULL_TURN = 360;
const HASH_SEED = 2166136261;
const HASH_PRIME = 16777619;

// FNV-1a, 32-bit: small, stable, and spreads similar addresses apart.
const hash = (value: string) => {
    let state = HASH_SEED;
    for (let index = 0; index < value.length; index++) {
        state = Math.imul(state ^ value.charCodeAt(index), HASH_PRIME);
    }
    return state >>> 0;
};

export const mosaicHues = (src: string) => {
    const base = hash(src) % FULL_TURN;
    return Array.from({length: MOSAIC_CELLS}, (_, cell) => (base + cell * HUE_STEP) % FULL_TURN);
};
