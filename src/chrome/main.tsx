import {StrictMode} from 'react';
import {createRoot} from 'react-dom/client';
import '../shared/tokens.css';
import {ChromeApp} from './ChromeApp';

const root = document.getElementById('root');
if (!root) throw new Error('index.html is missing #root');

createRoot(root).render(
    <StrictMode>
        <ChromeApp/>
    </StrictMode>,
);
