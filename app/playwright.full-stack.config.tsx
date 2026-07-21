/**
 * Playwright configuration for serial journeys against the real Rust service.
 */

import { defineConfig, devices } from '@playwright/test';

const BASE_URL = process.env.PLAYWRIGHT_FULL_STACK_BASE_URL?.trim();
const IS_CI = process.env.CI === 'true' || process.env.LITRADAR_TEST_CI === 'true';

if (!BASE_URL) {
  throw new Error('PLAYWRIGHT_FULL_STACK_BASE_URL is required');
}

export default defineConfig({
  testDir: './tests/e2e/full-stack',
  fullyParallel: false,
  forbidOnly: true,
  failOnFlakyTests: IS_CI,
  retries: IS_CI ? 1 : 0,
  workers: 1,
  reporter: IS_CI
    ? [
        ['list'],
        ['junit', { outputFile: './test-results/playwright-full-stack/junit.xml' }],
        ['html', { outputFolder: './playwright-report/full-stack', open: 'never' }],
      ]
    : 'list',
  outputDir: './test-results/playwright-full-stack/artifacts',
  use: {
    baseURL: BASE_URL,
    trace: IS_CI ? 'on-first-retry' : 'retain-on-failure',
    screenshot: 'only-on-failure',
    video: IS_CI ? 'on-first-retry' : 'off',
  },
  projects: [
    {
      name: 'full-stack-chromium',
      use: { ...devices['Desktop Chrome'] },
    },
  ],
});
