/**
 * Default UI chrome color-contract coverage.
 */

import { existsSync, readFileSync } from 'node:fs';
import path from 'node:path';
import { describe, expect, test } from 'vitest';

const PROJECT_ROOT = process.cwd();

const DEFAULT_CHROME_TOKENS = [
  '--background',
  '--foreground',
  '--card',
  '--card-foreground',
  '--popover',
  '--popover-foreground',
  '--primary-foreground',
  '--secondary',
  '--secondary-foreground',
  '--muted',
  '--muted-foreground',
  '--accent',
  '--accent-foreground',
  '--border',
  '--input',
  '--sidebar',
  '--sidebar-foreground',
  '--sidebar-primary-foreground',
  '--sidebar-accent',
  '--sidebar-accent-foreground',
  '--sidebar-border',
  '--scrollbar-thumb',
  '--scrollbar-thumb-hover',
] as const;

const DEFAULT_CHROME_COMPONENTS = [
  'components/feature/workspace-shell.tsx',
  'components/feature/search-workspace-view.tsx',
  'components/feature/sidebar.tsx',
  'components/feature/sidebar-navigation.tsx',
  'components/feature/sectioned-dialog.tsx',
  'components/feature/user-menu.tsx',
  'components/admin/admin-center-dialog.tsx',
  'components/settings/settings-center-dialog.tsx',
] as const;

const CHROMATIC_UTILITY_PATTERN =
  /(?:bg|text|border|ring|outline|fill|stroke)-(?:red|orange|amber|yellow|lime|green|emerald|teal|cyan|sky|blue|indigo|violet|purple|fuchsia|pink|rose)(?:-\d+)?/g;

const SEMANTIC_COLOR_FIXTURES = [
  {
    path: 'components/feature/results-list.tsx',
    utilities: ['text-info-foreground', 'bg-info'],
  },
  {
    path: 'components/ui/state-message.tsx',
    utilities: ['text-destructive', 'text-success-foreground', 'text-warning-foreground'],
  },
  {
    path: 'components/feature/favorite-button.tsx',
    utilities: ['text-warning-foreground', 'fill-current'],
  },
  {
    path: 'components/admin/scheduled-tasks-card.tsx',
    utilities: ['border-warning-border', 'text-warning-foreground'],
  },
  {
    path: 'components/tracking/tracking-settings-content.tsx',
    utilities: ['text-success-foreground'],
  },
] as const;

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
 * Extract the declarations inside one top-level CSS selector.
 *
 * @param stylesheet - Complete CSS source.
 * @param selector - Selector whose declaration block is required.
 * @returns Declaration block without braces.
 */
function extractDeclarationBlock(stylesheet: string, selector: ':root' | '.dark'): string {
  const selectorIndex = stylesheet.indexOf(`${selector} {`);
  if (selectorIndex < 0) {
    throw new Error(`Missing ${selector} declaration block`);
  }

  const openingBraceIndex = stylesheet.indexOf('{', selectorIndex);
  let depth = 0;
  for (let index = openingBraceIndex; index < stylesheet.length; index += 1) {
    if (stylesheet[index] === '{') {
      depth += 1;
    } else if (stylesheet[index] === '}') {
      depth -= 1;
      if (depth === 0) {
        return stylesheet.slice(openingBraceIndex + 1, index);
      }
    }
  }

  throw new Error(`Unclosed ${selector} declaration block`);
}

/**
 * Parse custom-property declarations from one CSS block.
 *
 * @param declarationBlock - CSS declarations without outer braces.
 * @returns Custom properties keyed by their complete names.
 */
function parseCustomProperties(declarationBlock: string): ReadonlyMap<string, string> {
  const variables = new Map<string, string>();
  const declarationPattern = /^\s*(--[a-z0-9-]+):\s*([^;]+);/gim;

  for (const match of declarationBlock.matchAll(declarationPattern)) {
    variables.set(match[1], match[2].trim());
  }

  return variables;
}

/**
 * Assert that a six-digit hex color has equal red, green, and blue channels.
 *
 * @param value - CSS color value.
 * @param context - Token and theme description for failures.
 */
function expectGrayscaleHex(value: string, context: string): void {
  const match = /^#([0-9a-f]{6})$/i.exec(value);
  if (!match) {
    throw new Error(`${context} must use a six-digit hex color, received ${value}`);
  }

  const channels = [
    Number.parseInt(match[1].slice(0, 2), 16),
    Number.parseInt(match[1].slice(2, 4), 16),
    Number.parseInt(match[1].slice(4, 6), 16),
  ];
  expect(new Set(channels).size).toBe(1);
}

/**
 * Verify structural surfaces and text stay neutral while interaction colors use Indigo.
 */
function keepsDefaultChromeTokensGrayscale(): void {
  const stylesheet = readProjectFile('app/globals.css');

  for (const selector of [':root', '.dark'] as const) {
    const variables = parseCustomProperties(extractDeclarationBlock(stylesheet, selector));
    for (const token of DEFAULT_CHROME_TOKENS) {
      const value = variables.get(token);
      expect(value).toBeDefined();
      expectGrayscaleHex(value ?? '', `${selector} ${token}`);
    }
  }

  const darkVariables = parseCustomProperties(extractDeclarationBlock(stylesheet, '.dark'));
  expect(darkVariables.get('--sidebar-primary')).toBe('#3e63dd');
  expect(darkVariables.get('--sidebar-primary-foreground')).toBe('#ffffff');
}

/**
 * Verify shell, navigation, account, and settings chrome contain no palette hue utilities.
 */
function keepsChromeComponentUtilitiesNeutral(): void {
  for (const relativePath of DEFAULT_CHROME_COMPONENTS) {
    const source = readProjectFile(relativePath);
    expect(source.match(CHROMATIC_UTILITY_PATTERN) ?? []).toEqual([]);
  }
}

/** Verify settings delegates responsive dialog chrome to the shared frame. */
function sharesSectionedDialogChrome(): void {
  const frameSource = readProjectFile('components/feature/sectioned-dialog.tsx');
  const adminSource = readProjectFile('components/admin/admin-center-dialog.tsx');
  const settingsSource = readProjectFile('components/settings/settings-center-dialog.tsx');

  expect(frameSource).toContain('md:w-[min(calc(100vw-2rem),72rem)]');
  expect(frameSource).toContain('hidden w-60');
  expect(frameSource).toContain('overflow-x-auto');
  expect(settingsSource).toContain('<SectionedDialogFrame');
  expect(settingsSource).not.toContain('<DialogContent');
  expect(settingsSource).not.toContain('SettingsCenterNavigation');
  expect(adminSource).toContain('<SectionedDialogFrame');
  expect(adminSource).not.toContain('<DialogContent');
}

/**
 * Verify semantic status colors and raster identity assets remain explicit exceptions.
 */
function retainsSemanticColorsAndRasterAssets(): void {
  const stylesheet = readProjectFile('app/globals.css');
  const lightVariables = parseCustomProperties(extractDeclarationBlock(stylesheet, ':root'));
  const darkVariables = parseCustomProperties(extractDeclarationBlock(stylesheet, '.dark'));

  expect({
    destructive: lightVariables.get('--destructive'),
    info: lightVariables.get('--info'),
    infoForeground: lightVariables.get('--info-foreground'),
  }).toEqual({ destructive: '#ce2c31', info: '#edf2fe', infoForeground: '#3a5bc7' });
  expect({
    destructive: darkVariables.get('--destructive'),
    info: darkVariables.get('--info'),
    infoForeground: darkVariables.get('--info-foreground'),
  }).toEqual({ destructive: '#ff9592', info: '#182449', infoForeground: '#9eb1ff' });

  for (const fixture of SEMANTIC_COLOR_FIXTURES) {
    const source = readProjectFile(fixture.path);
    for (const utility of fixture.utilities) {
      expect(source).toContain(utility);
    }
  }

  expect(existsSync(path.resolve(PROJECT_ROOT, 'public/litradar-logo.png'))).toBe(true);
  expect(readProjectFile('components/feature/sidebar.tsx')).toContain('/litradar-logo.png');
  expect(readProjectFile('components/feature/user-menu.tsx')).toContain('/litradar-logo.png');
}

/** Calculate relative luminance for a six-digit sRGB token. */
function colorLuminance(value: string): number {
  const channels = [1, 3, 5].map((offset) => {
    const channel = Number.parseInt(value.slice(offset, offset + 2), 16) / 255;
    return channel <= 0.04045 ? channel / 12.92 : ((channel + 0.055) / 1.055) ** 2.4;
  });
  return channels[0] * 0.2126 + channels[1] * 0.7152 + channels[2] * 0.0722;
}

/** Keep small text legible on both neutral and semantic surfaces. */
function keepsThemeTextLegible(): void {
  const stylesheet = readProjectFile('app/globals.css');
  const pairs = [
    ['--foreground', '--background'],
    ['--muted-foreground', '--card'],
    ['--primary-foreground', '--primary'],
    ['--primary-foreground', '--primary-hover'],
    ['--primary-foreground', '--primary-pressed'],
    ['--primary-text', '--background'],
    ['--destructive', '--card'],
    ['--destructive-foreground', '--destructive'],
    ['--destructive-foreground', '--destructive-hover'],
    ['--info-foreground', '--info'],
    ['--success-foreground', '--success'],
    ['--warning-foreground', '--warning'],
  ];
  for (const selector of [':root', '.dark'] as const) {
    const variables = parseCustomProperties(extractDeclarationBlock(stylesheet, selector));
    for (const [foreground, background] of pairs) {
      const luminances = [foreground, background].map((token) => {
        const value = variables.get(token);
        expect(value, `${selector} ${token}`).toMatch(/^#[0-9a-f]{6}$/i);
        return colorLuminance(value!);
      });
      const contrast = (Math.max(...luminances) + 0.05) / (Math.min(...luminances) + 0.05);
      expect(contrast, `${selector} ${foreground} on ${background}`).toBeGreaterThanOrEqual(4.5);
    }
  }
}

describe('default chrome theme contract', () => {
  test(
    'keeps neutral, interactive, and status text readable in both themes',
    keepsThemeTextLegible,
  );
  test('keeps ordinary light and dark tokens grayscale', keepsDefaultChromeTokensGrayscale);
  test(
    'keeps shell component utilities free of palette hues',
    keepsChromeComponentUtilitiesNeutral,
  );
  test('shares responsive sectioned dialog chrome', sharesSectionedDialogChrome);
  test(
    'retains semantic status colors and raster identity assets',
    retainsSemanticColorsAndRasterAssets,
  );
});
