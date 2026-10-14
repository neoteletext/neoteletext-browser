import {useEffect} from 'react';
import {loadEgui} from '../egui';
import {reportReady} from '../reporter';
import {PageHeader} from './PageHeader';
import {Wall} from './Wall';
import './TabApp.css';

interface TabAppProps {
    src: string;
}

export const TabApp = ({src}: TabAppProps) => {
    // The tab strip shows a spinner until the egui module is ready.
    useEffect(() => {
        loadEgui()
            .then(reportReady)
            .catch((error: unknown) => console.error('The egui module failed to load:', error));
    }, []);

    return (
        <>
            <PageHeader src={src}/>
            <main className="tab-main">
                <Wall/>
            </main>
        </>
    );
};
