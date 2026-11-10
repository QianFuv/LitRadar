/**
 * Theme changes grow from where they were picked and still apply without view transitions.
 */

import { afterEach, describe, expect, test, vi } from 'vite-plus/test';

import { getFocusedOrigin, revealThemeChange } from '@/lib/theme-transition';

type StartViewTransition = (options: unknown) => { ready: Promise<void> };

/**
 * Install a view transition stub that runs its update right away.
 *
 * @returns Stubbed `document.startViewTransition`.
 */
function stubViewTransitions() {
  const start = vi.fn<StartViewTransition>((options) => {
    const update =
      typeof options === 'function' ? options : (options as { update: () => void }).update;
    update();
    return { ready: Promise.resolve() };
  });
  Object.defineProperty(document, 'startViewTransition', { configurable: true, value: start });
  return start;
}

afterEach(() => {
  Reflect.deleteProperty(document, 'startViewTransition');
  vi.restoreAllMocks();
});

/** Apply the theme at once when the browser has no view transitions. */
function appliesWithoutViewTransitions(): void {
  const applyTheme = vi.fn();
  revealThemeChange(applyTheme, { x: 10, y: 10 });
  expect(applyTheme).toHaveBeenCalledOnce();
}

/** Grow a circle from the origin until it covers the farthest viewport corner. */
async function growsCircleFromOrigin(): Promise<void> {
  const start = stubViewTransitions();
  const animate = vi.spyOn(document.documentElement, 'animate').mockReturnValue({} as Animation);
  const applyTheme = vi.fn();
  revealThemeChange(applyTheme, { x: 100, y: 50 });
  expect(applyTheme).toHaveBeenCalledOnce();
  expect(start).toHaveBeenCalledWith(expect.objectContaining({ types: ['litradar-theme-reveal'] }));
  await vi.waitFor(() => expect(animate).toHaveBeenCalledOnce());
  const radius = Math.hypot(window.innerWidth - 100, window.innerHeight - 50);
  expect(animate.mock.calls[0][0]).toEqual({
    clipPath: ['circle(0px at 100px 50px)', `circle(${radius}px at 100px 50px)`],
  });
  expect(animate.mock.calls[0][1]).toMatchObject({ pseudoElement: '::view-transition-new(root)' });
}

/** Start from the focused control, or from the viewport center when nothing is focused. */
function readsFocusedOrigin(): void {
  expect(getFocusedOrigin()).toEqual({ x: window.innerWidth / 2, y: window.innerHeight / 2 });
  const button = document.createElement('button');
  document.body.append(button);
  vi.spyOn(button, 'getBoundingClientRect').mockReturnValue({
    left: 20,
    top: 40,
    width: 60,
    height: 20,
  } as DOMRect);
  button.focus();
  expect(getFocusedOrigin()).toEqual({ x: 50, y: 50 });
  button.remove();
}

describe('theme transition', () => {
  test('applies without view transitions', appliesWithoutViewTransitions);
  test('grows a circle from the origin', growsCircleFromOrigin);
  test('reads the focused origin', readsFocusedOrigin);
});
