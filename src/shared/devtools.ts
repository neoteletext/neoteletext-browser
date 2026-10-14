// What a tab reports to the chrome's devtools panel. Tabs are untrusted, so
// the chrome parses every entry with parseEntry rather than casting it.

export interface ConsoleEntry {
    kind: 'console';
    level: 'log' | 'info' | 'warn' | 'error';
    message: string;
    time: number;
}

export interface NetworkEntry {
    kind: 'network';
    method: string;
    url: string;
    status: number | null;
    bytes: number | null;
    durationMs: number;
    error: string | null;
    time: number;
}

export interface StorageEntry {
    kind: 'storage';
    cookies: [string, string][];
    local: [string, string][];
    time: number;
}

export interface ImageEntry {
    kind: 'image';
    id: string;
    title: string;
    cssWidth: number;
    cssHeight: number;
    pixelWidth: number;
    pixelHeight: number;
    zoom: number;
    frameMs: number;
    pointer: [number, number] | null;
    transform: string;
    time: number;
}

export interface LifecycleEntry {
    kind: 'lifecycle';
    state: 'ready';
    time: number;
}

export type DevtoolsEntry = ConsoleEntry | NetworkEntry | StorageEntry | ImageEntry | LifecycleEntry;

export interface TabReport {
    tabId: string;
    entry: DevtoolsEntry;
}

// Display limits: long values are cut, not rejected, so a noisy tab still
// shows what it sent.
export const MAX_TEXT = 2000;
const MAX_PAIRS = 100;
const LEVELS = ['log', 'info', 'warn', 'error'] as const;

type Record_ = Record<string, unknown>;
const isRecord = (value: unknown): value is Record_ => typeof value === 'object' && value !== null && !Array.isArray(value);
const text = (value: unknown) => (typeof value === 'string' ? value.slice(0, MAX_TEXT) : null);
const number = (value: unknown) => (typeof value === 'number' && Number.isFinite(value) ? value : null);
const pairs = (value: unknown): [string, string][] | null => {
    if (!Array.isArray(value)) return null;
    const result: [string, string][] = [];
    for (const item of value.slice(0, MAX_PAIRS)) {
        if (!Array.isArray(item)) return null;
        const [key, entryValue] = [text(item[0]), text(item[1])];
        if (key === null || entryValue === null) return null;
        result.push([key, entryValue]);
    }
    return result;
};

const parseInner = (entry: Record_): DevtoolsEntry | null => {
    const time = number(entry.time);
    if (time === null) return null;
    switch (entry.kind) {
        case 'console': {
            const level = LEVELS.find(candidate => candidate === entry.level);
            const message = text(entry.message);
            return level && message !== null ? {kind: 'console', level, message, time} : null;
        }
        case 'network': {
            const method = text(entry.method);
            const url = text(entry.url);
            const durationMs = number(entry.durationMs);
            if (method === null || url === null || durationMs === null) return null;
            return {kind: 'network', method, url, status: number(entry.status), bytes: number(entry.bytes), durationMs, error: text(entry.error), time};
        }
        case 'storage': {
            const cookies = pairs(entry.cookies);
            const local = pairs(entry.local);
            return cookies && local ? {kind: 'storage', cookies, local, time} : null;
        }
        case 'image': {
            const fields = ['cssWidth', 'cssHeight', 'pixelWidth', 'pixelHeight', 'zoom', 'frameMs'] as const;
            const values = fields.map(field => number(entry[field]));
            const [id, title, transform] = [text(entry.id), text(entry.title), text(entry.transform)];
            if (values.some(value => value === null) || id === null || title === null || transform === null) return null;
            const [cssWidth, cssHeight, pixelWidth, pixelHeight, zoom, frameMs] = values as number[];
            const pointerValues = Array.isArray(entry.pointer) ? entry.pointer.map(number) : null;
            const pointer: [number, number] | null =
                pointerValues?.length === 2 && pointerValues[0] !== null && pointerValues[1] !== null ? [pointerValues[0], pointerValues[1]] : null;
            return {kind: 'image', id, title, cssWidth, cssHeight, pixelWidth, pixelHeight, zoom, frameMs, pointer, transform, time};
        }
        case 'lifecycle':
            return entry.state === 'ready' ? {kind: 'lifecycle', state: 'ready', time} : null;
        default:
            return null;
    }
};

export const parseReport = (payload: unknown): TabReport | null => {
    if (!isRecord(payload) || !isRecord(payload.entry)) return null;
    const tabId = text(payload.tabId);
    const entry = parseInner(payload.entry);
    return tabId && entry ? {tabId, entry} : null;
};
