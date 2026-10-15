import {useLayoutEffect, useState, type CSSProperties} from 'react';
import {FRAMES} from '../frames';
import {DEFAULT_WIDTH, THUMBNAIL_SIZE_EVENT, gridWidth, layoutForWidth, type ThumbnailSizeDetail} from '../thumbnailSize';
import {TRANSFORM_EVENT, type WallTransform} from '../wallTransform';
import {EguiCard} from './EguiCard';
import './Wall.css';

export const Wall = () => {
    const [size, setSize] = useState<ThumbnailSizeDetail>({width: DEFAULT_WIDTH, layout: layoutForWidth(DEFAULT_WIDTH)});
    const [transform, setTransform] = useState<WallTransform>('flat');

    // Layout effects run before the slider's passive effect, so the
    // subscription is in place for its first announcement.
    useLayoutEffect(() => {
        const onSize = (event: CustomEvent<ThumbnailSizeDetail>) => setSize(event.detail);
        const onTransform = (event: CustomEvent<WallTransform>) => setTransform(event.detail);
        document.addEventListener(THUMBNAIL_SIZE_EVENT, onSize);
        document.addEventListener(TRANSFORM_EVENT, onTransform);
        return () => {
            document.removeEventListener(THUMBNAIL_SIZE_EVENT, onSize);
            document.removeEventListener(TRANSFORM_EVENT, onTransform);
        };
    }, []);

    const width = size.layout === 'grid' ? gridWidth(size.width) : size.width;

    return (
        <div className={`wall-stage wall-stage--${transform}`}>
            <section
                className={`wall wall--${size.layout} wall--${transform}`}
                aria-label="egui images"
                style={{'--card-width': `${width}px`} as CSSProperties}
            >
                {FRAMES.map(frame => (
                    <EguiCard key={frame.id} frame={frame}/>
                ))}
            </section>
        </div>
    );
};
