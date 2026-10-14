import {Mosaic} from '../shared/Mosaic';
import {SAMPLE_SRC} from '../shared/t42';
import './EmptyState.css';

interface EmptyStateProps {
    onOpenSample: () => void;
}

export const EmptyState = ({onOpenSample}: EmptyStateProps) => (
    <section className="empty-state">
        <Mosaic src={SAMPLE_SRC} size="large"/>
        <h1 className="empty-state__title">No tabs open</h1>
        <p className="empty-state__text">
            Type the address of a <code>.t42</code> file in the bar above, or open the sample wall. Each tab runs in its own sandbox with its own
            cookies and storage.
        </p>
        <button className="empty-state__button" type="button" onClick={onOpenSample}>
            Open sample wall
        </button>
    </section>
);
