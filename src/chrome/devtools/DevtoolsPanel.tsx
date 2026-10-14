import {useEffect, useState} from 'react';
import {listen} from '@tauri-apps/api/event';
import {parseReport, type ConsoleEntry, type ImageEntry, type NetworkEntry, type StorageEntry} from '../../shared/devtools';
import {Icon} from '../../shared/Icon';
import {DEVTOOLS_EVENT} from '../commands';
import {ConsolePane} from './ConsolePane';
import {EguiPane} from './EguiPane';
import {NetworkPane} from './NetworkPane';
import {StoragePane} from './StoragePane';
import './DevtoolsPanel.css';

export interface TabLog {
    console: ConsoleEntry[];
    network: NetworkEntry[];
    storage: StorageEntry | null;
    images: Record<string, ImageEntry>;
}

const EMPTY_LOG: TabLog = {console: [], network: [], storage: null, images: {}};
// Per tab and per list; older rows fall off the top.
const MAX_ROWS = 500;

type Pane = 'console' | 'network' | 'storage' | 'egui';
const PANES: {id: Pane; label: string}[] = [
    {id: 'console', label: 'Console'},
    {id: 'network', label: 'Network'},
    {id: 'storage', label: 'Storage'},
    {id: 'egui', label: 'egui'},
];

const append = <T,>(rows: T[], row: T) => [...rows.slice(-(MAX_ROWS - 1)), row];

interface DevtoolsPanelProps {
    open: boolean;
    activeTabId: string | null;
    storeId: string | null;
    onClose: () => void;
}

// Always mounted, so it keeps collecting while hidden.
export const DevtoolsPanel = ({open, activeTabId, storeId, onClose}: DevtoolsPanelProps) => {
    const [logs, setLogs] = useState<Record<string, TabLog>>({});
    const [pane, setPane] = useState<Pane>('console');

    useEffect(() => {
        const unlisten = listen(DEVTOOLS_EVENT, event => {
            const report = parseReport(event.payload);
            if (!report || report.entry.kind === 'lifecycle') return;
            const {tabId, entry} = report;
            setLogs(current => {
                const log = current[tabId] ?? EMPTY_LOG;
                switch (entry.kind) {
                    case 'console':
                        return {...current, [tabId]: {...log, console: append(log.console, entry)}};
                    case 'network':
                        return {...current, [tabId]: {...log, network: append(log.network, entry)}};
                    case 'storage':
                        return {...current, [tabId]: {...log, storage: entry}};
                    case 'image':
                        return {...current, [tabId]: {...log, images: {...log.images, [entry.id]: entry}}};
                }
            });
        });
        return () => void unlisten.then(stop => stop());
    }, []);

    const log = (activeTabId && logs[activeTabId]) || EMPTY_LOG;
    const counts: Record<Pane, number> = {
        console: log.console.length,
        network: log.network.length,
        storage: log.storage ? log.storage.cookies.length + log.storage.local.length : 0,
        egui: Object.keys(log.images).length,
    };
    const errorCount = log.console.filter(entry => entry.level === 'error').length;

    const clear = () => {
        if (!activeTabId) return;
        setLogs(current => ({...current, [activeTabId]: {...(current[activeTabId] ?? EMPTY_LOG), console: [], network: []}}));
    };

    return (
        <section className="devtools" hidden={!open} aria-label="Developer tools">
            <header className="devtools__header">
                <div className="devtools__tabs" role="tablist">
                    {PANES.map(({id, label}) => (
                        <button
                            key={id}
                            type="button"
                            role="tab"
                            aria-selected={pane === id}
                            className={pane === id ? 'devtools__tab devtools__tab--active' : 'devtools__tab'}
                            onClick={() => setPane(id)}
                        >
                            {label}
                            {counts[id] > 0 && <span className="devtools__count">{counts[id]}</span>}
                            {id === 'console' && errorCount > 0 && <span className="devtools__count devtools__count--error">{errorCount}</span>}
                        </button>
                    ))}
                </div>
                <div className="devtools__actions">
                    <button className="devtools__icon-button" type="button" onClick={clear} aria-label="Clear console and network" title="Clear">
                        <Icon name="trash"/>
                    </button>
                    <button className="devtools__icon-button" type="button" onClick={onClose} aria-label="Close developer tools" title="Close (⌥⌘I)">
                        <Icon name="close"/>
                    </button>
                </div>
            </header>
            <div className="devtools__body">
                {!activeTabId && <p className="devtools__empty">Open a tab to inspect it.</p>}
                {activeTabId && pane === 'console' && <ConsolePane entries={log.console}/>}
                {activeTabId && pane === 'network' && <NetworkPane entries={log.network}/>}
                {activeTabId && pane === 'storage' && <StoragePane snapshot={log.storage} storeId={storeId}/>}
                {activeTabId && pane === 'egui' && <EguiPane images={Object.values(log.images)}/>}
            </div>
        </section>
    );
};
