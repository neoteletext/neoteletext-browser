import type {StorageEntry} from '../../shared/devtools';
import {Icon} from '../../shared/Icon';
import {formatClock} from './format';

interface StoragePaneProps {
    snapshot: StorageEntry | null;
    storeId: string | null;
}

interface PairTableProps {
    caption: string;
    pairs: [string, string][];
    empty: string;
}

const PairTable = ({caption, pairs, empty}: PairTableProps) => (
    <div className="storage__group">
        <h3 className="storage__caption">{caption}</h3>
        {pairs.length === 0 ? (
            <p className="storage__none">{empty}</p>
        ) : (
            <table className="devtools__table">
                <tbody>
                    {pairs.map(([key, value]) => (
                        <tr key={key}>
                            <th scope="row" className="storage__key">
                                {key}
                            </th>
                            <td className="devtools__wide">{value}</td>
                        </tr>
                    ))}
                </tbody>
            </table>
        )}
    </div>
);

export const StoragePane = ({snapshot, storeId}: StoragePaneProps) => (
    <div className="storage">
        <p className="storage__store">
            <Icon name="shield"/>
            <span>
                Data store <code>{storeId ?? '—'}</code> belongs to this tab alone. Other tabs cannot read these values.
            </span>
            {snapshot && <time className="storage__time">as of {formatClock(snapshot.time)}</time>}
        </p>
        {snapshot ? (
            <div className="storage__groups">
                <PairTable caption="Cookies" pairs={snapshot.cookies} empty="No cookies set."/>
                <PairTable caption="Local storage" pairs={snapshot.local} empty="Nothing stored."/>
            </div>
        ) : (
            <p className="devtools__empty">Waiting for the tab to report its storage.</p>
        )}
    </div>
);
