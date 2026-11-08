/**
 * Reveal a theme change as a circle growing from the control that asked for it.
 */

import { flushSync } from 'react-dom';

/** View transition type that scopes the reveal styles in `globals.css`. */
const THEME_REVEAL_TYPE = 'litradar-theme-reveal';

/** Duration of the circular reveal, in milliseconds. */
const THEME_REVEAL_MS = 560;

/** Easing of the circular reveal: quick start, gentle landing. */
const THEME_REVEAL_EASING = 'cubic-bezier(0.22, 1, 0.36, 1)';

/** A point in viewport coordinates. */
export type ScreenPoint = { x: number; y: number };

/**
 * Read the center of an element in viewport coordinates.
 *
 * @param element - Element to measure.
 * @returns Center point of the element.
 */
function getElementCenter(element: Element): ScreenPoint {
  const rect = element.getBoundingClientRect();
  return { x: rect.left + rect.width / 2, y: rect.top + rect.height / 2 };
}

/**
 * Read where a keyboard or menu selection happened: the center of the focused element, or the
 * viewport center when nothing is focused.
 *
 * @returns Viewport point to reveal a theme change from.
 */
export function getFocusedOrigin(): ScreenPoint {
  const focused = document.activeElement;
  return focused && focused !== document.body
    ? getElementCenter(focused)
    : { x: window.innerWidth / 2, y: window.innerHeight / 2 };
}

/**
 * Start a view transition, tagging it with the reveal type where the browser supports types.
 *
 * @param update - DOM update to capture.
 * @returns Started view transition.
 */
function startRevealTransition(update: () => void): ViewTransition {
  try {
    return document.startViewTransition({ update, types: [THEME_REVEAL_TYPE] });
  } catch {
    return document.startViewTransition(update);
  }
}

/**
 * Apply a theme change behind a circular reveal that grows from `origin` until it covers the
 * viewport. Browsers without view transitions apply the change at once.
 *
 * @param applyTheme - Synchronous theme update, such as next-themes' `setTheme`.
 * @param origin - Viewport point the circle grows from.
 */
export function revealThemeChange(applyTheme: () => void, origin: ScreenPoint): void {
  if (typeof document.startViewTransition !== 'function') {
    applyTheme();
    return;
  }
  const transition = startRevealTransition(() => flushSync(applyTheme));
  const radius = Math.hypot(
    Math.max(origin.x, window.innerWidth - origin.x),
    Math.max(origin.y, window.innerHeight - origin.y),
  );
  const center = `${origin.x}px ${origin.y}px`;
  transition.ready
    .then(() => {
      document.documentElement.animate(
        { clipPath: [`circle(0px at ${center})`, `circle(${radius}px at ${center})`] },
        {
          duration: THEME_REVEAL_MS,
          easing: THEME_REVEAL_EASING,
          pseudoElement: '::view-transition-new(root)',
        },
      );
    })
    .catch(() => undefined);
}
