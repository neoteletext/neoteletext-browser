import {useLayoutEffect, useRef} from 'react';
import type {ConsoleEntry} from '../../shared/devtools';
import {formatClock} from './format';

interface ConsolePaneProps {
    entries: ConsoleEntry[];
}

// Pixels from the bottom that still count as "following" the log.
const FOLLOW_SLACK = 24;

export const ConsolePane = ({entries}: ConsolePaneProps) => {
    const listRef = useRef<HTMLOListElement>(null);
    const following = useRef(true);

    // Stay pinned to the newest line unless the reader has scrolled up.
    useLayoutEffect(() => {
        const list = listRef.current;
        if (list && following.current) list.scrollTop = list.scrollHeight;
    }, [entries]);

    if (entries.length === 0) return <p className="devtools__empty">Nothing logged yet.</p>;

    return (
        <ol
            ref={listRef}
            className="console"
            onScroll={event => {
                const list = event.currentTarget;
                following.current = list.scrollHeight - list.scrollTop - list.clientHeight < FOLLOW_SLACK;
            }}
        >
            {entries.map((entry, index) => (
                <li key={index} className={`console__row console__row--${entry.level}`}>
                    <time className="console__time">{formatClock(entry.time)}</time>
                    <span className="console__message">{entry.message}</span>
                </li>
            ))}
        </ol>
    );
};
