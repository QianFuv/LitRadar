/**
 * Playwright configuration for local fixture-only browser flows.
 */

import { defineConfig, devices } from '@playwright/test';

const MANAGED_BASE_URL = 'http://127.0.0.1:3100';
const EXTERNAL_BASE_URL = process.env.PLAYWRIGHT_BASE_URL?.trim();
const BASE_URL = EXTERNAL_BASE_URL || MANAGED_BASE_URL;
const IS_CI = process.env.CI === 'true' || process.env.LITRADAR_TEST_CI === 'true';

export default defineConfig({
  testDir: './tests/e2e',
  testMatch: 'local-fixtures.spec.tsx',
  fullyParallel: true,
  forbidOnly: IS_CI,
  failOnFlakyTests: IS_CI,
  retries: IS_CI ? 1 : 0,
  workers: IS_CI ? 1 : undefined,
  reporter: IS_CI
    ? [
        ['list'],
        ['junit', { outputFile: './test-results/playwright-fixtures/junit.xml' }],
        ['html', { outputFolder: './playwright-report/fixtures', open: 'never' }],
      ]
    : 'list',
  outputDir: './test-results/playwright-fixtures/artifacts',
  use: {
    baseURL: BASE_URL,
    trace: IS_CI ? 'on-first-retry' : 'retain-on-failure',
    screenshot: 'only-on-failure',
    video: IS_CI ? 'on-first-retry' : 'off',
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
        reuseExistingServer: !IS_CI,
        timeout: 120_000,
        stdout: 'ignore',
        stderr: 'pipe',
      },
});
