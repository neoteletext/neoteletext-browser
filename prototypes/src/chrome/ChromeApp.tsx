import {useCallback, useEffect, useLayoutEffect, useRef, useState} from 'react';
import {listen} from '@tauri-apps/api/event';
import {parseReport} from '../shared/devtools';
import {SAMPLE_SRC} from '../shared/t42';
import {
    DEVTOOLS_EVENT,
    MENU_EVENT,
    closeTab as closeTabCommand,
    createTab,
    errorMessage,
    navigateTab,
    requestUrlFocus,
    selectTab as selectTabCommand,
    type MenuCommand,
} from './commands';
import {ContentHost} from './ContentHost';
import {DevtoolsPanel} from './devtools/DevtoolsPanel';
import {currentSrc, type Tab} from './tabs';
import {TabStrip} from './TabStrip';
import {Toolbar} from './Toolbar';
import {UrlBar} from './UrlBar';
import './ChromeApp.css';

export const ChromeApp = () => {
    const [tabs, setTabs] = useState<Tab[]>([]);
    const [activeId, setActiveId] = useState<string | null>(null);
    const [devtoolsOpen, setDevtoolsOpen] = useState(false);

    // Menu and IPC listeners are registered once; they read the latest
    // state through these refs instead of re-subscribing every render.
    const tabsRef = useRef(tabs);
    const activeRef = useRef(activeId);
    useLayoutEffect(() => {
        tabsRef.current = tabs;
        activeRef.current = activeId;
    });

    const activeTab = tabs.find(tab => tab.id === activeId) ?? null;

    const openTab = useCallback(async (src: string) => {
        try {
            const created = await createTab(src);
            setTabs(current => [...current, {id: created.id, storeId: created.storeId, history: [src.trim()], index: 0, ready: false}]);
            setActiveId(created.id);
            return null;
        } catch (error) {
            return errorMessage(error);
        }
    }, []);

    const selectTab = useCallback(async (id: string) => {
        try {
            await selectTabCommand(id);
            setActiveId(id);
        } catch (error) {
            console.error('Could not switch tab:', errorMessage(error));
        }
    }, []);

    const closeTab = useCallback(
        async (id: string) => {
            const current = tabsRef.current;
            const index = current.findIndex(tab => tab.id === id);
            if (index < 0) return;
            try {
                await closeTabCommand(id);
            } catch (error) {
                console.error('Could not close tab:', errorMessage(error));
                return;
            }
            const remaining = current.filter(tab => tab.id !== id);
            setTabs(remaining);
            if (activeRef.current !== id) return;
            const next = remaining[Math.min(index, remaining.length - 1)];
            if (next) await selectTab(next.id);
            else setActiveId(null);
        },
        [selectTab],
    );

    // Moves the active tab to another entry in its own history.
    const goTo = useCallback(async (index: number) => {
        const tab = tabsRef.current.find(candidate => candidate.id === activeRef.current);
        const src = tab?.history[index];
        if (!tab || src === undefined) return;
        try {
            await navigateTab(tab.id, src);
            setTabs(current => current.map(candidate => (candidate.id === tab.id ? {...candidate, index, ready: false} : candidate)));
        } catch (error) {
            console.error('Could not navigate:', errorMessage(error));
        }
    }, []);

    const submitAddress = useCallback(
        async (src: string) => {
            const tab = tabsRef.current.find(candidate => candidate.id === activeRef.current);
            if (!tab) return openTab(src);
            try {
                await navigateTab(tab.id, src);
            } catch (error) {
                return errorMessage(error);
            }
            const trimmed = src.trim();
            setTabs(current =>
                current.map(candidate =>
                    candidate.id === tab.id
                        ? {...candidate, history: [...candidate.history.slice(0, candidate.index + 1), trimmed], index: candidate.index + 1, ready: false}
                        : candidate,
                ),
            );
            return null;
        },
        [openTab],
    );

    const step = useCallback(
        (direction: number) => {
            const tab = tabsRef.current.find(candidate => candidate.id === activeRef.current);
            if (tab) void goTo(tab.index + direction);
        },
        [goTo],
    );

    const reload = useCallback(() => {
        const tab = tabsRef.current.find(candidate => candidate.id === activeRef.current);
        if (tab) void goTo(tab.index);
    }, [goTo]);

    const cycleTabs = useCallback(
        (direction: number) => {
            const current = tabsRef.current;
            if (current.length < 2) return;
            const index = current.findIndex(tab => tab.id === activeRef.current);
            const next = current[(index + direction + current.length) % current.length];
            void selectTab(next.id);
        },
        [selectTab],
    );

    const newTab = useCallback(async () => {
        const error = await openTab(SAMPLE_SRC);
        if (error) console.error('Could not open a tab:', error);
        else requestUrlFocus();
    }, [openTab]);

    useEffect(() => {
        const handlers: Record<MenuCommand, () => void> = {
            'new-tab': () => void newTab(),
            'open-location': requestUrlFocus,
            'close-tab': () => {
                if (activeRef.current) void closeTab(activeRef.current);
            },
            reload,
            back: () => step(-1),
            forward: () => step(1),
            'toggle-devtools': () => setDevtoolsOpen(open => !open),
            'next-tab': () => cycleTabs(1),
            'previous-tab': () => cycleTabs(-1),
        };
        const unlisten = listen<string>(MENU_EVENT, event => {
            const handler = handlers[event.payload as MenuCommand] as (() => void) | undefined;
            handler?.();
        });
        return () => void unlisten.then(stop => stop());
    }, [closeTab, cycleTabs, newTab, reload, step]);

    // A tab says it is ready once its page and egui module have loaded.
    useEffect(() => {
        const unlisten = listen(DEVTOOLS_EVENT, event => {
            const report = parseReport(event.payload);
            if (report?.entry.kind !== 'lifecycle') return;
            setTabs(current => current.map(tab => (tab.id === report.tabId ? {...tab, ready: true} : tab)));
        });
        return () => void unlisten.then(stop => stop());
    }, []);

    // React's development double-mount keeps refs, so this opens one tab.
    const openedInitial = useRef(false);
    useEffect(() => {
        if (openedInitial.current) return;
        openedInitial.current = true;
        void openTab(SAMPLE_SRC);
    }, [openTab]);

    return (
        <div className="browser">
            <TabStrip tabs={tabs} activeId={activeId} onSelect={id => void selectTab(id)} onClose={id => void closeTab(id)} onNew={() => void newTab()}/>
            <Toolbar
                canGoBack={!!activeTab && activeTab.index > 0}
                canGoForward={!!activeTab && activeTab.index < activeTab.history.length - 1}
                canReload={!!activeTab}
                devtoolsOpen={devtoolsOpen}
                onBack={() => step(-1)}
                onForward={() => step(1)}
                onReload={reload}
                onToggleDevtools={() => setDevtoolsOpen(open => !open)}
            >
                <UrlBar
                    key={activeTab ? `${activeTab.id}:${activeTab.index}` : 'none'}
                    src={activeTab ? currentSrc(activeTab) : ''}
                    storeId={activeTab?.storeId ?? null}
                    onSubmit={submitAddress}
                />
            </Toolbar>
            <ContentHost empty={tabs.length === 0} onOpenSample={() => void openTab(SAMPLE_SRC)}/>
            <DevtoolsPanel open={devtoolsOpen} activeTabId={activeId} storeId={activeTab?.storeId ?? null} onClose={() => setDevtoolsOpen(false)}/>
        </div>
    );
};
