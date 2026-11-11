/**
 * Real Chromium coverage for the client-only theme contract: next-themes reports the stored and
 * system themes on the very first render, so theme controls need no mounted-state guard.
 */

import { render } from '@testing-library/react';
import { ThemeProvider, useTheme } from 'next-themes';
import { afterEach, describe, expect, test } from 'vite-plus/test';

/** Theme values observed on each render of the probe. */
let observedThemes: { theme?: string; resolvedTheme?: string }[] = [];

/** Record the theme next-themes reports on every render. */
function ThemeProbe() {
  const { theme, resolvedTheme } = useTheme();
  observedThemes.push({ theme, resolvedTheme });
  return null;
}

/**
 * Render the probe under the application's theme provider settings.
 */
function renderThemeProbe(): void {
  render(
    <ThemeProvider attribute="class" defaultTheme="system" enableSystem>
      <ThemeProbe />
    </ThemeProvider>,
  );
}

/**
 * Verify an explicitly stored theme is reported on the first render.
 */
function reportsStoredThemeFirst(): void {
  window.localStorage.setItem('theme', 'dark');
  renderThemeProbe();

  expect(observedThemes[0]).toEqual({ theme: 'dark', resolvedTheme: 'dark' });
}

/**
 * Verify the default system theme resolves from the color-scheme query on the first render.
 */
function resolvesSystemThemeFirst(): void {
  window.localStorage.removeItem('theme');
  const systemTheme = window.matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'light';
  renderThemeProbe();

  expect(observedThemes[0]).toEqual({ theme: 'system', resolvedTheme: systemTheme });
}

afterEach(() => {
  observedThemes = [];
  window.localStorage.removeItem('theme');
});

describe('next-themes first render in Chromium', () => {
  test('reports the stored theme on the first render', reportsStoredThemeFirst);
  test('resolves the system theme on the first render', resolvesSystemThemeFirst);
});
