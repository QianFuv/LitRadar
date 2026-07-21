/**
 * Playwright configuration for local fixture-only browser flows.
 */

import { defineConfig, devices } from '@playwright/test';

const MANAGED_BASE_URL = 'http://127.0.0.1:3100';
const EXTERNAL_BASE_URL = process.env.PLAYWRIGHT_BASE_URL?.trim();
const BASE_URL = EXTERNAL_BASE_URL || MANAGED_BASE_URL;

export default defineConfig({
  testDir: './tests/e2e',
  testMatch: 'local-fixtures.spec.tsx',
  fullyParallel: true,
  forbidOnly: Boolean(process.env.CI),
  retries: process.env.CI ? 2 : 0,
  workers: process.env.CI ? 1 : undefined,
  reporter: 'list',
  outputDir: './test-results',
  use: {
    baseURL: BASE_URL,
    trace: 'on-first-retry',
    screenshot: 'only-on-failure',
  },
  projects: [
    {
      name: 'fixture-chromium',
      use: { ...devices['Desktop Chrome'] },
    },
  ],
  webServer: EXTERNAL_BASE_URL
    ? undefined
    : {
        command: 'pnpm exec next dev --hostname 127.0.0.1 --port 3100',
        url: MANAGED_BASE_URL,
        reuseExistingServer: !process.env.CI,
        timeout: 120_000,
        stdout: 'ignore',
        stderr: 'pipe',
      },
});
