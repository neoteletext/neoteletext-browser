import './Icon.css';

// 16-unit grid, 1.5 stroke: drawn to sit with SF Symbols at small sizes.
const PATHS = {
    back: 'M10 3.5 5.5 8 10 12.5',
    forward: 'M6 3.5 10.5 8 6 12.5',
    reload: 'M12.6 9.2A4.75 4.75 0 1 1 11.4 4.6M12 2.6v2.6H9.4',
    close: 'M4.5 4.5l7 7M11.5 4.5l-7 7',
    plus: 'M8 3.25v9.5M3.25 8h9.5',
    devtools: 'M5.5 4.5 2 8l3.5 3.5M10.5 4.5 14 8l-3.5 3.5',
    shield: 'M8 1.9 3.2 3.7v3.9c0 2.9 2 5.2 4.8 6.4 2.8-1.2 4.8-3.5 4.8-6.4V3.7Z',
    file: 'M4.2 1.9h4.9l2.7 2.7v9.5H4.2ZM9 1.9v2.8h2.8',
    trash: 'M3 4.5h10M6.2 4.5V3h3.6v1.5M4.5 4.5l.6 8.5h5.8l.6-8.5',
} as const;

export type IconName = keyof typeof PATHS;

interface IconProps {
    name: IconName;
}

export const Icon = ({name}: IconProps) => (
    <svg className="icon" viewBox="0 0 16 16" aria-hidden="true">
        <path d={PATHS[name]}/>
    </svg>
);
