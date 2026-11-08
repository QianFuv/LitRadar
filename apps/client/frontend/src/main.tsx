/**
 * Browser entry point that mounts the LitRadar single-page application.
 *
 * @packageDocumentation
 */

import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';

import { App } from '@/src/app';
import '@/src/globals.css';

const rootElement = document.getElementById('root');
if (!rootElement) {
  throw new Error('LitRadar root element is missing from index.html');
}

createRoot(rootElement).render(
  <StrictMode>
    <App />
  </StrictMode>,
);
