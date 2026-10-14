import type {ReactNode} from 'react';
import {Icon} from '../shared/Icon';
import './Toolbar.css';

interface ToolbarProps {
    canGoBack: boolean;
    canGoForward: boolean;
    canReload: boolean;
    devtoolsOpen: boolean;
    onBack: () => void;
    onForward: () => void;
    onReload: () => void;
    onToggleDevtools: () => void;
    children: ReactNode;
}

export const Toolbar = (props: ToolbarProps) => (
    <nav className="toolbar" aria-label="Navigation">
        <div className="toolbar__group">
            <button className="toolbar__button" type="button" onClick={props.onBack} disabled={!props.canGoBack} aria-label="Back" title="Back (⌘[)">
                <Icon name="back"/>
            </button>
            <button className="toolbar__button" type="button" onClick={props.onForward} disabled={!props.canGoForward} aria-label="Forward" title="Forward (⌘])">
                <Icon name="forward"/>
            </button>
            <button className="toolbar__button" type="button" onClick={props.onReload} disabled={!props.canReload} aria-label="Reload" title="Reload (⌘R)">
                <Icon name="reload"/>
            </button>
        </div>
        {props.children}
        <button
            className={props.devtoolsOpen ? 'toolbar__button toolbar__button--on' : 'toolbar__button'}
            type="button"
            onClick={props.onToggleDevtools}
            aria-pressed={props.devtoolsOpen}
            aria-label="Developer tools"
            title="Developer tools (⌥⌘I)"
        >
            <Icon name="devtools"/>
        </button>
    </nav>
);
