/**
 * Next.js configuration for static production exports and one-origin local development.
 *
 * @packageDocumentation
 */

import type { NextConfig } from 'next';
import { PHASE_DEVELOPMENT_SERVER } from 'next/constants';

const DEVELOPMENT_BACKEND_URL = 'http://127.0.0.1:8001';

/**
 * Build the development-only backend rewrites.
 *
 * @returns Rewrites that proxy backend namespaces to the internal Rust listener.
 */
const DEVELOPMENT_REWRITES: NonNullable<NextConfig['rewrites']> = async () => ({
  beforeFiles: [],
  afterFiles: [],
  fallback: [
    {
      source: '/api/:path*',
      destination: `${DEVELOPMENT_BACKEND_URL}/api/:path*`,
    },
    {
      source: '/mcp/:path*',
      destination: `${DEVELOPMENT_BACKEND_URL}/mcp/:path*`,
    },
    {
      source: '/docs/',
      destination: `${DEVELOPMENT_BACKEND_URL}/docs/`,
    },
    {
      source: '/docs/:path*',
      destination: `${DEVELOPMENT_BACKEND_URL}/docs/:path*`,
    },
    {
      source: '/openapi.json',
      destination: `${DEVELOPMENT_BACKEND_URL}/openapi.json`,
    },
  ],
});

/**
 * Select deterministic development rewrites or production static export behavior.
 *
 * @param phase - Current Next.js build or development phase.
 * @returns Next.js configuration for the selected phase.
 */
export default function buildNextConfig(phase: string): NextConfig {
  return {
    ...(phase === PHASE_DEVELOPMENT_SERVER
      ? {
          rewrites: DEVELOPMENT_REWRITES,
          skipTrailingSlashRedirect: true,
        }
      : { output: 'export' }),
    allowedDevOrigins: ['127.0.0.1'],
    images: {
      unoptimized: true,
    },
  };
}
