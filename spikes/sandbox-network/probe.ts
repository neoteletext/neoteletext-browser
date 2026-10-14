// Proof A probe. The security policy is off on purpose, so only the
// operating-system sandbox stands between the page and the network. Every
// probe targets the local test server; its log is the ground truth.

const SERVER_PORT = 47420;
const STUN_PORT = 47421;
// The server's ordinary network address: loopback addresses bypass proxies.
const SERVER_HOST = '192.168.0.169';
const SERVER = `http://${SERVER_HOST}:${SERVER_PORT}`;
const SETTLE_MS = 4000;

declare global {
    interface Window {
        __TAURI__: { core: { invoke: (command: string, args: Record<string, unknown>) => Promise<unknown> } };
    }
}

const results: string[] = [];
const note = (name: string, outcome: string) => results.push(`${name}: ${outcome}`);
const failed = (error: unknown) => `failed (${String(error).slice(0, 80)})`;

const attach = (element: HTMLElement) => document.body.appendChild(element);

const probes: [string, () => Promise<string>][] = [
    ['own asset', async () => `status ${(await fetch('own-asset.txt')).status}`],
    ['fetch', async () => `status ${(await fetch(`${SERVER}/fetch`)).status}`],
    ['XMLHttpRequest', () => new Promise(resolve => {
        const request = new XMLHttpRequest();
        request.onload = () => resolve(`status ${request.status}`);
        request.onerror = () => resolve('failed (error event)');
        request.open('GET', `${SERVER}/xhr`);
        request.send();
    })],
    ['WebSocket', () => new Promise(resolve => {
        const socket = new WebSocket(`ws://${SERVER_HOST}:${SERVER_PORT}/ws`);
        socket.onopen = () => resolve('opened');
        socket.onerror = () => resolve('failed (error event)');
    })],
    ['Server-Sent Events', () => new Promise(resolve => {
        const source = new EventSource(`${SERVER}/sse`);
        source.onopen = () => resolve('opened');
        source.onerror = () => { source.close(); resolve('failed (error event)'); };
    })],
    ['beacon', async () => (navigator.sendBeacon(`${SERVER}/beacon`, 'x') ? 'queued' : 'refused')],
    ['image', () => new Promise(resolve => {
        const image = new Image();
        image.onload = () => resolve('loaded');
        image.onerror = () => resolve('failed (error event)');
        image.src = `${SERVER}/img`;
    })],
    ['font', async () => { await new FontFace('probe', `url(${SERVER}/font)`).load(); return 'loaded'; }],
    ['stylesheet', () => new Promise(resolve => {
        const link = document.createElement('link');
        link.rel = 'stylesheet';
        link.href = `${SERVER}/css`;
        link.onload = () => resolve('loaded');
        link.onerror = () => resolve('failed (error event)');
        attach(link);
    })],
    ['prefetch', async () => {
        const link = document.createElement('link');
        link.rel = 'prefetch';
        link.href = `${SERVER}/prefetch`;
        attach(link);
        return 'requested';
    }],
    ['iframe', async () => {
        const frame = document.createElement('iframe');
        frame.src = `${SERVER}/iframe`;
        attach(frame);
        return 'requested';
    }],
    ['WebRTC', async () => {
        if (!('RTCPeerConnection' in window)) return 'not available';
        const connection = new RTCPeerConnection({iceServers: [{urls: `stun:${SERVER_HOST}:${STUN_PORT}`}]});
        connection.createDataChannel('probe');
        await connection.setLocalDescription(await connection.createOffer());
        return 'gathering started';
    }],
    ['WebRTC through a blank iframe', async () => {
        const frame = document.createElement('iframe');
        attach(frame);
        const inner = frame.contentWindow as (Window & {RTCPeerConnection?: typeof RTCPeerConnection}) | null;
        const Peer = inner?.RTCPeerConnection;
        if (!Peer) return 'not available';
        const connection = new Peer({iceServers: [{urls: `stun:${SERVER_HOST}:${STUN_PORT}`}]});
        connection.createDataChannel('probe');
        await connection.setLocalDescription(await connection.createOffer());
        return 'gathering started';
    }],
];

const run = async () => {
    for (const [name, probe] of probes) {
        try {
            note(name, await probe());
        } catch (error) {
            note(name, failed(error));
        }
    }
    if (results[0]?.includes('status 200')) {
        const asset = document.getElementById('asset');
        if (asset) asset.hidden = false;
    }
    await new Promise(resolve => setTimeout(resolve, SETTLE_MS));
    await window.__TAURI__.core.invoke('report', {lines: results});
};

void run();
export {};
