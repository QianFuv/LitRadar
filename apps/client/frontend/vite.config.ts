/**
 * Vite+ configuration for the LitRadar single-page client: build, local backend proxy, tests,
 * lint, and formatting.
 *
 * The default `web` mode builds the browser client. `vp build --mode native` builds the Wails
 * desktop and Android frontend by aliasing the active platform module to the native
 * implementation, so the Wails runtime stays out of the web bundle.
 *
 * @packageDocumentation
 */

import { readdirSync, readFileSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import babel from '@rolldown/plugin-babel';
import stylex from '@stylexjs/unplugin';
import react, { reactCompilerPreset } from '@vitejs/plugin-react';
import browserslist from 'browserslist';
import { browserslistToTargets } from 'lightningcss';
import { defineConfig, type ProxyOptions } from 'vite-plus';
import { playwright } from 'vite-plus/test/browser-playwright';

const PROJECT_ROOT = path.dirname(fileURLToPath(import.meta.url));
const DEVELOPMENT_BACKEND_URL = process.env.LITRADAR_BACKEND_URL?.trim() || 'http://127.0.0.1:8001';
const JUNIT_OUTPUT_FILE = process.env.LITRADAR_VITEST_JUNIT?.trim();
const IS_GITHUB_ACTIONS = process.env.GITHUB_ACTIONS === 'true';

/** Vite mode that builds the frontend embedded in the native Wails hosts. */
const NATIVE_MODE = 'native';

/**
 * Compatibility baseline: every browser release since January 1 of the previous year. It sets
 * Vite's script and CSS targets and the targets StyleX passes to Lightning CSS.
 */
const BROWSERSLIST_QUERY = `since ${new Date().getFullYear() - 1}-01-01`;

/**
 * Browserslist names mapped to the engine names that Vite's `build.target` understands.
 *
 * Browsers without an engine name (such as Samsung Internet) share an engine already listed.
 */
const BUILD_TARGET_ENGINES = new Map(
  Object.entries({
    and_chr: 'chrome',
    and_ff: 'firefox',
    android: 'chrome',
    chrome: 'chrome',
    edge: 'edge',
    firefox: 'firefox',
    ios_saf: 'ios',
    opera: 'opera',
    safari: 'safari',
  }),
);

/**
 * Convert a browserslist query into the oldest version of each engine for `build.target`.
 *
 * @param query - Browserslist query that defines the supported browsers.
 * @returns Sorted engine targets such as `chrome132` and `safari18.3`.
 */
function resolveBuildTargets(query: string): string[] {
  const oldestVersions = new Map<string, number>();
  for (const entry of browserslist(query)) {
    const [browser, versionRange] = entry.split(' ');
    const engine = BUILD_TARGET_ENGINES.get(browser);
    const version = Number.parseFloat(versionRange.split('-')[0]);
    if (!engine || Number.isNaN(version)) {
      continue;
    }
    const oldestVersion = oldestVersions.get(engine);
    if (oldestVersion === undefined || version < oldestVersion) {
      oldestVersions.set(engine, version);
    }
  }
  return [...oldestVersions].map(([engine, version]) => `${engine}${version}`).sort();
}

const BUILD_TARGETS = resolveBuildTargets(BROWSERSLIST_QUERY);

/**
 * Build the backend namespace proxy shared by the development and preview servers.
 *
 * @param target - Backend origin that receives proxied requests.
 * @returns Proxy rules keyed by anchored path patterns.
 */
function createBackendProxy(target: string) {
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
 * Collect the Animate UI and Base UI entry points the application and tests import.
 *
 * Animate UI only exposes a `./*` subpath pattern, which dependency optimization cannot expand,
 * and Base UI is imported per component, so the entry points are read from source to keep new
 * imports pre-bundled automatically.
 *
 * @returns Sorted, unique Animate UI and Base UI module specifiers.
 */
function findUiEntryPoints(): string[] {
  const specifiers = new Set<string>();
  for (const directory of ['components', 'lib', 'src', 'tests']) {
    const directoryPath = path.join(PROJECT_ROOT, directory);
    for (const file of readdirSync(directoryPath, { encoding: 'utf8', recursive: true })) {
      if (!file.endsWith('.tsx')) {
        continue;
      }
      const source = readFileSync(path.join(directoryPath, file), 'utf8');
      for (const [, specifier] of source.matchAll(
        /['"](@animate-ui\/react\/[^'"]+|@base-ui\/react(?:\/[^'"]+)?)['"]/g,
      )) {
        specifiers.add(specifier);
      }
    }
  }
  return [...specifiers].sort();
}

/**
 * Many-file ESM UI dependencies bundled into a few cached modules for jsdom and browser tests.
 *
 * Every test file runs in a fresh worker, and loading these packages natively resolves and reads
 * thousands of files each time; that file-system work stretched lazy route loads and setup past
 * test timeouts. `motion/react` must stay in this list so setup and the Animate UI bundles share
 * the instance whose `MotionGlobalConfig.skipAnimations` is enabled. Browser tests pre-bundle the
 * same list so a cold dependency cache does not make Vite reload a running test file.
 */
const PREBUNDLED_UI_DEPENDENCIES = [
  'motion/react',
  ...findUiEntryPoints(),
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

/** Generated sources that lint and formatting never touch. */
const GENERATED_SOURCE_PATTERNS = ['lib/generated/**', 'bindings/**', '**/routeTree.gen.*'];

/** Tool output and dependency directories outside the source tree. */
const OUTPUT_PATTERNS = [
  'node_modules/**',
  'dist/**',
  'coverage/**',
  'playwright-report/**',
  'test-results/**',
];

export default defineConfig(({ mode }) => ({
  plugins: [
    stylex.vite({
      aliases: { '@/*': [path.join(PROJECT_ROOT, '*')] },
      unstable_moduleResolution: { type: 'commonJS', rootDir: PROJECT_ROOT },
      styleResolution: 'application-order',
      lightningcssOptions: { targets: browserslistToTargets(browserslist(BROWSERSLIST_QUERY)) },
    }),
    react(),
    babel({ presets: [reactCompilerPreset()] }),
  ],
  resolve: {
    alias: [
      ...(mode === NATIVE_MODE
        ? [
            {
              find: /^@\/lib\/platform\/active$/,
              replacement: path.join(PROJECT_ROOT, 'lib', 'platform', 'native.tsx'),
            },
          ]
        : []),
      { find: '@', replacement: PROJECT_ROOT },
    ],
  },
  build: {
    outDir: 'dist',
    emptyOutDir: true,
    sourcemap: false,
    target: BUILD_TARGETS,
    cssTarget: BUILD_TARGETS,
  },
  server: {
    proxy: BACKEND_PROXY,
  },
  preview: {
    proxy: BACKEND_PROXY,
  },
  lint: {
    plugins: ['typescript', 'react', 'jsx-a11y', 'oxc'],
    jsPlugins: [
      { name: 'stylex', specifier: '@stylexjs/eslint-plugin' },
      { name: 'anti-slop', specifier: 'oxlint-anti-slop' },
    ],
    options: {
      typeAware: true,
      typeCheck: true,
    },
    env: {
      browser: true,
    },
    ignorePatterns: [...OUTPUT_PATTERNS, ...GENERATED_SOURCE_PATTERNS, 'public/**'],
    rules: {
      'no-empty': ['error', { allowEmptyCatch: true }],
      'no-control-regex': 'off',
      'react/rules-of-hooks': 'error',
      'react/exhaustive-deps': 'error',
      'jsx-a11y/no-autofocus': 'off',
      'jsx-a11y/no-noninteractive-tabindex': ['error', { tags: ['section'], roles: ['region'] }],
      'jsx-a11y/label-has-associated-control': ['error', { controlComponents: ['Checkbox'] }],
      'stylex/valid-styles': 'error',
      'stylex/valid-shorthands': 'error',
      'stylex/no-unused': 'error',
      'stylex/no-legacy-contextual-styles': 'error',
      'stylex/no-lookahead-selectors': 'error',
      'stylex/no-conflicting-props': 'error',
      'anti-slop/no-chained-type-assertions': 'error',
      'anti-slop/no-object-parameters': 'error',
      'anti-slop/no-meaningless-unknown-aliases': 'error',
      'anti-slop/no-unsafe-dictionary-values': 'error',
      'anti-slop/no-narrowing-after-widening': 'error',
      'anti-slop/no-reduce-accumulator-copy': 'error',
      'anti-slop/no-unnecessary-type-widening': 'error',
      'anti-slop/no-typeof-outside-guards': 'error',
    },
    overrides: [
      {
        files: ['src/**/*.tsx'],
        rules: {
          'react/only-export-components': ['error', { allowConstantExport: true }],
        },
      },
      {
        files: ['tests/**'],
        rules: {
          'anti-slop/no-chained-type-assertions': 'off',
        },
      },
      {
        files: ['components/ui/**/*.tsx'],
        rules: {
          'jsx-a11y/heading-has-content': 'off',
        },
      },
      {
        files: ['**/*.{js,mjs}', '*.config.{ts,tsx}'],
        env: {
          browser: false,
          node: true,
        },
      },
    ],
  },
  fmt: {
    printWidth: 100,
    semi: true,
    singleQuote: true,
    trailingComma: 'all',
    ignorePatterns: [...OUTPUT_PATTERNS, 'bindings/**', '**/routeTree.gen.*', 'pnpm-lock.yaml'],
  },
  test: {
    reporters: JUNIT_OUTPUT_FILE
      ? IS_GITHUB_ACTIONS
        ? ['default', ['github-actions', { jobSummary: { enabled: false } }], 'junit']
        : ['default', 'junit']
      : IS_GITHUB_ACTIONS
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
        optimizeDeps: {
          include: PREBUNDLED_UI_DEPENDENCIES,
        },
        test: {
          name: 'component-browser',
          globals: true,
          setupFiles: ['./tests/browser-components/setup.tsx'],
          include: ['./tests/browser-components/**/*.browser.test.tsx'],
          clearMocks: true,
          restoreMocks: true,
          api: {
            port: 46115,
          },
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
      include: ['lib/**/*.tsx', 'components/**/*.tsx', 'src/**/*.tsx'],
      exclude: ['lib/generated/**', 'components/ui/**'],
    },
  },
}));
