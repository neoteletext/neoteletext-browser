const pad = (value: number, width: number) => String(value).padStart(width, '0');
const MS_DIGITS = 3;
const CLOCK_DIGITS = 2;

// 14:03:27.415 — wall-clock time with milliseconds, as consoles show it.
export const formatClock = (epochMs: number) => {
    const date = new Date(epochMs);
    return `${pad(date.getHours(), CLOCK_DIGITS)}:${pad(date.getMinutes(), CLOCK_DIGITS)}:${pad(date.getSeconds(), CLOCK_DIGITS)}.${pad(date.getMilliseconds(), MS_DIGITS)}`;
};

const KIB = 1024;
const MIB = KIB * KIB;

export const formatBytes = (bytes: number | null) => {
    if (bytes === null) return '—';
    if (bytes < KIB) return `${bytes} B`;
    if (bytes < MIB) return `${(bytes / KIB).toFixed(1)} KB`;
    return `${(bytes / MIB).toFixed(2)} MB`;
};

// WebKit rounds performance.now() to whole milliseconds, so 0 there means
// "under a millisecond", not "free".
export const formatMs = (ms: number) => {
    if (ms === 0) return '< 1 ms';
    return ms < 10 ? `${ms.toFixed(2)} ms` : `${Math.round(ms)} ms`;
};
