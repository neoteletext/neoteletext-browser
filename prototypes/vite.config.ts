import {resolve} from 'node:path';
import {defineConfig} from 'vite';
import react from '@vitejs/plugin-react';

// tauri.conf.json devUrl points at this port. It differs from Vite's
// default so this app and the slider demo can run side by side; strictPort
// makes a clash an error instead of a silent move the window never loads.
const DEV_PORT = 5174;

export default defineConfig({
    plugins: [react()],
    clearScreen: false,
    server: {
        port: DEV_PORT,
        strictPort: true,
        watch: {ignored: ['**/src-tauri/**', '**/egui-image/**', '**/tools/**']},
    },
    build: {
        // Two pages: the browser chrome, and the page every tab loads.
        rollupOptions: {
            input: {
                chrome: resolve(import.meta.dirname, 'index.html'),
                tab: resolve(import.meta.dirname, 'tab.html'),
            },
        },
    },
});
