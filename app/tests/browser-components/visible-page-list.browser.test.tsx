/**
 * Real Chromium IntersectionObserver and scroll-reset coverage for visible page lists.
 */

import { act, render, screen, waitFor } from '@testing-library/react';
import { describe, expect, test, vi } from 'vitest';

import { useVisiblePageList } from '@/components/feature/use-visible-page-list';

type VisiblePageHarnessProps = {
  isFetchingNextPage?: boolean;
  listKey: string;
  loadedPages?: number;
  onFetchNextPage: () => void;
};

/**
 * Render separated native observer sentinels around a visible-page count probe.
 *
 * @param props - Current list identity and next-page callback.
 * @returns Scrollable observer harness.
 */
function VisiblePageHarness({
  isFetchingNextPage = false,
  listKey,
  loadedPages = 2,
  onFetchNextPage,
}: VisiblePageHarnessProps) {
  const { loadMoreRef, prefetchRef, visiblePages } = useVisiblePageList({
    listKey,
    loadedPages,
    hasNextPage: true,
    isFetchingNextPage,
    onFetchNextPage,
  });

  return (
    <main>
      <output data-testid="visible-pages">{visiblePages}</output>
      <div aria-hidden="true" style={{ height: '150vh' }} />
      <div ref={prefetchRef} data-testid="prefetch-sentinel" style={{ height: '20px' }} />
      <div aria-hidden="true" style={{ height: '150vh' }} />
      <div ref={loadMoreRef} data-testid="load-more-sentinel" style={{ height: '20px' }} />
    </main>
  );
}

/**
 * Wait for two real animation frames so Chromium can deliver intersection records.
 */
async function waitForIntersectionDelivery(): Promise<void> {
  await new Promise<void>((resolve) => window.requestAnimationFrame(() => resolve()));
  await new Promise<void>((resolve) => window.requestAnimationFrame(() => resolve()));
}

/**
 * Verify native intersections reveal loaded pages, request the next page, and reset scrolling.
 */
async function respondsToNativeIntersections(): Promise<void> {
  const onFetchNextPage = vi.fn();
  const { rerender } = render(
    <VisiblePageHarness listKey="first-list" onFetchNextPage={onFetchNextPage} />,
  );

  expect(screen.getByTestId('visible-pages')).toHaveTextContent('1');
  await act(async () => {
    screen.getByTestId('load-more-sentinel').scrollIntoView({ block: 'center' });
    await waitForIntersectionDelivery();
  });
  await waitFor(() => expect(screen.getByTestId('visible-pages')).toHaveTextContent('2'));

  await act(async () => {
    screen.getByTestId('prefetch-sentinel').scrollIntoView({ block: 'center' });
    await waitForIntersectionDelivery();
  });
  await waitFor(() => expect(onFetchNextPage).toHaveBeenCalledTimes(1));
  expect(window.scrollY).toBeGreaterThan(0);

  rerender(<VisiblePageHarness listKey="second-list" onFetchNextPage={onFetchNextPage} />);
  await waitFor(() => expect(window.scrollY).toBe(0));
  expect(screen.getByTestId('visible-pages')).toHaveTextContent('1');
}

/** Verify a delayed page is revealed while its sentinel remains intersecting. */
async function revealsDelayedPageWithoutSecondIntersection(): Promise<void> {
  const onFetchNextPage = vi.fn();
  const { rerender } = render(
    <VisiblePageHarness listKey="delayed-list" loadedPages={1} onFetchNextPage={onFetchNextPage} />,
  );

  await act(async () => {
    screen.getByTestId('load-more-sentinel').scrollIntoView({ block: 'center' });
    await waitForIntersectionDelivery();
  });
  await waitFor(() => expect(onFetchNextPage).toHaveBeenCalledTimes(1));

  rerender(
    <VisiblePageHarness
      isFetchingNextPage
      listKey="delayed-list"
      loadedPages={1}
      onFetchNextPage={onFetchNextPage}
    />,
  );
  rerender(
    <VisiblePageHarness listKey="delayed-list" loadedPages={2} onFetchNextPage={onFetchNextPage} />,
  );

  await waitFor(() => expect(screen.getByTestId('visible-pages')).toHaveTextContent('2'));
  expect(onFetchNextPage).toHaveBeenCalledTimes(1);
}

describe('visible page list in Chromium', () => {
  test('responds to native intersections and list reset events', respondsToNativeIntersections);
  test(
    'reveals a delayed page without a second intersection',
    revealsDelayedPageWithoutSecondIntersection,
  );
});
