import {StrictMode} from 'react';
import {createRoot} from 'react-dom/client';
import '../shared/tokens.css';
import {TabApp} from './components/TabApp';
import {installReporter} from './reporter';
import {runSandboxProbe} from './sandboxProbe';

// Before anything else, so the devtools see the page's own loads and logs.
installReporter();
void runSandboxProbe();

const src = new URLSearchParams(window.location.search).get('src') ?? '';
const root = document.getElementById('root');
if (!root) throw new Error('tab.html is missing #root');

createRoot(root).render(
    <StrictMode>
        <TabApp src={src}/>
    </StrictMode>,
);
