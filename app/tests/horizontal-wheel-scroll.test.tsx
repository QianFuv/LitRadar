/**
 * Vertical wheel input glides horizontal strips only while they can move that way.
 */

import { afterEach, beforeEach, describe, expect, test, vi } from 'vitest';

import { attachHorizontalWheelScroll, scrollStripOnWheel } from '@/lib/horizontal-wheel-scroll';

let pendingFrames: FrameRequestCallback[] = [];

/** Run queued animation frames until every glide has settled. */
function flushFrames(): void {
  for (let guard = 0; pendingFrames.length > 0 && guard < 500; guard += 1) {
    const frames = pendingFrames;
    pendingFrames = [];
    for (const frame of frames) frame(performance.now());
  }
}

/**
 * Build a strip with fixed scroll geometry.
 *
 * @param scrollLeft - Current horizontal offset.
 * @param scrollWidth - Full content width.
 * @returns Strip element.
 */
function strip(scrollLeft: number, scrollWidth = 600): HTMLElement {
  const element = document.createElement('div');
  Object.defineProperty(element, 'scrollWidth', { configurable: true, value: scrollWidth });
  Object.defineProperty(element, 'clientWidth', { configurable: true, value: 200 });
  element.scrollLeft = scrollLeft;
  return element;
}

/**
 * Build a cancelable wheel event.
 *
 * @param init - Wheel deltas and modifiers.
 * @returns Wheel event.
 */
function wheel(init: WheelEventInit): WheelEvent {
  return new WheelEvent('wheel', { cancelable: true, ...init });
}

/** Glide rightwards on a downward wheel and keep the page from scrolling. */
function glidesRightOnWheelDown(): void {
  const element = strip(0);
  const event = wheel({ deltaY: 120 });
  expect(scrollStripOnWheel(element, event)).toBe(true);
  expect(event.defaultPrevented).toBe(true);
  expect(element.scrollLeft).toBeGreaterThan(0);
  expect(element.scrollLeft).toBeLessThan(120);
  flushFrames();
  expect(element.scrollLeft).toBe(120);
}

/** Add quick consecutive ticks onto the same glide instead of restarting from mid-flight. */
function accumulatesQuickTicks(): void {
  const element = strip(0);
  scrollStripOnWheel(element, wheel({ deltaY: 120 }));
  scrollStripOnWheel(element, wheel({ deltaY: 120 }));
  flushFrames();
  expect(element.scrollLeft).toBe(240);
}

/** Clamp at the far edge, then hand the wheel back to the page once the edge is the target. */
function releasesAtTheEdge(): void {
  const element = strip(350);
  expect(scrollStripOnWheel(element, wheel({ deltaY: 120 }))).toBe(true);
  flushFrames();
  expect(element.scrollLeft).toBe(400);
  const atEdge = wheel({ deltaY: 120 });
  expect(scrollStripOnWheel(element, atEdge)).toBe(false);
  expect(atEdge.defaultPrevented).toBe(false);
}

/** Leave horizontal gestures, pinch zoom, and strips without overflow to the browser. */
function ignoresOtherInput(): void {
  const horizontal = wheel({ deltaX: 80, deltaY: 10 });
  expect(scrollStripOnWheel(strip(0), horizontal)).toBe(false);
  expect(horizontal.defaultPrevented).toBe(false);
  expect(scrollStripOnWheel(strip(0), wheel({ deltaY: 120, ctrlKey: true }))).toBe(false);
  expect(scrollStripOnWheel(strip(0, 200), wheel({ deltaY: 120 }))).toBe(false);
}

/** Convert line-based wheel deltas into pixels. */
function convertsLineDeltas(): void {
  const element = strip(0);
  scrollStripOnWheel(element, wheel({ deltaY: 3, deltaMode: WheelEvent.DOM_DELTA_LINE }));
  flushFrames();
  expect(element.scrollLeft).toBe(48);
}

/** Attach one listener per strip even when the ref runs on every render. */
function attachesOnce(): void {
  const element = strip(0);
  attachHorizontalWheelScroll(element);
  attachHorizontalWheelScroll(element);
  element.dispatchEvent(wheel({ deltaY: 120 }));
  flushFrames();
  expect(element.scrollLeft).toBe(120);
}

beforeEach(() => {
  pendingFrames = [];
  vi.stubGlobal(
    'requestAnimationFrame',
    vi.fn((callback: FrameRequestCallback) => pendingFrames.push(callback)),
  );
});

afterEach(() => {
  vi.unstubAllGlobals();
});

describe('horizontal wheel scrolling', () => {
  test('glides right on a downward wheel', glidesRightOnWheelDown);
  test('accumulates quick wheel ticks', accumulatesQuickTicks);
  test('releases the wheel at the edge', releasesAtTheEdge);
  test('ignores horizontal gestures, zoom, and strips without overflow', ignoresOtherInput);
  test('converts line deltas to pixels', convertsLineDeltas);
  test('attaches one listener per strip', attachesOnce);
});
