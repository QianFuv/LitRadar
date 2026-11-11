/**
 * Transient feedback keeps the newest value for its full duration and never resets after unmount.
 */

import { act, renderHook } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, test, vi } from 'vite-plus/test';

import { useTransientState } from '@/lib/use-transient-state';

const DURATION_MS = 3000;

/**
 * Render the hook with a string value that idles at null.
 *
 * @returns Hook render result.
 */
function renderTransientMessage() {
  return renderHook(() => useTransientState<string | null>(null, DURATION_MS));
}

/**
 * Verify a quick second value is not cleared by the first value's countdown.
 */
function keepsNewestValueForFullDuration(): void {
  const { result } = renderTransientMessage();

  act(() => result.current[1]('first'));
  act(() => {
    vi.advanceTimersByTime(2000);
  });
  act(() => result.current[1]('second'));
  act(() => {
    vi.advanceTimersByTime(1500);
  });
  expect(result.current[0]).toBe('second');

  act(() => {
    vi.advanceTimersByTime(DURATION_MS - 1500);
  });
  expect(result.current[0]).toBeNull();
}

/**
 * Verify unmounting cancels the pending reset.
 */
function cancelsResetOnUnmount(): void {
  const { result, unmount } = renderTransientMessage();
  act(() => result.current[1]('copied'));
  unmount();

  expect(vi.getTimerCount()).toBe(0);
}

beforeEach(() => {
  vi.useFakeTimers();
});

afterEach(() => {
  vi.useRealTimers();
});

describe('useTransientState', () => {
  test('keeps the newest value for its full duration', keepsNewestValueForFullDuration);
  test('cancels the pending reset on unmount', cancelsResetOnUnmount);
});
