import {useEffect, useState, type ChangeEvent, type CSSProperties} from 'react';
import {
    DEFAULT_WIDTH,
    LIST_STOP,
    MAX_WIDTH,
    clampWidth,
    dispatchThumbnailSize,
    gridWidth,
    layoutForWidth,
    loadStoredWidth,
    maxWidthForViewport,
    nextLayout,
    storeWidth,
} from '../thumbnailSize';
import './ThumbnailSizeSlider.css';

// The thumb must stay past a threshold this long before the layout
// switches, so a fast sweep across the band does not trigger it.
const SWITCH_DWELL_MS = 120;

// Clamped to the slider's full range only: a tab's webview can start at
// zero size before the chrome places it, and clamping to that viewport
// would throw away the stored width. effectiveWidth applies the viewport.
const initialWidth = () => clampWidth(loadStoredWidth() ?? DEFAULT_WIDTH, MAX_WIDTH);

export const ThumbnailSizeSlider = () => {
    const [maxWidth, setMaxWidth] = useState(maxWidthForViewport);
    const [width, setWidth] = useState(initialWidth);
    const [layout, setLayout] = useState(() => layoutForWidth(width));

    useEffect(() => {
        const handleResize = () => setMaxWidth(maxWidthForViewport());
        window.addEventListener('resize', handleResize);
        return () => window.removeEventListener('resize', handleResize);
    }, []);

    // A narrower window lowers the ceiling; pull the width down with it so
    // the thumbnails never outgrow the screen.
    const effectiveWidth = clampWidth(width, maxWidth);
    const targetLayout = nextLayout(layout, effectiveWidth);

    // Keyed on the target, not the width: the timer survives continued
    // dragging on the same side and is cancelled if the thumb crosses back.
    useEffect(() => {
        if (targetLayout === layout) return;
        const timer = window.setTimeout(() => setLayout(targetLayout), SWITCH_DWELL_MS);
        return () => window.clearTimeout(timer);
    }, [targetLayout, layout]);

    // Announce on mount too, so listeners start at the restored width.
    useEffect(() => {
        dispatchThumbnailSize(effectiveWidth, layout);
    }, [effectiveWidth, layout]);

    const handleChange = (event: ChangeEvent<HTMLInputElement>) => {
        const next = clampWidth(Number.parseInt(event.target.value, 10), maxWidth);
        setWidth(next);
        storeWidth(next);
    };

    const fill = (effectiveWidth - LIST_STOP) / (maxWidth - LIST_STOP);
    const isList = layout === 'list';

    return (
        <div className="size-slider">
            <label className="size-slider__label" htmlFor="thumbnail-size">Thumbnail width</label>
            <input
                id="thumbnail-size"
                className="size-slider__input"
                type="range"
                min={LIST_STOP}
                max={maxWidth}
                value={effectiveWidth}
                aria-valuetext={isList ? 'List view' : `${gridWidth(effectiveWidth)} pixels wide`}
                onChange={handleChange}
                style={{'--fill': fill} as CSSProperties}
            />
            <div className="size-slider__scale" aria-hidden="true">
                <span className={isList ? 'size-slider__stop size-slider__stop--active' : 'size-slider__stop'}>List</span>
                <span>{maxWidth}</span>
            </div>
        </div>
    );
};
