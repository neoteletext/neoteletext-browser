import type {NetworkEntry} from '../../shared/devtools';
import {formatBytes, formatMs} from './format';

interface NetworkPaneProps {
    entries: NetworkEntry[];
}

const HTTP_OK_MAX = 399;

// Resource Timing omits the status on some engines; unknown is not failed.
const statusClass = (entry: NetworkEntry) => {
    if (entry.error) return 'status status--failed';
    if (entry.status === null) return 'status status--unknown';
    return entry.status <= HTTP_OK_MAX ? 'status status--ok' : 'status status--failed';
};

export const NetworkPane = ({entries}: NetworkPaneProps) => {
    if (entries.length === 0) return <p className="devtools__empty">No requests yet. Reload the tab to capture its loads.</p>;
    return (
        <div className="devtools__table-wrap">
            <table className="devtools__table">
                <thead>
                    <tr>
                        <th scope="col">Status</th>
                        <th scope="col">Method</th>
                        <th scope="col" className="devtools__wide">Address</th>
                        <th scope="col" className="devtools__number">Size</th>
                        <th scope="col" className="devtools__number">Time</th>
                    </tr>
                </thead>
                <tbody>
                    {entries.map((entry, index) => (
                        <tr key={index}>
                            <td>
                                <span className={statusClass(entry)}>{entry.error ? 'failed' : (entry.status ?? '—')}</span>
                            </td>
                            <td>{entry.method}</td>
                            <td className="devtools__wide" title={entry.error ?? entry.url}>
                                {entry.url}
                            </td>
                            <td className="devtools__number">{formatBytes(entry.bytes)}</td>
                            <td className="devtools__number">{formatMs(entry.durationMs)}</td>
                        </tr>
                    ))}
                </tbody>
            </table>
        </div>
    );
};
