/**
 * Vite configuration for the LitRadar single-page client and its local backend proxy.
 *
 * @packageDocumentation
 */

import path from 'node:path';
import { fileURLToPath } from 'node:url';
import tailwindcss from '@tailwindcss/vite';
import react from '@vitejs/plugin-react';
import { defineConfig, type ProxyOptions } from 'vite';

const PROJECT_ROOT = path.dirname(fileURLToPath(import.meta.url));
const DEVELOPMENT_BACKEND_URL = process.env.LITRADAR_BACKEND_URL?.trim() || 'http://127.0.0.1:8001';

/**
 * Build the backend namespace proxy shared by the development and preview servers.
 *
 * @param target - Backend origin that receives proxied requests.
 * @returns Proxy rules keyed by anchored path patterns.
 */
function createBackendProxy(target: string): Record<string, ProxyOptions> {
  const backend: ProxyOptions = { target, changeOrigin: false };
  return {
    '^/api(?:/|$)': backend,
    '^/mcp(?:/|$)': backend,
    '^/docs(?:/|$)': backend,
    '^/openapi\\.json$': backend,
    '^/health(?:/|$)': backend,
  };
}

const BACKEND_PROXY = createBackendProxy(DEVELOPMENT_BACKEND_URL);

export default defineConfig({
  plugins: [react(), tailwindcss()],
  resolve: {
    alias: {
      '@': PROJECT_ROOT,
    },
  },
  build: {
    outDir: 'dist',
    emptyOutDir: true,
    sourcemap: false,
  },
  server: {
    proxy: BACKEND_PROXY,
  },
  preview: {
    proxy: BACKEND_PROXY,
  },
});
