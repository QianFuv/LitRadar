/**
 * Vitest configuration for browser-facing component and API contract tests.
 */

import { readdirSync, readFileSync } from 'node:fs';
import path from 'node:path';
import { playwright } from '@vitest/browser-playwright';
import { defineConfig } from 'vitest/config';

const PROJECT_ROOT = process.cwd();
const JUNIT_OUTPUT_FILE = process.env.LITRADAR_VITEST_JUNIT?.trim();

/**
 * React entry points every pre-bundled dependency must import instead of embedding, so the
 * renderer, hooks, and Testing Library share one React instance.
 */
const SHARED_REACT_ENTRY_POINTS = [
  'react',
  'react-dom',
  'react-dom/client',
  'react/jsx-runtime',
  'react/jsx-dev-runtime',
];

/**
 * Collect the Animate UI entry points the application and tests import.
 *
 * Animate UI only exposes a `./*` subpath pattern, which dependency optimization cannot expand,
 * so the entry points are read from source to keep new imports pre-bundled automatically.
 *
 * @returns Sorted, unique Animate UI module specifiers.
 */
function findAnimateUiEntryPoints(): string[] {
  const specifiers = new Set<string>();
  for (const directory of ['components', 'lib', 'src', 'tests']) {
    const directoryPath = path.join(PROJECT_ROOT, directory);
    for (const file of readdirSync(directoryPath, { encoding: 'utf8', recursive: true })) {
      if (!file.endsWith('.tsx')) {
        continue;
      }
      const source = readFileSync(path.join(directoryPath, file), 'utf8');
      for (const [, specifier] of source.matchAll(/['"](@animate-ui\/react\/[^'"]+)['"]/g)) {
        specifiers.add(specifier);
      }
    }
  }
  return [...specifiers].sort();
}

/**
 * Many-file ESM UI dependencies bundled into a few cached modules for jsdom tests.
 *
 * Every test file runs in a fresh worker, and loading these packages natively resolves and reads
 * thousands of files each time; that file-system work stretched lazy route loads and setup past
 * test timeouts. `motion/react` must stay in this list so setup and the Animate UI bundles share
 * the instance whose `MotionGlobalConfig.skipAnimations` is enabled.
 */
const PREBUNDLED_UI_DEPENDENCIES = [
  'motion/react',
  ...findAnimateUiEntryPoints(),
  'radix-ui',
  '@radix-ui/react-label',
  '@radix-ui/react-scroll-area',
  '@radix-ui/react-slot',
  'lucide-react',
  '@xyflow/react',
];

/**
 * Share of logical CPUs that runs jsdom test files at once.
 *
 * jsdom rendering is CPU-bound, and the Vite server in the main process also needs a core. On
 * hybrid or SMT CPUs one worker per logical CPU roughly halves each worker's speed, which pushed
 * interaction-heavy tests past their timeouts while finishing the suite no sooner.
 */
const JSDOM_MAX_WORKERS = '50%';

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
          maxWorkers: JSDOM_MAX_WORKERS,
          deps: {
            optimizer: {
              client: {
                enabled: true,
                include: PREBUNDLED_UI_DEPENDENCIES,
                exclude: SHARED_REACT_ENTRY_POINTS,
              },
            },
          },
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
            api: {
              port: 46115,
            },
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
      include: ['lib/**/*.tsx', 'components/**/*.tsx', 'src/**/*.tsx'],
      exclude: ['lib/generated/**', 'components/ui/**'],
    },
  },
});
