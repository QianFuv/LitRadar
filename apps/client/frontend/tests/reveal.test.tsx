/**
 * Reading-order reveal contract: placeholders first, content strictly in document order.
 */

import { act, render, screen, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, test, vi } from 'vite-plus/test';

import { REVEAL_WAIT_LIMIT_MS, RevealItem, RevealScope } from '@/components/ui/reveal';

const BLOCK_NAMES = ['Header', 'Toolbar', 'List'] as const;

/**
 * Render three blocks of one surface with the given readiness.
 *
 * @param props - Readiness per block, top to bottom.
 * @returns Surface with three reveal blocks.
 */
function Surface({ readiness }: { readiness: readonly boolean[] }) {
  return (
    <RevealScope>
      {BLOCK_NAMES.map((name, index) => (
        <RevealItem key={name} isReady={readiness[index]} placeholder={<p>{name} loading</p>}>
          <p>{name} content</p>
        </RevealItem>
      ))}
    </RevealScope>
  );
}

/** Keep later blocks on their placeholders until every earlier block has content. */
async function revealsInDocumentOrder(): Promise<void> {
  const view = render(<Surface readiness={[false, true, true]} />);
  expect(screen.getByText('Header loading')).toBeInTheDocument();
  expect(screen.getByText('Toolbar loading')).toBeInTheDocument();
  expect(screen.getByText('List loading')).toBeInTheDocument();
  expect(screen.queryByText('List content')).toBeNull();

  view.rerender(<Surface readiness={[true, true, true]} />);
  await waitFor(() => expect(screen.getByText('List content')).toBeInTheDocument());
  expect(screen.getByText('Header content')).toBeInTheDocument();
  expect(screen.getByText('Toolbar content')).toBeInTheDocument();
}

/** Show content that is already available when the surface mounts without any placeholder. */
function showsAvailableContentImmediately(): void {
  render(<Surface readiness={[true, true, true]} />);
  expect(screen.getByText('Header content')).toBeInTheDocument();
  expect(screen.getByText('List content')).toBeInTheDocument();
  expect(screen.queryByText('Header loading')).toBeNull();
}

/** Release later ready blocks once an earlier block has waited past the limit. */
async function releasesAfterWaitLimit(): Promise<void> {
  vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout', 'performance'] });
  render(<Surface readiness={[false, true, true]} />);
  expect(screen.getByText('Toolbar loading')).toBeInTheDocument();

  act(() => {
    vi.advanceTimersByTime(REVEAL_WAIT_LIMIT_MS);
  });
  vi.useRealTimers();
  await waitFor(() => expect(screen.getByText('List content')).toBeInTheDocument());
  expect(screen.getByText('Toolbar content')).toBeInTheDocument();
  expect(screen.getByText('Header loading')).toBeInTheDocument();
}

/** Return a reloading block to its placeholder without hiding content below it. */
async function reloadsWithoutHidingLaterBlocks(): Promise<void> {
  const view = render(<Surface readiness={[true, true, true]} />);
  view.rerender(<Surface readiness={[false, true, true]} />);
  await waitFor(() => expect(screen.getByText('Header loading')).toBeInTheDocument());
  expect(screen.getByText('Toolbar content')).toBeInTheDocument();
  expect(screen.getByText('List content')).toBeInTheDocument();
}

/** Order each surface on its own, and reveal blocks outside any surface by readiness alone. */
function ordersSurfacesIndependently(): void {
  render(
    <>
      <RevealScope>
        <RevealItem isReady={false} placeholder={<p>Page loading</p>}>
          <p>Page content</p>
        </RevealItem>
      </RevealScope>
      <RevealScope>
        <RevealItem isReady placeholder={<p>Menu loading</p>}>
          <p>Menu content</p>
        </RevealItem>
      </RevealScope>
      <RevealItem isReady placeholder={<p>Loose loading</p>}>
        <p>Loose content</p>
      </RevealItem>
    </>,
  );
  expect(screen.getByText('Page loading')).toBeInTheDocument();
  expect(screen.getByText('Menu content')).toBeInTheDocument();
  expect(screen.getByText('Loose content')).toBeInTheDocument();
}

afterEach(() => {
  vi.useRealTimers();
});

describe('reading-order reveal', () => {
  test('reveals ready blocks strictly in document order', revealsInDocumentOrder);
  test('shows available content immediately', showsAvailableContentImmediately);
  test('releases later blocks after the wait limit', releasesAfterWaitLimit);
  test('reloads a block without hiding later blocks', reloadsWithoutHidingLaterBlocks);
  test('orders surfaces independently', ordersSurfacesIndependently);
});
