import {Mosaic} from '../../shared/Mosaic';
import {fileTitle} from '../../shared/t42';
import {FRAMES} from '../frames';
import {SizeReadout} from './SizeReadout';
import {ThumbnailSizeSlider} from './ThumbnailSizeSlider';
import {TransformPicker} from './TransformPicker';
import './PageHeader.css';

interface PageHeaderProps {
    src: string;
}

export const PageHeader = ({src}: PageHeaderProps) => (
    <header className="page-header">
        <div className="page-header__inner">
            <div className="page-header__identity">
                <Mosaic src={src} size="large"/>
                <div className="page-header__text">
                    <h1 className="page-header__title">{fileTitle(src)}</h1>
                    <p className="page-header__meta">
                        <span>{FRAMES.length} egui images</span>
                        <span className="page-header__sample">Sample data</span>
                    </p>
                </div>
            </div>
            <div className="page-header__controls">
                <TransformPicker/>
                <ThumbnailSizeSlider/>
                <SizeReadout/>
            </div>
        </div>
    </header>
);
