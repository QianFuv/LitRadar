/**
 * Real Chromium IntersectionObserver coverage for the visibility hook.
 */

import { render } from '@testing-library/react';
import { page } from 'vite-plus/test/browser';
import { describe, expect, test } from 'vite-plus/test';

import { useInView } from '@/lib/use-in-view';

/**
 * Render a tall page with one observed marker far below the fold.
 *
 * @param props - Whether the marker keeps its visibility once seen.
 * @returns Tall page with a labelled visibility readout.
 */
function VisibilityHarness({ isOnce }: { isOnce: boolean }) {
  const { ref, isInView } = useInView({ isOnce });
  return (
    <div>
      <output aria-label="可见状态">{isInView ? 'visible' : 'hidden'}</output>
      <div style={{ height: '300vh' }} />
      <div ref={ref} data-testid="marker" style={{ height: '1px' }} />
      <div style={{ height: '300vh' }} />
    </div>
  );
}

/**
 * Scroll the window so the marker is inside or outside the viewport.
 *
 * @param isMarkerVisible - Whether to bring the marker into view.
 */
function scrollMarker(isMarkerVisible: boolean): void {
  if (isMarkerVisible) {
    document.querySelector('[data-testid="marker"]')?.scrollIntoView();
  } else {
    window.scrollTo({ behavior: 'auto', top: 0 });
  }
}

/** Verify visibility follows scrolling in both directions. */
async function followsVisibility(): Promise<void> {
  render(<VisibilityHarness isOnce={false} />);
  const readout = page.getByLabelText('可见状态');

  await expect.element(readout).toHaveTextContent('hidden');
  scrollMarker(true);
  await expect.element(readout).toHaveTextContent('visible');
  scrollMarker(false);
  await expect.element(readout).toHaveTextContent('hidden');
}

/** Verify a one-shot observer keeps reporting visible after the element leaves the viewport. */
async function keepsFirstVisibility(): Promise<void> {
  render(<VisibilityHarness isOnce />);
  const readout = page.getByLabelText('可见状态');

  scrollMarker(true);
  await expect.element(readout).toHaveTextContent('visible');
  scrollMarker(false);
  await new Promise((resolve) => setTimeout(resolve, 300));
  await expect.element(readout).toHaveTextContent('visible');
}

describe('useInView in Chromium', () => {
  test('follows visibility while scrolling', followsVisibility);
  test('keeps the first visibility when observing once', keepsFirstVisibility);
});
