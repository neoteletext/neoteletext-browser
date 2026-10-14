import init, {EguiImage} from './egui-pkg/egui_image.js';

// The wasm module is compiled once per tab and shared by every image in it;
// each EguiImage is an independent egui app on its own canvas.
let ready: Promise<unknown> | null = null;

export const loadEgui = () => (ready ??= init());

export {EguiImage};
