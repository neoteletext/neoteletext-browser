import {invoke} from '@tauri-apps/api/core';

// Typed wrappers for the chrome-only commands in src-tauri/src/browser.rs.

export interface TabCreated {
    id: string;
    storeId: string;
}

export interface Bounds {
    x: number;
    y: number;
    width: number;
    height: number;
}

export const createTab = (src: string) => invoke<TabCreated>('create_tab', {src});
export const closeTab = (id: string) => invoke<null>('close_tab', {id});
export const selectTab = (id: string) => invoke<null>('select_tab', {id});
export const navigateTab = (id: string, src: string) => invoke<null>('navigate_tab', {id, src});
export const setContentBounds = (bounds: Bounds) => invoke<null>('set_content_bounds', {bounds});

// Menu item ids from src-tauri/src/menu.rs.
export type MenuCommand =
    | 'new-tab'
    | 'open-location'
    | 'close-tab'
    | 'reload'
    | 'back'
    | 'forward'
    | 'toggle-devtools'
    | 'next-tab'
    | 'previous-tab';

export const MENU_EVENT = 'menu-command';
export const DEVTOOLS_EVENT = 'devtools-entry';

// Asks the URL bar to take focus and select its text.
export const FOCUS_URL_EVENT = 'chrome:focus-url';
export const requestUrlFocus = () => document.dispatchEvent(new CustomEvent(FOCUS_URL_EVENT));

// Commands return Rust's error string; show it as written.
export const errorMessage = (error: unknown) => (typeof error === 'string' ? error : String(error));
