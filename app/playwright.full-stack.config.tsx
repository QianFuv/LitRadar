/**
 * Playwright configuration for serial journeys against the real Rust service.
 */

import { defineConfig, devices } from '@playwright/test';

const BASE_URL = process.env.PLAYWRIGHT_FULL_STACK_BASE_URL?.trim();

if (!BASE_URL) {
  throw new Error('PLAYWRIGHT_FULL_STACK_BASE_URL is required');
}

export default defineConfig({
  testDir: './tests/e2e/full-stack',
  fullyParallel: false,
  forbidOnly: true,
  retries: 0,
  workers: 1,
  reporter: 'list',
  outputDir: './test-results/full-stack',
  use: {
    baseURL: BASE_URL,
    trace: 'retain-on-failure',
    screenshot: 'only-on-failure',
  },
  projects: [
    {
      name: 'full-stack-chromium',
      use: { ...devices['Desktop Chrome'] },
    },
  ],
});
