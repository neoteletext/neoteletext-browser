import {Icon} from '../shared/Icon';
import {TabButton} from './TabButton';
import type {Tab} from './tabs';
import './TabStrip.css';

interface TabStripProps {
    tabs: Tab[];
    activeId: string | null;
    onSelect: (id: string) => void;
    onClose: (id: string) => void;
    onNew: () => void;
}

// data-tauri-drag-region drags the window only when the press lands on the
// element carrying it, so the strip and its trailing space both carry it
// while the tabs themselves stay clickable.
export const TabStrip = ({tabs, activeId, onSelect, onClose, onNew}: TabStripProps) => (
    <header className="tab-strip" data-tauri-drag-region>
        <div className="tab-strip__tabs" role="tablist" aria-label="Tabs">
            {tabs.map(tab => (
                <TabButton key={tab.id} tab={tab} active={tab.id === activeId} onSelect={onSelect} onClose={onClose}/>
            ))}
        </div>
        <button className="tab-strip__new" type="button" onClick={onNew} aria-label="New tab" title="New tab (⌘T)">
            <Icon name="plus"/>
        </button>
        <div className="tab-strip__drag" data-tauri-drag-region/>
    </header>
);
