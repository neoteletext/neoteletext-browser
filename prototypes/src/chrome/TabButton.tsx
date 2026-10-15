import {Icon} from '../shared/Icon';
import {Mosaic} from '../shared/Mosaic';
import {fileTitle} from '../shared/t42';
import {currentSrc, type Tab} from './tabs';

interface TabButtonProps {
    tab: Tab;
    active: boolean;
    onSelect: (id: string) => void;
    onClose: (id: string) => void;
}

export const TabButton = ({tab, active, onSelect, onClose}: TabButtonProps) => {
    const src = currentSrc(tab);
    return (
        <div className={active ? 'tab tab--active' : 'tab'} title={`${src}\nData store ${tab.storeId}`}>
            <button className="tab__select" type="button" role="tab" aria-selected={active} onClick={() => onSelect(tab.id)}>
                {tab.ready ? <Mosaic src={src} size="small"/> : <span className="tab__spinner" aria-label="Loading"/>}
                <span className="tab__title">{fileTitle(src)}</span>
            </button>
            <button className="tab__close" type="button" onClick={() => onClose(tab.id)} aria-label={`Close ${fileTitle(src)}`}>
                <Icon name="close"/>
            </button>
        </div>
    );
};
