export interface Tab {
    id: string;
    storeId: string;
    history: string[];
    index: number;
    ready: boolean;
}

export const currentSrc = (tab: Tab) => tab.history[tab.index];
