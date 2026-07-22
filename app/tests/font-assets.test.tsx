/**
 * Deterministic local-font asset and global activation coverage.
 */

import { existsSync, readFileSync, readdirSync } from 'node:fs';
import path from 'node:path';
import { describe, expect, test } from 'vitest';

const PROJECT_ROOT = process.cwd();
const FONT_FAMILY = 'JetBrainsLxgwNerdMono';
const FONT_ASSET_EXPECTATIONS = [
  {
    directoryName: 'JetBrainsLxgwNerdMono-Regular',
    faceCount: 344,
    fontWeight: 400,
  },
  {
    directoryName: 'JetBrainsLxgwNerdMono-Bold',
    faceCount: 324,
    fontWeight: 700,
  },
] as const;
const FORBIDDEN_ARCHIVE_FILES = ['index.html', 'index.proto', 'reporter.bin'] as const;

/**
 * Read one UTF-8 project file relative to the frontend package root.
 *
 * @param relativePath - Frontend-relative source path.
 * @returns File contents.
 */
function readProjectFile(relativePath: string): string {
  return readFileSync(path.resolve(PROJECT_ROOT, relativePath), 'utf8');
}

/**
 * Extract relative WOFF2 filenames from the generated font stylesheet.
 *
 * @param stylesheet - Generated font stylesheet.
 * @returns Referenced WOFF2 basenames in source order.
 */
function extractWoff2References(stylesheet: string): string[] {
  return Array.from(
    stylesheet.matchAll(/url\((?:["']?)\.\/([^"')]+\.woff2)(?:["']?)\)/g),
    (match) => match[1],
  );
}

/** Verify every generated stylesheet and extracted font directory form an exact closed set. */
function matchesGeneratedFontAssetSets(): void {
  for (const expectation of FONT_ASSET_EXPECTATIONS) {
    const fontDirectory = path.resolve(PROJECT_ROOT, 'assets', expectation.directoryName);
    const resultCssPath = path.join(fontDirectory, 'result.css');

    expect(existsSync(fontDirectory)).toBe(true);
    expect(existsSync(resultCssPath)).toBe(true);

    const assetNames = readdirSync(fontDirectory).sort();
    const woff2Files = assetNames.filter((name) => name.endsWith('.woff2'));
    const stylesheet = readFileSync(resultCssPath, 'utf8');
    const woff2References = extractWoff2References(stylesheet);

    expect(woff2Files).toHaveLength(expectation.faceCount);
    expect(woff2References).toHaveLength(expectation.faceCount);
    expect(new Set(woff2References).size).toBe(expectation.faceCount);
    expect([...woff2References].sort()).toEqual(woff2Files);
    expect(stylesheet.match(/@font-face/g)).toHaveLength(expectation.faceCount);
    expect(
      stylesheet.match(new RegExp(`font-weight:\\s*${expectation.fontWeight}`, 'g')),
    ).toHaveLength(expectation.faceCount);
    expect(stylesheet).toMatch(new RegExp(`font-family:\\s*["']${FONT_FAMILY}["']`));
    expect(stylesheet).toMatch(/font-display:\s*swap/);
    expect(stylesheet).toContain('Copyright (c) 2024 lvbibir');
    expect(stylesheet).toContain('SIL Open Font License, Version 1.1');
    expect(stylesheet).toContain('JetBrains Mono: OFL-1.1, LXGW WenKai: OFL-1.1, Nerd Fonts: MIT.');
    expect(assetNames).toEqual(['result.css', ...woff2Files].sort());

    for (const forbiddenFile of FORBIDDEN_ARCHIVE_FILES) {
      expect(assetNames).not.toContain(forbiddenFile);
    }
  }
}

/** Verify global source and documentation activate both local font weights. */
function activatesGlobalLocalFontWeights(): void {
  const globals = readProjectFile('app/globals.css');
  const rootLayout = readProjectFile('app/layout.tsx');
  const routeTests = readProjectFile('tests/route-boundaries.test.tsx');
  const readme = readProjectFile('README.md');
  const designSystem = readFileSync(
    path.resolve(PROJECT_ROOT, '../docs/reference/design-system.md'),
    'utf8',
  );
  const activeFontSources = [globals, rootLayout, routeTests, readme, designSystem].join('\n');

  expect(globals).toContain("@import '../assets/JetBrainsLxgwNerdMono-Regular/result.css';");
  expect(globals).toContain("@import '../assets/JetBrainsLxgwNerdMono-Bold/result.css';");
  expect(globals).toContain(`--font-sans: '${FONT_FAMILY}', monospace;`);
  expect(globals).toContain(`--font-mono: '${FONT_FAMILY}', monospace;`);
  expect(globals.match(new RegExp(`font-family: '${FONT_FAMILY}', monospace`, 'g'))).toHaveLength(
    2,
  );
  expect(rootLayout).toContain('<body className="antialiased">');
  expect(activeFontSources).not.toMatch(
    /Maple Mono|MapleMono|font-geist|next\/font\/google|Geist_Mono/,
  );
  expect(existsSync(path.resolve(PROJECT_ROOT, 'assets/MapleMonoNormalNL-CN-Regular'))).toBe(false);
  expect(readme).toContain('JetBrainsLxgwNerdMono-Bold');
  expect(designSystem).toContain('JetBrainsLxgwNerdMono-Bold');
}

describe('global font assets', () => {
  test(
    'matches every generated CSS reference to its extracted WOFF2 file',
    matchesGeneratedFontAssetSets,
  );
  test('activates local regular and bold weights globally', activatesGlobalLocalFontWeights);
});
