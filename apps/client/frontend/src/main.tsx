/**
 * Browser entry point that mounts the LitRadar single-page application.
 *
 * @packageDocumentation
 */

import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';

import { CLIENT_PLATFORM } from '@/lib/platform';
import { App } from '@/src/app';
import '@/src/globals.css';

const rootElement = document.getElementById('root');
if (!rootElement) {
  throw new Error('LitRadar root element is missing from index.html');
}

CLIENT_PLATFORM.prepareDocument();
createRoot(rootElement).render(
  <StrictMode>
    <App />
  </StrictMode>,
);
