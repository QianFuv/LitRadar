/**
 * Hydration, storage, subscription, and reconciliation coverage for database selection.
 */

import { act } from 'react';
import { render, screen, waitFor } from '@testing-library/react';
import { hydrateRoot, type Root } from 'react-dom/client';
import { renderToString } from 'react-dom/server';
import { describe, expect, test, vi } from 'vitest';

import { DEFAULT_DATABASE, SELECTED_DATABASE_KEY } from '@/lib/api/client';
import {
  getSelectedDatabaseSnapshot,
  getSelectedDatabaseServerSnapshot,
  reconcileSelectedDatabase,
  setSelectedDatabase,
  subscribeSelectedDatabase,
  useSelectedDatabase,
} from '@/lib/selected-database';

const LEGACY_SELECTED_DATABASE_KEY = 'selected_database';

/**
 * Render the selected database snapshot for hydration and subscription assertions.
 *
 * @returns Current selected database text.
 */
function SelectedDatabaseProbe() {
  const selectedDatabase = useSelectedDatabase();
  return <output>{selectedDatabase}</output>;
}

/**
 * Verify server markup stays deterministic before the stored client value takes over.
 */
async function hydratesWithoutMismatch(): Promise<void> {
  window.localStorage.setItem(SELECTED_DATABASE_KEY, 'stored.sqlite');
  const serverMarkup = renderToString(<SelectedDatabaseProbe />);
  expect(serverMarkup).toContain(DEFAULT_DATABASE);
  expect(getSelectedDatabaseServerSnapshot()).toBe(DEFAULT_DATABASE);

  const container = document.createElement('div');
  container.innerHTML = serverMarkup;
  document.body.append(container);
  const consoleError = vi.spyOn(console, 'error').mockImplementation(() => undefined);
  let root: Root | null = null;

  try {
    await act(async () => {
      root = hydrateRoot(container, <SelectedDatabaseProbe />);
    });
    await waitFor(() => expect(container).toHaveTextContent('stored.sqlite'));
    const hydrationErrors = consoleError.mock.calls.filter((call) =>
      call.some((value) => /hydration|did not match|server rendered/iu.test(String(value))),
    );
    expect(hydrationErrors).toEqual([]);
  } finally {
    if (root) {
      await act(async () => root?.unmount());
    }
    consoleError.mockRestore();
    container.remove();
  }
}

/**
 * Verify same-document setters and cross-document storage events notify each consumer once.
 */
function synchronizesSubscribers(): void {
  window.localStorage.setItem(SELECTED_DATABASE_KEY, 'initial.sqlite');
  const listener = vi.fn();
  const unsubscribe = subscribeSelectedDatabase(listener);

  render(<SelectedDatabaseProbe />);
  expect(screen.getByText('initial.sqlite')).toBeInTheDocument();

  act(() => setSelectedDatabase('same-document.sqlite'));
  expect(screen.getByText('same-document.sqlite')).toBeInTheDocument();
  expect(listener).toHaveBeenCalledTimes(1);

  act(() => {
    window.localStorage.setItem(SELECTED_DATABASE_KEY, 'cross-document.sqlite');
    window.dispatchEvent(
      new StorageEvent('storage', {
        key: SELECTED_DATABASE_KEY,
        oldValue: 'same-document.sqlite',
        newValue: 'cross-document.sqlite',
      }),
    );
  });
  expect(screen.getByText('cross-document.sqlite')).toBeInTheDocument();
  expect(listener).toHaveBeenCalledTimes(2);

  unsubscribe();
  act(() => setSelectedDatabase('after-unsubscribe.sqlite'));
  expect(listener).toHaveBeenCalledTimes(2);
}

/**
 * Verify the legacy key still migrates into the namespaced storage key.
 */
function migratesLegacyStorage(): void {
  window.localStorage.setItem(LEGACY_SELECTED_DATABASE_KEY, 'legacy.sqlite');

  expect(getSelectedDatabaseSnapshot()).toBe('legacy.sqlite');
  expect(window.localStorage.getItem(SELECTED_DATABASE_KEY)).toBe('legacy.sqlite');
  expect(window.localStorage.getItem(LEGACY_SELECTED_DATABASE_KEY)).toBeNull();
}

/**
 * Verify unavailable storage falls back without throwing during reads or writes.
 */
function toleratesUnavailableStorage(): void {
  const getItem = vi.spyOn(Storage.prototype, 'getItem').mockImplementation(() => {
    throw new Error('storage read disabled');
  });
  expect(getSelectedDatabaseSnapshot()).toBe(DEFAULT_DATABASE);
  getItem.mockRestore();

  const setItem = vi.spyOn(Storage.prototype, 'setItem').mockImplementation(() => {
    throw new Error('storage write disabled');
  });
  const removeItem = vi.spyOn(Storage.prototype, 'removeItem').mockImplementation(() => {
    throw new Error('storage removal disabled');
  });
  expect(() => setSelectedDatabase('unpersisted.sqlite')).not.toThrow();
  setItem.mockRestore();
  removeItem.mockRestore();
}

/**
 * Verify a removed stored database is replaced by the first available database once.
 */
function reconcilesUnavailableDatabase(): void {
  window.localStorage.setItem(SELECTED_DATABASE_KEY, 'removed.sqlite');
  const listener = vi.fn();
  const unsubscribe = subscribeSelectedDatabase(listener);

  expect(
    reconcileSelectedDatabase('removed.sqlite', ['available.sqlite', 'secondary.sqlite']),
  ).toBe('available.sqlite');
  expect(window.localStorage.getItem(SELECTED_DATABASE_KEY)).toBe('available.sqlite');
  expect(listener).toHaveBeenCalledTimes(1);

  expect(
    reconcileSelectedDatabase('available.sqlite', ['available.sqlite', 'secondary.sqlite']),
  ).toBe('available.sqlite');
  expect(listener).toHaveBeenCalledTimes(1);
  expect(reconcileSelectedDatabase('available.sqlite', [])).toBe('available.sqlite');
  unsubscribe();
}

describe('selected database external store', () => {
  test('hydrates with the default snapshot before applying stored state', hydratesWithoutMismatch);
  test('synchronizes same-document and cross-document subscribers', synchronizesSubscribers);
  test('migrates the legacy storage key', migratesLegacyStorage);
  test('tolerates unavailable browser storage', toleratesUnavailableStorage);
  test('reconciles unavailable stored databases', reconcilesUnavailableDatabase);
});
