/**
 * Default UI chrome color-contract coverage.
 */

import { existsSync, readFileSync } from 'node:fs';
import path from 'node:path';
import { describe, expect, test } from 'vite-plus/test';

const PROJECT_ROOT = process.cwd();

const DEFAULT_CHROME_TOKENS = [
  'background',
  'foreground',
  'card',
  'cardForeground',
  'popover',
  'popoverForeground',
  'primaryForeground',
  'secondary',
  'secondaryForeground',
  'muted',
  'mutedForeground',
  'accent',
  'accentForeground',
  'border',
  'input',
  'rail',
  'railForeground',
  'railActive',
  'railHover',
  'railBorder',
] as const;

const DEFAULT_CHROME_COMPONENTS = [
  'components/feature/workspace-shell.tsx',
  'components/feature/search-workspace-view.tsx',
  'components/feature/search-filters.tsx',
  'components/feature/filter-pill.tsx',
  'components/shell/app-frame.tsx',
  'components/shell/primary-navigation.tsx',
  'components/feature/sectioned-dialog.tsx',
  'components/feature/user-menu.tsx',
  'components/admin/admin-center-dialog.tsx',
  'components/settings/settings-center-dialog.tsx',
] as const;

const MAX_NEUTRAL_CHANNEL_SPREAD = 12;

const LITERAL_COLOR_PATTERN = /#[0-9a-f]{3,8}\b|\b(?:rgba?|hsla?|oklch|oklab)\(/gi;

const LIGHT_DARK_HEX_PATTERN = /light-dark\((#[0-9a-f]{6}), (#[0-9a-f]{6})\)/gi;

const SEMANTIC_COLOR_FIXTURES = [
  {
    path: 'lib/fts-highlight.tsx',
    tokens: ['colors.infoForeground', 'colors.info'],
  },
  {
    path: 'components/ui/state-message.tsx',
    tokens: ['colors.destructive', 'colors.successForeground', 'colors.warningForeground'],
  },
  {
    path: 'components/feature/favorite-button.tsx',
    tokens: ['colors.warningForeground', "fill: 'currentcolor'"],
  },
  {
    path: 'components/admin/scheduled-tasks-card.tsx',
    tokens: ['colors.warningBorder', 'colors.warningForeground'],
  },
  {
    path: 'components/tracking/tracking-settings-content.tsx',
    tokens: ['colors.successForeground'],
  },
] as const;

type ColorScheme = 'light' | 'dark';

type SchemeColors = { light: string; dark: string };

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
 * Split a `light-dark()` token value into its two colors; other values apply to both schemes.
 *
 * @param value - Token value.
 * @returns Light and dark colors.
 */
function splitLightDark(value: string): SchemeColors {
  const match = /^light-dark\((#[0-9a-f]{6}), (#[0-9a-f]{6})\)$/i.exec(value);
  return match ? { light: match[1], dark: match[2] } : { light: value, dark: value };
}

/**
 * Parse the single-line color tokens of `stylex.defineVars` in lib/theme.stylex.tsx.
 *
 * @returns Light and dark colors keyed by token name.
 */
function readThemeColors(): ReadonlyMap<string, SchemeColors> {
  const source = readProjectFile('lib/theme.stylex.tsx');
  const colorsBlock = source.slice(
    source.indexOf('export const colors'),
    source.indexOf('export const shadows'),
  );
  return new Map(
    Array.from(colorsBlock.matchAll(/^\s+(\w+): '([^']+)',$/gm), (match) => [
      match[1],
      splitLightDark(match[2]),
    ]),
  );
}

/**
 * Read one theme color for one color scheme.
 *
 * @param colors - Parsed theme colors.
 * @param token - Token name.
 * @param scheme - Color scheme.
 * @returns Color value.
 */
function themeColor(
  colors: ReadonlyMap<string, SchemeColors>,
  token: string,
  scheme: ColorScheme,
): string {
  const value = colors.get(token);
  if (!value) {
    throw new Error(`Missing theme color ${token}`);
  }
  return value[scheme];
}

/**
 * Assert that a six-digit hex color is a near-neutral gray whose channels differ by at most a slight tint.
 *
 * @param value - CSS color value.
 * @param context - Token and theme description for failures.
 */
function expectNeutralHex(value: string, context: string): void {
  const match = /^#([0-9a-f]{6})$/i.exec(value);
  if (!match) {
    throw new Error(`${context} must use a six-digit hex color, received ${value}`);
  }

  const channels = [
    Number.parseInt(match[1].slice(0, 2), 16),
    Number.parseInt(match[1].slice(2, 4), 16),
    Number.parseInt(match[1].slice(4, 6), 16),
  ];
  expect(Math.max(...channels) - Math.min(...channels), context).toBeLessThanOrEqual(
    MAX_NEUTRAL_CHANNEL_SPREAD,
  );
}

/**
 * Verify structural surfaces, text, and scrollbars stay neutral while interaction colors use the
 * blue accent.
 */
function keepsDefaultChromeTokensNeutral(): void {
  const colors = readThemeColors();
  for (const scheme of ['light', 'dark'] as const) {
    for (const token of DEFAULT_CHROME_TOKENS) {
      expectNeutralHex(themeColor(colors, token, scheme), `${scheme} ${token}`);
    }
  }

  const scrollbarColors = readProjectFile('src/globals.css').matchAll(
    /scrollbar[^{]*\{[^}]*?light-dark\((#[0-9a-f]{6}), (#[0-9a-f]{6})\)/gi,
  );
  for (const [declaration, light, dark] of scrollbarColors) {
    expectNeutralHex(light, `light ${declaration}`);
    expectNeutralHex(dark, `dark ${declaration}`);
  }

  expect(themeColor(colors, 'primary', 'dark')).toBe('#346bf1');
  expect(themeColor(colors, 'primaryForeground', 'dark')).toBe('#ffffff');
}

/**
 * Verify shell, navigation, account, and settings chrome take every color from theme tokens.
 */
function keepsChromeComponentColorsOnTokens(): void {
  for (const relativePath of DEFAULT_CHROME_COMPONENTS) {
    const source = readProjectFile(relativePath);
    expect(source.match(LITERAL_COLOR_PATTERN) ?? []).toEqual([]);
  }
}

/**
 * Verify every document-level color pair in globals.css repeats a theme token, apart from the
 * scrollbar thumb colors that only the document stylesheet uses.
 */
function keepsGlobalColorsInSync(): void {
  const colors = readThemeColors();
  const tokenPairs = new Set(Array.from(colors.values(), ({ light, dark }) => `${light} ${dark}`));
  const globals = readProjectFile('src/globals.css');
  const scrollbarThumbPairs = new Set(
    Array.from(
      globals.matchAll(
        /scrollbar-thumb(?::hover)? \{\s*background: light-dark\((#\w{6}), (#\w{6})\)/g,
      ),
      ([, light, dark]) => `${light} ${dark}`,
    ),
  );
  for (const [, light, dark] of globals.matchAll(LIGHT_DARK_HEX_PATTERN)) {
    const pair = `${light} ${dark}`;
    expect(tokenPairs.has(pair) || scrollbarThumbPairs.has(pair), pair).toBe(true);
  }
}

/** Verify settings delegates responsive dialog chrome to the shared frame. */
function sharesSectionedDialogChrome(): void {
  const frameSource = readProjectFile('components/feature/sectioned-dialog.tsx');
  const adminSource = readProjectFile('components/admin/admin-center-dialog.tsx');
  const settingsSource = readProjectFile('components/settings/settings-center-dialog.tsx');

  expect(frameSource).toContain('placement="takeover"');
  expect(frameSource).toContain("aria-current={isActive ? 'page' : undefined}");
  expect(frameSource).toMatch(/overflowX: \{\s*default: 'auto',/);
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
  const colors = readThemeColors();

  expect({
    destructive: themeColor(colors, 'destructive', 'light'),
    info: themeColor(colors, 'info', 'light'),
    infoForeground: themeColor(colors, 'infoForeground', 'light'),
  }).toEqual({ destructive: '#c10007', info: '#dbe1f2', infoForeground: '#1b4ed8' });
  expect({
    destructive: themeColor(colors, 'destructive', 'dark'),
    info: themeColor(colors, 'info', 'dark'),
    infoForeground: themeColor(colors, 'infoForeground', 'dark'),
  }).toEqual({ destructive: '#fb414a', info: '#1b2440', infoForeground: '#51a2ff' });

  for (const fixture of SEMANTIC_COLOR_FIXTURES) {
    const source = readProjectFile(fixture.path);
    for (const token of fixture.tokens) {
      expect(source).toContain(token);
    }
  }

  expect(existsSync(path.resolve(PROJECT_ROOT, 'public/litradar-logo.png'))).toBe(true);
  expect(readProjectFile('components/shell/app-frame.tsx')).toContain('/litradar-logo.png');
  expect(readProjectFile('src/routes/login-client.tsx')).toContain('/litradar-logo.png');
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
  const colors = readThemeColors();
  const pairs = [
    ['foreground', 'background'],
    ['mutedForeground', 'card'],
    ['primaryForeground', 'primary'],
    ['primaryForeground', 'primaryHover'],
    ['primaryForeground', 'primaryPressed'],
    ['primaryText', 'background'],
    ['destructive', 'card'],
    ['destructiveForeground', 'destructive'],
    ['destructiveForeground', 'destructiveHover'],
    ['infoForeground', 'info'],
    ['successForeground', 'success'],
    ['warningForeground', 'warning'],
  ];
  for (const scheme of ['light', 'dark'] as const) {
    for (const [foreground, background] of pairs) {
      const luminances = [foreground, background].map((token) => {
        const value = themeColor(colors, token, scheme);
        expect(value, `${scheme} ${token}`).toMatch(/^#[0-9a-f]{6}$/i);
        return colorLuminance(value);
      });
      const contrast = (Math.max(...luminances) + 0.05) / (Math.min(...luminances) + 0.05);
      expect(contrast, `${scheme} ${foreground} on ${background}`).toBeGreaterThanOrEqual(4.5);
    }
  }
}

describe('default chrome theme contract', () => {
  test(
    'keeps neutral, interactive, and status text readable in both themes',
    keepsThemeTextLegible,
  );
  test('keeps ordinary light and dark tokens neutral', keepsDefaultChromeTokensNeutral);
  test('keeps shell component colors on theme tokens', keepsChromeComponentColorsOnTokens);
  test('keeps document-level colors in sync with theme tokens', keepsGlobalColorsInSync);
  test('shares responsive sectioned dialog chrome', sharesSectionedDialogChrome);
  test(
    'retains semantic status colors and raster identity assets',
    retainsSemanticColorsAndRasterAssets,
  );
});
