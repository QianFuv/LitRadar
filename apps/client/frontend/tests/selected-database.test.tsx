/**
 * Storage, subscription, reconciliation, and effective-database coverage for database selection.
 */

import { act } from 'react';
import { render, screen } from '@testing-library/react';
import { http, HttpResponse } from 'msw';
import { describe, expect, test, vi } from 'vite-plus/test';

import { DEFAULT_DATABASE, SELECTED_DATABASE_KEY, readSelectedDatabase } from '@/lib/api/client';
import {
  reconcileSelectedDatabase,
  setSelectedDatabase,
  useActiveDatabase,
  useSelectedDatabase,
} from '@/lib/selected-database';
import { server } from '@/tests/mocks/server';
import { renderWithQuery } from '@/tests/render';

vi.mock('@/lib/auth-context', () => ({
  useAuth: () => ({ user: { id: 1, username: 'database_user', is_admin: false } }),
}));

const LEGACY_SELECTED_DATABASE_KEY = 'selected_database';

/** Render the selected database as text. */
function SelectedDatabaseProbe() {
  const selectedDatabase = useSelectedDatabase();
  return <output>{selectedDatabase}</output>;
}

/**
 * Verify same-document setters and cross-document storage events reach subscribers.
 */
function synchronizesSubscribers(): void {
  act(() => setSelectedDatabase('initial.sqlite'));
  render(<SelectedDatabaseProbe />);
  expect(screen.getByText('initial.sqlite')).toBeInTheDocument();

  act(() => setSelectedDatabase('same-document.sqlite'));
  expect(screen.getByText('same-document.sqlite')).toBeInTheDocument();
  expect(window.localStorage.getItem(SELECTED_DATABASE_KEY)).toBe('same-document.sqlite');

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

  act(() => {
    window.localStorage.setItem('unrelated', 'value');
    window.dispatchEvent(new StorageEvent('storage', { key: 'unrelated' }));
  });
  expect(screen.getByText('cross-document.sqlite')).toBeInTheDocument();
}

/**
 * Verify the legacy key migrates to the namespaced key on first read.
 */
function migratesLegacyStorage(): void {
  window.localStorage.removeItem(SELECTED_DATABASE_KEY);
  window.localStorage.setItem(LEGACY_SELECTED_DATABASE_KEY, 'legacy.sqlite');

  expect(readSelectedDatabase()).toBe('legacy.sqlite');
  expect(window.localStorage.getItem(SELECTED_DATABASE_KEY)).toBe('legacy.sqlite');
  expect(window.localStorage.getItem(LEGACY_SELECTED_DATABASE_KEY)).toBeNull();
}

/**
 * Verify unavailable storage falls back to the default and never breaks a selection change.
 */
function toleratesUnavailableStorage(): void {
  const getItem = vi.spyOn(Storage.prototype, 'getItem').mockImplementation(() => {
    throw new Error('storage read disabled');
  });
  expect(readSelectedDatabase()).toBe(DEFAULT_DATABASE);
  getItem.mockRestore();

  const setItem = vi.spyOn(Storage.prototype, 'setItem').mockImplementation(() => {
    throw new Error('storage write disabled');
  });
  const removeItem = vi.spyOn(Storage.prototype, 'removeItem').mockImplementation(() => {
    throw new Error('storage removal disabled');
  });
  render(<SelectedDatabaseProbe />);
  expect(() => act(() => setSelectedDatabase('unpersisted.sqlite'))).not.toThrow();
  expect(screen.getByText('unpersisted.sqlite')).toBeInTheDocument();
  setItem.mockRestore();
  removeItem.mockRestore();
}

/**
 * Verify an unavailable stored database falls back to the first available one only when needed.
 */
function reconcilesUnavailableDatabase(): void {
  act(() => setSelectedDatabase('removed.sqlite'));
  render(<SelectedDatabaseProbe />);

  act(() => {
    expect(
      reconcileSelectedDatabase('removed.sqlite', ['available.sqlite', 'secondary.sqlite']),
    ).toBe('available.sqlite');
  });
  expect(window.localStorage.getItem(SELECTED_DATABASE_KEY)).toBe('available.sqlite');
  expect(screen.getByText('available.sqlite')).toBeInTheDocument();

  const setItem = vi.spyOn(Storage.prototype, 'setItem');
  expect(
    reconcileSelectedDatabase('available.sqlite', ['available.sqlite', 'secondary.sqlite']),
  ).toBe('available.sqlite');
  expect(reconcileSelectedDatabase('available.sqlite', [])).toBe('available.sqlite');
  expect(setItem).not.toHaveBeenCalled();
}

/** Render the effective database, or `pending` while it is unknown. */
function ActiveDatabaseProbe() {
  const activeDatabase = useActiveDatabase();
  return <output>{activeDatabase ?? 'pending'}</output>;
}

/**
 * Verify the effective database stays unknown until the server's list arrives, then resolves to an
 * offered database and persists it in place of an unavailable stored one.
 */
async function waitsForOfferedDatabase(): Promise<void> {
  act(() => setSelectedDatabase('removed.sqlite'));
  let releaseDatabases: (() => void) | undefined;
  const pending = new Promise<void>((resolve) => {
    releaseDatabases = resolve;
  });
  server.use(
    http.get('http://localhost/api/meta/databases', async () => {
      await pending;
      return HttpResponse.json(['offered.sqlite', 'secondary.sqlite']);
    }),
  );
  await renderWithQuery(<ActiveDatabaseProbe />);

  expect(screen.getByText('pending')).toBeInTheDocument();
  releaseDatabases?.();
  expect(await screen.findByText('offered.sqlite')).toBeInTheDocument();
  expect(window.localStorage.getItem(SELECTED_DATABASE_KEY)).toBe('offered.sqlite');
}

/**
 * Verify a failed database list leaves the stored selection usable instead of blocking queries.
 */
async function keepsStoredDatabaseWhenListFails(): Promise<void> {
  act(() => setSelectedDatabase('stored.sqlite'));
  server.use(
    http.get('http://localhost/api/meta/databases', () =>
      HttpResponse.json(
        { title: 'Internal Server Error', status: 500, detail: 'unavailable' },
        { status: 500, headers: { 'Content-Type': 'application/problem+json' } },
      ),
    ),
  );
  await renderWithQuery(<ActiveDatabaseProbe />);

  expect(await screen.findByText('stored.sqlite')).toBeInTheDocument();
}

describe('selected database store', () => {
  test('synchronizes same-document and cross-document subscribers', synchronizesSubscribers);
  test('migrates the legacy storage key', migratesLegacyStorage);
  test('tolerates unavailable browser storage', toleratesUnavailableStorage);
  test('reconciles unavailable stored databases', reconcilesUnavailableDatabase);
  test('waits for the offered database list', waitsForOfferedDatabase);
  test('keeps the stored database when the list fails', keepsStoredDatabaseWhenListFails);
});
