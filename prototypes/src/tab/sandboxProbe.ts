import {invoke} from '@tauri-apps/api/core';
import {SAMPLE_SRC} from '../shared/t42';

const VISITS_KEY = 't42.visits';
const VISITS_COOKIE = 'neoteletext_visits';
const ONE_YEAR_SECONDS = 60 * 60 * 24 * 365;

// Counts loads in this tab's data store. A new tab gets a new store, so it
// starts again at 1; reloading the same tab keeps counting.
const countVisit = () => {
    try {
        const visits = Number(localStorage.getItem(VISITS_KEY) ?? '0') + 1;
        localStorage.setItem(VISITS_KEY, String(visits));
        document.cookie = `${VISITS_COOKIE}=${visits}; path=/; max-age=${ONE_YEAR_SECONDS}; SameSite=Strict`;
        return visits;
    } catch {
        return null;
    }
};

// Proves the capability split at runtime: a tab asking to open a tab must
// be refused by Rust's ACL before the command runs.
const checkCommandIsolation = async () => {
    try {
        await invoke('create_tab', {src: SAMPLE_SRC});
        console.error('Sandbox check failed: this tab was allowed to call create_tab');
    } catch (error) {
        console.info(`Sandbox check passed: create_tab was refused (${String(error)})`);
    }
};

export const runSandboxProbe = async () => {
    const visits = countVisit();
    console.info(visits === null ? 'Storage is unavailable in this tab' : `Load ${visits} of this tab's data store`);
    await checkCommandIsolation();
};
