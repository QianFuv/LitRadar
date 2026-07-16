/**
 * Next.js configuration for static production exports and one-origin local development.
 *
 * @packageDocumentation
 */

import type { NextConfig } from 'next';

const BACKEND_URL = process.env.INTERNAL_API_URL || 'http://localhost:8001';
const IS_DEVELOPMENT = process.env.NODE_ENV === 'development';

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
      destination: `${BACKEND_URL}/api/:path*`,
    },
    {
      source: '/mcp/:path*',
      destination: `${BACKEND_URL}/mcp/:path*`,
    },
    {
      source: '/docs/',
      destination: `${BACKEND_URL}/docs/`,
    },
    {
      source: '/docs/:path*',
      destination: `${BACKEND_URL}/docs/:path*`,
    },
    {
      source: '/openapi.json',
      destination: `${BACKEND_URL}/openapi.json`,
    },
  ],
});

const NEXT_CONFIG: NextConfig = {
  ...(IS_DEVELOPMENT
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

export default NEXT_CONFIG;
