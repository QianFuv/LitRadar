/**
 * Selected article database as a TanStack Store atom persisted in local storage.
 *
 * Writes go through {@link setSelectedDatabase}, which persists the value and updates the atom;
 * `storage` events bring changes made in other documents into this one.
 */

import { useQuery } from '@tanstack/react-query';
import { createAtom, useSelector } from '@tanstack/react-store';
import { useEffect } from 'react';

import { databasesQuery } from '@/lib/api';
import { useAuth } from '@/lib/auth-context';

import {
  SELECTED_DATABASE_KEY,
  readSelectedDatabase,
  storeSelectedDatabase,
} from '@/lib/api/client';

/** Legacy key retained for cross-tab migration notifications. */
const LEGACY_SELECTED_DATABASE_KEY = 'selected_database';

/** Selected database shared by every component in this document. */
const selectedDatabaseAtom = createAtom(readSelectedDatabase());

/**
 * Reload the atom when another document changes the stored selection.
 *
 * @param event - Browser storage event.
 */
function handleSelectedDatabaseStorage(event: StorageEvent): void {
  if (
    event.key === null ||
    event.key === SELECTED_DATABASE_KEY ||
    event.key === LEGACY_SELECTED_DATABASE_KEY
  ) {
    selectedDatabaseAtom.set(readSelectedDatabase());
  }
}

window.addEventListener('storage', handleSelectedDatabaseStorage);

/**
 * Persist and publish a selected database change.
 *
 * @param dbName - Database file name.
 */
export function setSelectedDatabase(dbName: string): void {
  storeSelectedDatabase(dbName);
  selectedDatabaseAtom.set(dbName);
}

/**
 * Resolve a selected database against the currently available database list.
 *
 * @param selectedDatabase - Current selection.
 * @param availableDatabases - Databases returned by the metadata endpoint.
 * @returns Current value when valid, otherwise the first available value.
 */
function resolveAvailableSelectedDatabase(
  selectedDatabase: string,
  availableDatabases: readonly string[],
): string {
  if (availableDatabases.length === 0 || availableDatabases.includes(selectedDatabase)) {
    return selectedDatabase;
  }
  return availableDatabases[0];
}

/**
 * Persist the safe fallback when a stored database no longer exists.
 *
 * @param selectedDatabase - Current selection.
 * @param availableDatabases - Databases returned by the metadata endpoint.
 * @returns Reconciled database value.
 */
export function reconcileSelectedDatabase(
  selectedDatabase: string,
  availableDatabases: readonly string[],
): string {
  const resolvedDatabase = resolveAvailableSelectedDatabase(selectedDatabase, availableDatabases);
  if (resolvedDatabase !== selectedDatabase) {
    setSelectedDatabase(resolvedDatabase);
  }
  return resolvedDatabase;
}

/**
 * Read the selected database and re-render when it changes.
 *
 * @returns Selected database name.
 */
export function useSelectedDatabase(): string {
  return useSelector(selectedDatabaseAtom);
}

/**
 * Resolve the database that database-scoped queries should use: the stored selection once the
 * server's database list confirms it, or the list's first database when the stored one (or the
 * built-in default) is not offered, which is then persisted. Until the list arrives the result is
 * null, so dependent queries wait instead of sending a request for a database that may be
 * replaced a moment later. When the list fails to load the stored selection is used as is.
 *
 * @returns Effective database name, or null while the database list is loading.
 */
export function useActiveDatabase(): string | null {
  const { user } = useAuth();
  const selectedDatabase = useSelectedDatabase();
  const { data: databases, isError } = useQuery({ ...databasesQuery(), enabled: !!user });

  useEffect(() => {
    if (databases) {
      reconcileSelectedDatabase(selectedDatabase, databases);
    }
  }, [databases, selectedDatabase]);

  if (databases) {
    return resolveAvailableSelectedDatabase(selectedDatabase, databases);
  }
  return isError ? selectedDatabase : null;
}
