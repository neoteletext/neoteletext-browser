import type {CSSProperties} from 'react';
import {mosaicHues} from './t42';
import './Mosaic.css';

interface MosaicProps {
    src: string;
    size: 'small' | 'large';
}

export const Mosaic = ({src, size}: MosaicProps) => (
    <span className={`mosaic mosaic--${size}`} aria-hidden="true">
        {mosaicHues(src).map((hue, cell) => (
            <span key={cell} className="mosaic__cell" style={{'--hue': hue} as CSSProperties}/>
        ))}
    </span>
);
