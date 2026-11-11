/**
 * Selected article database as a TanStack Store atom persisted in local storage.
 *
 * Writes go through {@link setSelectedDatabase}, which persists the value and updates the atom;
 * `storage` events bring changes made in other documents into this one.
 */

import { createAtom, useSelector } from '@tanstack/react-store';

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
export function resolveAvailableSelectedDatabase(
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
