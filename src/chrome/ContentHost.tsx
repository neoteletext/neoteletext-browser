import {useLayoutEffect, useRef} from 'react';
import {errorMessage, setContentBounds} from './commands';
import {EmptyState} from './EmptyState';
import './ContentHost.css';

interface ContentHostProps {
    empty: boolean;
    onOpenSample: () => void;
}

// The tabs are native webviews laid over this element. Whenever its box
// changes (window resize, devtools dock opening) Rust is told the new
// rectangle and moves every tab to match.
export const ContentHost = ({empty, onOpenSample}: ContentHostProps) => {
    const hostRef = useRef<HTMLElement>(null);

    useLayoutEffect(() => {
        const host = hostRef.current;
        if (!host) return;
        let frame = 0;
        const sync = () => {
            cancelAnimationFrame(frame);
            frame = requestAnimationFrame(() => {
                const box = host.getBoundingClientRect();
                const bounds = {x: Math.round(box.left), y: Math.round(box.top), width: Math.round(box.width), height: Math.round(box.height)};
                setContentBounds(bounds).catch(error => console.error('Could not place tabs:', errorMessage(error)));
            });
        };
        const observer = new ResizeObserver(sync);
        observer.observe(host);
        window.addEventListener('resize', sync);
        return () => {
            cancelAnimationFrame(frame);
            observer.disconnect();
            window.removeEventListener('resize', sync);
        };
    }, []);

    return (
        <main ref={hostRef} className="content-host">
            {empty && <EmptyState onOpenSample={onOpenSample}/>}
        </main>
    );
};
