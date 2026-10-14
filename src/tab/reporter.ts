import {invoke} from '@tauri-apps/api/core';
import {MAX_TEXT, type DevtoolsEntry} from '../shared/devtools';

// Feeds the chrome's devtools panel. `devtools_report` is the only native
// command a tab may call; Rust stamps each report with this tab's id.

type Draft = DevtoolsEntry extends infer Entry ? (Entry extends DevtoolsEntry ? Omit<Entry, 'time'> : never) : never;

// Reporting must never report its own failure, or a broken channel would
// loop; failures go to the untouched console instead.
const originalConsole = {log: console.log, info: console.info, warn: console.warn, error: console.error};

export const report = (entry: Draft) => {
    invoke('devtools_report', {entry: {...entry, time: Date.now()}}).catch((error: unknown) =>
        originalConsole.warn.call(console, 'devtools report dropped:', error),
    );
};

const describe = (value: unknown): string => {
    if (typeof value === 'string') return value;
    if (value instanceof Error) return `${value.name}: ${value.message}`;
    try {
        return JSON.stringify(value) ?? String(value);
    } catch {
        return String(value);
    }
};

const hookConsole = () => {
    for (const level of ['log', 'info', 'warn', 'error'] as const) {
        const original = originalConsole[level];
        console[level] = (...args: unknown[]) => {
            original.apply(console, args);
            report({kind: 'console', level, message: args.map(describe).join(' ').slice(0, MAX_TEXT)});
        };
    }
    window.addEventListener('error', event => {
        report({kind: 'console', level: 'error', message: `${event.message} (${event.filename}:${event.lineno})`});
    });
    window.addEventListener('unhandledrejection', event => {
        report({kind: 'console', level: 'error', message: `Unhandled rejection: ${describe(event.reason)}`});
    });
};

// Tauri's own IPC travels as requests too; showing them would make every
// report generate another row.
const isIpc = (url: string) => url.startsWith('ipc:') || url.includes('ipc.localhost');

// Resource Timing sees every load (scripts, styles, the egui wasm, fetches)
// without wrapping fetch. It does not expose the method, and only some
// engines expose the status, so those show as GET and "—".
const observeNetwork = () => {
    const observer = new PerformanceObserver(list => {
        for (const entry of list.getEntries() as PerformanceResourceTiming[]) {
            if (isIpc(entry.name)) continue;
            const status = 'responseStatus' in entry && typeof entry.responseStatus === 'number' && entry.responseStatus > 0 ? entry.responseStatus : null;
            const bytes = entry.encodedBodySize || entry.transferSize || null;
            report({kind: 'network', method: 'GET', url: entry.name, status, bytes, durationMs: entry.duration, error: null});
        }
    });
    observer.observe({type: 'resource', buffered: true});
};

const STORAGE_POLL_MS = 1500;

const readCookies = (): [string, string][] =>
    document.cookie
        .split('; ')
        .filter(Boolean)
        .map(cookie => {
            const split = cookie.indexOf('=');
            return split < 0 ? [cookie, ''] : [cookie.slice(0, split), decodeURIComponent(cookie.slice(split + 1))];
        });

const readLocal = (): [string, string][] => {
    try {
        return Object.keys(localStorage)
            .sort()
            .map(key => [key, localStorage.getItem(key) ?? '']);
    } catch {
        return [];
    }
};

// Polls because neither cookies nor same-document storage writes fire events.
const watchStorage = () => {
    let last = '';
    const check = () => {
        const snapshot = {cookies: readCookies(), local: readLocal()};
        const serialized = JSON.stringify(snapshot);
        if (serialized === last) return;
        last = serialized;
        report({kind: 'storage', ...snapshot});
    };
    check();
    window.setInterval(check, STORAGE_POLL_MS);
};

export const installReporter = () => {
    hookConsole();
    observeNetwork();
    watchStorage();
};

export const reportReady = () => report({kind: 'lifecycle', state: 'ready'});
