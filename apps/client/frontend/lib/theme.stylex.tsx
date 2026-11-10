/**
 * Design tokens shared by every StyleX style.
 *
 * Each value is a `light-dark()` pair, so the active theme follows the root `color-scheme` that
 * next-themes and `public/theme-init.js` set; no theme class has to be applied to elements.
 *
 * @packageDocumentation
 */

import * as stylex from '@stylexjs/stylex';

/** Surface, text, state, and navigation colors for the light and dark themes. */
export const colors = stylex.defineVars({
  background: 'light-dark(#f6f6f6, #141414)',
  foreground: 'light-dark(#27272a, #f5f5f5)',
  card: 'light-dark(#fbfbfb, #1b1b1b)',
  cardForeground: 'light-dark(#27272a, #f5f5f5)',
  popover: 'light-dark(#fbfbfb, #1b1b1b)',
  popoverForeground: 'light-dark(#27272a, #f5f5f5)',
  canvas: 'light-dark(#f6f6f6, #141414)',
  paper: 'light-dark(#f8f8f8, #1b1b1b)',
  primary: 'light-dark(#1b4ed8, #346bf1)',
  primaryHover: 'light-dark(#3160db, #3061d9)',
  primaryPressed: 'light-dark(#1740b0, #2a55c0)',
  primaryText: 'light-dark(#1b4ed8, #51a2ff)',
  primaryForeground: '#ffffff',
  secondary: 'light-dark(#f3f3f3, #1b1b1b)',
  secondaryForeground: 'light-dark(#27272a, #f5f5f5)',
  muted: 'light-dark(#f3f3f3, #1b1b1b)',
  mutedForeground: 'light-dark(#696973, #8e8e8e)',
  accent: 'light-dark(#ededee, #1e1e1e)',
  accentPressed: 'light-dark(#e4e4e6, #262626)',
  accentForeground: 'light-dark(#18181b, #f5f5f5)',
  destructive: 'light-dark(#c10007, #fb414a)',
  destructiveHover: 'light-dark(#a10006, #ff6467)',
  destructiveForeground: 'light-dark(#ffffff, #141414)',
  destructiveRing:
    'light-dark(color-mix(in oklab, #c10007 20%, transparent), color-mix(in oklab, #fb414a 40%, transparent))',
  info: 'light-dark(#dbe1f2, #1b2440)',
  infoForeground: 'light-dark(#1b4ed8, #51a2ff)',
  success: 'light-dark(#ecfdf3, #0f2a1c)',
  successForeground: 'light-dark(#166534, #6ee7a0)',
  successBorder: 'light-dark(#b7e4c7, #1f5c3a)',
  warning: 'light-dark(#f7efe2, #3b2a11)',
  warningForeground: 'light-dark(#b54b00, #ffb900)',
  warningBorder: 'light-dark(#facd88, #7f510b)',
  border: 'light-dark(#e0e0e3, #242424)',
  input: 'light-dark(#d0d0d4, #2a2a2a)',
  inputHover: 'light-dark(#b8b8bd, #3a3a3a)',
  ring: 'light-dark(#1b4ed8, #346bf1)',
  rail: 'light-dark(#efefef, #1c1c1c)',
  railForeground: 'light-dark(#676771, #a3a3a3)',
  railActive: 'light-dark(#27272a, #f1f3f7)',
  railHover: 'light-dark(#e9e9ea, #2c2c2c)',
  railBorder: 'light-dark(#e0e0e3, #2b2b2b)',
});

/**
 * Elevation shadows. Layers that exist in only one theme are kept in both with a transparent
 * color, so a single `light-dark()` per layer renders the original light and dark shadows.
 */
export const shadows = stylex.defineVars({
  vercelRing: 'light-dark(#d0d0d4, #2a2a2a) 0px 0px 0px 1px',
  vercelCard:
    'light-dark(#e0e0e3, #242424) 0px 0px 0px 1px, light-dark(rgba(0, 0, 0, 0.03), rgba(0, 0, 0, 0.3)) 0px 1px 2px',
  paper:
    '0 0 0 1px light-dark(rgba(0, 0, 0, 0.05), rgba(255, 255, 255, 0.06)), 0 1px 2px light-dark(rgba(0, 0, 0, 0.04), transparent), 0 8px 24px -12px light-dark(rgba(0, 0, 0, 0.12), rgba(0, 0, 0, 0.6))',
  overlay:
    '0 0 0 1px light-dark(rgba(0, 0, 0, 0.06), rgba(255, 255, 255, 0.08)), 0 12px 32px -8px light-dark(rgba(0, 0, 0, 0.18), transparent), 0 16px 40px -8px light-dark(transparent, rgba(0, 0, 0, 0.7))',
});

/**
 * Unitless line heights of the type scale, kept as exact ratios: written inline, the CSS build
 * rounds them to five decimals and shifts text by a fraction of a pixel.
 */
export const lineHeights = stylex.defineVars({
  textXs: 'calc(1 / 0.75)',
  textSm: 'calc(1.25 / 0.875)',
  textLg: 'calc(1.75 / 1.125)',
  text2xl: 'calc(2 / 1.5)',
});
