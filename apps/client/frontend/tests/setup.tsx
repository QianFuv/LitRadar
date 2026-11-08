/**
 * Global DOM assertions, cleanup, storage isolation, and MSW lifecycle hooks.
 */

import '@testing-library/jest-dom/vitest';
import { cleanup } from '@testing-library/react';
import { MotionGlobalConfig } from 'motion/react';
import { afterAll, afterEach, beforeAll } from 'vitest';

import { server } from '@/tests/mocks/server';

/**
 * Create a static media-query result that never matches, for unit tests.
 *
 * @param query - Browser media query string.
 * @returns Standards-shaped immutable media-query result.
 */
function createTestMediaQueryList(query: string): MediaQueryList {
  return {
    matches: false,
    media: query,
    onchange: null,
    addEventListener: () => undefined,
    addListener: () => undefined,
    dispatchEvent: () => false,
    removeEventListener: () => undefined,
    removeListener: () => undefined,
  };
}

/**
 * Install deterministic media-query behavior for jsdom.
 */
function installTestMediaQueries(): void {
  Object.defineProperty(window, 'matchMedia', {
    configurable: true,
    writable: true,
    value: createTestMediaQueryList,
  });
}

/**
 * Web Animation stand-in that completes at once, for code paths jsdom cannot animate.
 */
class FinishedTestAnimation extends EventTarget {
  currentTime = 0;
  effect = null;
  finished = Promise.resolve(this);
  id = '';
  oncancel = null;
  pending = false;
  playState = 'finished';
  playbackRate = 1;
  ready = Promise.resolve(this);
  replaceState = 'active';
  startTime = 0;
  timeline = null;
  private finishHandler: ((event: Event) => void) | null = null;

  get onfinish(): ((event: Event) => void) | null {
    return this.finishHandler;
  }

  set onfinish(handler: ((event: Event) => void) | null) {
    this.finishHandler = handler;
    queueMicrotask(() => handler?.(new Event('finish')));
  }

  cancel(): void {}
  commitStyles(): void {}
  finish(): void {}
  pause(): void {}
  persist(): void {}
  play(): void {}
  reverse(): void {}
  updatePlaybackRate(): void {}
}

/**
 * Observer stand-in for jsdom, which does not implement layout observation.
 */
class StaticTestResizeObserver {
  disconnect(): void {}
  observe(): void {}
  unobserve(): void {}
}

/**
 * Visibility observer stand-in for jsdom, which never reports elements as intersecting.
 */
class StaticTestIntersectionObserver {
  readonly root = null;
  readonly rootMargin = '0px';
  readonly thresholds = [0];
  disconnect(): void {}
  observe(): void {}
  takeRecords(): IntersectionObserverEntry[] {
    return [];
  }
  unobserve(): void {}
}

/**
 * Identity transform stand-in for jsdom, which lacks `DOMMatrix`; Animate UI's toast swipe reads
 * the current transform when a pointer presses a toast.
 */
class IdentityTestDOMMatrix {
  readonly a = 1;
  readonly b = 0;
  readonly c = 0;
  readonly d = 1;
  readonly e = 0;
  readonly f = 0;
  readonly m41 = 0;
  readonly m42 = 0;
}

/**
 * Install layout and animation APIs that jsdom lacks, and finish Motion animations at once.
 */
function installTestAnimationApis(): void {
  MotionGlobalConfig.skipAnimations = true;
  globalThis.ResizeObserver ??= StaticTestResizeObserver;
  globalThis.IntersectionObserver ??=
    StaticTestIntersectionObserver as unknown as typeof IntersectionObserver;
  globalThis.SVGGeometryElement ??= class extends SVGElement {} as typeof SVGGeometryElement;
  globalThis.DOMMatrix ??= IdentityTestDOMMatrix as unknown as typeof DOMMatrix;
  Element.prototype.getAnimations ??= () => [];
  Element.prototype.animate ??= function animate() {
    return new FinishedTestAnimation() as unknown as Animation;
  };
}

/**
 * Start request interception and fail tests on unhandled network calls.
 */
function startMockServer(): void {
  server.listen({ onUnhandledRequest: 'error' });
}

/**
 * Reset DOM, handlers, and browser storage after each test.
 */
function resetTestState(): void {
  cleanup();
  server.resetHandlers();
  window.localStorage.clear();
  window.sessionStorage.clear();
}

/**
 * Close request interception after the test process finishes.
 */
function closeMockServer(): void {
  server.close();
}

installTestMediaQueries();
installTestAnimationApis();
beforeAll(startMockServer);
afterEach(resetTestState);
afterAll(closeMockServer);
