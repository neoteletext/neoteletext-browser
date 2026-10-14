import {useState} from 'react';
import {WALL_TRANSFORMS, dispatchTransform, type WallTransform} from '../wallTransform';
import './TransformPicker.css';

const LABELS: Record<WallTransform, string> = {flat: 'Flat', tilt: 'Tilt', fan: 'Fan'};

export const TransformPicker = () => {
    const [selected, setSelected] = useState<WallTransform>('flat');

    const choose = (transform: WallTransform) => {
        setSelected(transform);
        dispatchTransform(transform);
    };

    return (
        <fieldset className="transform-picker">
            <legend className="transform-picker__legend">CSS transform</legend>
            <div className="transform-picker__options">
                {WALL_TRANSFORMS.map(transform => (
                    <label key={transform} className="transform-picker__option">
                        <input
                            id={`transform-${transform}`}
                            type="radio"
                            name="wall-transform"
                            value={transform}
                            checked={selected === transform}
                            onChange={() => choose(transform)}
                        />
                        <span>{LABELS[transform]}</span>
                    </label>
                ))}
            </div>
        </fieldset>
    );
};
