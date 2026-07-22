/**
 * Vitest configuration for browser-facing component and API contract tests.
 */

import path from 'node:path';
import { playwright } from '@vitest/browser-playwright';
import { defineConfig } from 'vitest/config';

const PROJECT_ROOT = process.cwd();
const JUNIT_OUTPUT_FILE = process.env.LITRADAR_VITEST_JUNIT?.trim();

export default defineConfig({
  resolve: {
    alias: {
      '@': path.resolve(PROJECT_ROOT),
    },
  },
  test: {
    reporters: JUNIT_OUTPUT_FILE
      ? process.env.GITHUB_ACTIONS === 'true'
        ? ['default', ['github-actions', { jobSummary: { enabled: false } }], 'junit']
        : ['default', 'junit']
      : process.env.GITHUB_ACTIONS === 'true'
        ? ['default', ['github-actions', { jobSummary: { enabled: false } }]]
        : ['default'],
    outputFile: JUNIT_OUTPUT_FILE ? { junit: JUNIT_OUTPUT_FILE } : undefined,
    projects: [
      {
        extends: true,
        test: {
          name: 'unit-jsdom',
          environment: 'jsdom',
          environmentOptions: {
            jsdom: {
              url: 'http://localhost/',
            },
          },
          globals: true,
          setupFiles: ['./tests/setup.tsx'],
          include: ['./tests/**/*.test.tsx'],
          exclude: ['./tests/browser-components/**'],
          clearMocks: true,
          restoreMocks: true,
        },
      },
      {
        extends: true,
        test: {
          name: 'component-browser',
          globals: true,
          setupFiles: ['./tests/browser-components/setup.tsx'],
          include: ['./tests/browser-components/**/*.browser.test.tsx'],
          clearMocks: true,
          restoreMocks: true,
          browser: {
            enabled: true,
            headless: true,
            provider: playwright({
              contextOptions: {
                permissions: ['clipboard-read', 'clipboard-write'],
              },
            }),
            instances: [{ browser: 'chromium' }],
            screenshotFailures: true,
            screenshotDirectory: './test-results/browser-components/screenshots',
          },
        },
      },
    ],
    coverage: {
      provider: 'v8',
      reporter: ['text', 'html', 'lcov'],
      reportsDirectory: './coverage',
      include: ['lib/**/*.tsx', 'components/**/*.tsx', 'app/**/*.tsx'],
      exclude: ['lib/generated/**', 'components/ui/**'],
    },
  },
});
