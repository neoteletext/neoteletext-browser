// The slider and every view that sizes thumbnails agree on width through
// this one event, so neither imports the other and no state is shared.
export const THUMBNAIL_SIZE_EVENT = 'IMAGE_SIZE_SLIDER_UPDATED';

export type ThumbnailLayout = 'grid' | 'list';

export interface ThumbnailSizeDetail {
    width: number;
    layout: ThumbnailLayout;
}

declare global {
    interface DocumentEventMap {
        [THUMBNAIL_SIZE_EVENT]: CustomEvent<ThumbnailSizeDetail>;
    }
}

// Smallest width at which a 16:9 frame and its caption stay legible.
export const MIN_WIDTH = 192;
// Slider travel below MIN_WIDTH belongs to the list layout, which shows
// the frames as rows with their details beside.
const LIST_ZONE = 48;
export const LIST_STOP = MIN_WIDTH - LIST_ZONE;

// Hysteresis: the layout switches at a different point in each direction,
// so a hand wobbling near the boundary cannot flip it back and forth.
// Entering needs the thumb halfway into the list zone; leaving needs it
// clear of the grid minimum by a margin.
const LEAVE_MARGIN = 8;
export const LIST_ENTER_BELOW = LIST_STOP + LIST_ZONE / 2;
export const LIST_LEAVE_ABOVE = MIN_WIDTH + LEAVE_MARGIN;

// A restored width has no direction of travel; the zone boundary decides.
export const layoutForWidth = (width: number): ThumbnailLayout => (width < MIN_WIDTH ? 'list' : 'grid');

export const nextLayout = (current: ThumbnailLayout, width: number): ThumbnailLayout => {
    if (current === 'grid') return width < LIST_ENTER_BELOW ? 'list' : 'grid';
    return width > LIST_LEAVE_ABOVE ? 'grid' : 'list';
};

// width is where the thumb sits; layout is the committed layout, which
// trails the thumb through the hysteresis band and the dwell delay.
export const dispatchThumbnailSize = (width: number, layout: ThumbnailLayout) => {
    const detail: ThumbnailSizeDetail = {width, layout};
    document.dispatchEvent(new CustomEvent<ThumbnailSizeDetail>(THUMBNAIL_SIZE_EVENT, {detail}));
};

// 1080p: the source frames are never upscaled past their native width.
export const MAX_WIDTH = 1920;
// A single thumbnail may not exceed this share of the viewport, so the
// largest setting still leaves a margin around one frame.
const MAX_VIEWPORT_SHARE = 0.9;
export const DEFAULT_WIDTH = 320;

export const ASPECT_WIDTH = 16;
export const ASPECT_HEIGHT = 9;

export const heightForWidth = (width: number) => Math.round(width * ASPECT_HEIGHT / ASPECT_WIDTH);

export const maxWidthForViewport = () =>
    Math.max(MIN_WIDTH, Math.min(MAX_WIDTH, Math.floor(window.innerWidth * MAX_VIEWPORT_SHARE)));

export const clampWidth = (width: number, max: number) => Math.min(Math.max(width, LIST_STOP), max);

// Grid frames never go below the minimum, even while the thumb sits in
// the hysteresis band on its way into or out of the list.
export const gridWidth = (width: number) => Math.max(width, MIN_WIDTH);

const STORAGE_KEY = 't42.thumbnailWidth';

// Storage can be missing or throw (private windows, blocked site data);
// the slider then simply starts at the default each visit.
export const loadStoredWidth = (): number | null => {
    try {
        const stored = Number.parseInt(localStorage.getItem(STORAGE_KEY) ?? '', 10);
        return Number.isFinite(stored) ? stored : null;
    } catch {
        return null;
    }
};

export const storeWidth = (width: number) => {
    try {
        localStorage.setItem(STORAGE_KEY, String(width));
    } catch {
        // Persistence is a convenience; the current session keeps working.
    }
};
