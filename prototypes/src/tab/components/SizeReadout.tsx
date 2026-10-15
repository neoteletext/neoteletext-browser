import {useLayoutEffect, useState} from 'react';
import {THUMBNAIL_SIZE_EVENT, gridWidth, heightForWidth, type ThumbnailSizeDetail} from '../thumbnailSize';
import './SizeReadout.css';

export const SizeReadout = () => {
    const [size, setSize] = useState<ThumbnailSizeDetail | null>(null);

    // Layout effects run before the slider's passive effect, so the
    // subscription is in place for its first announcement.
    useLayoutEffect(() => {
        const handler = (event: CustomEvent<ThumbnailSizeDetail>) => setSize(event.detail);
        document.addEventListener(THUMBNAIL_SIZE_EVENT, handler);
        return () => document.removeEventListener(THUMBNAIL_SIZE_EVENT, handler);
    }, []);

    if (size === null) return null;

    if (size.layout === 'list') {
        return (
            <output className="size-readout" htmlFor="thumbnail-size" aria-live="polite">
                <span className="size-readout__value">List</span>
            </output>
        );
    }

    const width = gridWidth(size.width);

    return (
        <output className="size-readout" htmlFor="thumbnail-size" aria-live="polite">
            <span className="size-readout__value size-readout__value--width">{width}</span>
            <span className="size-readout__times">×</span>
            <span className="size-readout__value">{heightForWidth(width)}</span>
            <span className="size-readout__unit">px</span>
        </output>
    );
};
