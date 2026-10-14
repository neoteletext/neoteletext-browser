import type {ImageEntry} from '../../shared/devtools';
import {formatMs} from './format';

interface EguiPaneProps {
    images: ImageEntry[];
}

// At 60 Hz a frame has 16.7 ms; past this the egui pass alone eats a
// third of it, which is worth seeing at a glance.
const SLOW_FRAME_MS = 5;

const describeTransform = (transform: string) => (transform === 'none' ? 'none' : transform.replace(/^matrix(3d)?\(/, 'matrix$1('));

export const EguiPane = ({images}: EguiPaneProps) => {
    if (images.length === 0) return <p className="devtools__empty">No egui images have drawn yet.</p>;
    return (
        <div className="devtools__table-wrap">
            <table className="devtools__table">
                <thead>
                    <tr>
                        <th scope="col">Image</th>
                        <th scope="col" className="devtools__number">Layout</th>
                        <th scope="col" className="devtools__number">Backing</th>
                        <th scope="col" className="devtools__number">Zoom</th>
                        <th scope="col" className="devtools__number">egui pass</th>
                        <th scope="col" className="devtools__number">Pointer</th>
                        <th scope="col" className="devtools__wide">CSS transform</th>
                    </tr>
                </thead>
                <tbody>
                    {images.map(image => (
                        <tr key={image.id}>
                            <td>{image.title}</td>
                            <td className="devtools__number">
                                {Math.round(image.cssWidth)} × {Math.round(image.cssHeight)}
                            </td>
                            <td className="devtools__number">
                                {image.pixelWidth} × {image.pixelHeight}
                            </td>
                            <td className="devtools__number">{image.zoom.toFixed(1)}×</td>
                            <td className={image.frameMs > SLOW_FRAME_MS ? 'devtools__number devtools__slow' : 'devtools__number'}>{formatMs(image.frameMs)}</td>
                            <td className="devtools__number">{image.pointer ? `${Math.round(image.pointer[0])}, ${Math.round(image.pointer[1])}` : '—'}</td>
                            <td className="devtools__wide" title={image.transform}>
                                {describeTransform(image.transform)}
                            </td>
                        </tr>
                    ))}
                </tbody>
            </table>
        </div>
    );
};
