import {useEffect, useRef, useState, type FormEvent, type KeyboardEvent} from 'react';
import {Icon} from '../shared/Icon';
import {FOCUS_URL_EVENT} from './commands';
import './UrlBar.css';

interface UrlBarProps {
    src: string;
    storeId: string | null;
    // Resolves to an error message, or null when the address opened.
    onSubmit: (src: string) => Promise<string | null>;
}

// Keyed by tab and history position in the parent, so switching either
// resets the field to that page's address.
export const UrlBar = ({src, storeId, onSubmit}: UrlBarProps) => {
    const [value, setValue] = useState(src);
    const [error, setError] = useState<string | null>(null);
    const [busy, setBusy] = useState(false);
    const inputRef = useRef<HTMLInputElement>(null);

    useEffect(() => {
        const focus = () => {
            inputRef.current?.focus();
            inputRef.current?.select();
        };
        document.addEventListener(FOCUS_URL_EVENT, focus);
        return () => document.removeEventListener(FOCUS_URL_EVENT, focus);
    }, []);

    const handleSubmit = async (event: FormEvent) => {
        event.preventDefault();
        setBusy(true);
        const message = await onSubmit(value);
        setBusy(false);
        setError(message);
        if (!message) inputRef.current?.blur();
    };

    const handleKeyDown = (event: KeyboardEvent<HTMLInputElement>) => {
        if (event.key !== 'Escape') return;
        setValue(src);
        setError(null);
        inputRef.current?.blur();
    };

    return (
        <form className={error ? 'url-bar url-bar--error' : 'url-bar'} onSubmit={event => void handleSubmit(event)}>
            {storeId && (
                <span className="url-bar__sandbox" title={`This tab has its own data store (${storeId}…): cookies and storage are not shared`}>
                    <Icon name="shield"/>
                    <span className="url-bar__store">{storeId}</span>
                </span>
            )}
            <input
                ref={inputRef}
                id="address"
                className="url-bar__input"
                value={value}
                onChange={event => {
                    setValue(event.target.value);
                    setError(null);
                }}
                onKeyDown={handleKeyDown}
                onFocus={event => event.target.select()}
                placeholder="Address of a .t42 file"
                aria-label="Address"
                aria-invalid={error !== null}
                aria-describedby={error ? 'address-error' : undefined}
                spellCheck={false}
                autoCapitalize="off"
                autoCorrect="off"
                autoComplete="off"
                disabled={busy}
            />
            {error ? (
                <span id="address-error" className="url-bar__error" role="alert">
                    {error}
                </span>
            ) : (
                <span className="url-bar__kind">T42</span>
            )}
        </form>
    );
};
