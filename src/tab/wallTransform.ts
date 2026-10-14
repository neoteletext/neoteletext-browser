// CSS transforms applied to the wall of egui images. They change only how
// the canvases are composited; egui keeps receiving pointer input in each
// canvas's own coordinates.
export const WALL_TRANSFORMS = ['flat', 'tilt', 'fan'] as const;
export type WallTransform = (typeof WALL_TRANSFORMS)[number];

export const TRANSFORM_EVENT = 'tab:wall-transform';

declare global {
    interface DocumentEventMap {
        [TRANSFORM_EVENT]: CustomEvent<WallTransform>;
    }
}

export const dispatchTransform = (transform: WallTransform) => {
    document.dispatchEvent(new CustomEvent<WallTransform>(TRANSFORM_EVENT, {detail: transform}));
};
