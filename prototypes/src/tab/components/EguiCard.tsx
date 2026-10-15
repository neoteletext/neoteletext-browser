import {useRef} from 'react';
import {FRAME_HEIGHT, FRAME_WIDTH, formatDuration, type Frame} from '../frames';
import {useEguiImage} from './useEguiImage';
import './EguiCard.css';

interface EguiCardProps {
    frame: Frame;
}

export const EguiCard = ({frame}: EguiCardProps) => {
    const canvasRef = useRef<HTMLCanvasElement>(null);
    const failure = useEguiImage(canvasRef, frame);

    return (
        <figure className="egui-card">
            <div className="egui-card__frame">
                <canvas ref={canvasRef} className="egui-card__canvas" aria-label={`${frame.title}, an interactive egui image`}/>
                {failure && <span className="egui-card__error">{failure}</span>}
            </div>
            <figcaption className="egui-card__caption">
                <span className="egui-card__title">{frame.title}</span>
                <span className="egui-card__template">{frame.template}</span>
                <dl className="egui-card__details">
                    <div>
                        <dt>Duration</dt>
                        <dd>{formatDuration(frame.durationSeconds)}</dd>
                    </div>
                    <div>
                        <dt>Picture</dt>
                        <dd>
                            {FRAME_WIDTH} × {FRAME_HEIGHT}
                        </dd>
                    </div>
                    <div>
                        <dt>Renderer</dt>
                        <dd>egui · WebGL 2</dd>
                    </div>
                </dl>
            </figcaption>
        </figure>
    );
};
