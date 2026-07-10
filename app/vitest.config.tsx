/**
 * Vitest configuration for browser-facing component and API contract tests.
 */

import path from 'node:path';
import { defineConfig } from 'vitest/config';

const PROJECT_ROOT = process.cwd();

export default defineConfig({
  resolve: {
    alias: {
      '@': path.resolve(PROJECT_ROOT),
    },
  },
  test: {
    environment: 'jsdom',
    globals: true,
    setupFiles: ['./tests/setup.tsx'],
    include: ['./tests/**/*.test.tsx'],
    clearMocks: true,
    restoreMocks: true,
    coverage: {
      provider: 'v8',
      reporter: ['text', 'lcov'],
      include: ['lib/**/*.tsx', 'components/**/*.tsx', 'app/**/*.tsx'],
      exclude: ['lib/generated/**', 'components/ui/**'],
    },
  },
});
