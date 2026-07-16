'use client';

/**
 * Hydration-safe external store for the selected article database.
 */

import { useSyncExternalStore } from 'react';

import {
  DEFAULT_DATABASE,
  SELECTED_DATABASE_KEY,
  readSelectedDatabase,
  storeSelectedDatabase,
} from '@/lib/api/client';

/** Legacy key retained for cross-tab migration notifications. */
const LEGACY_SELECTED_DATABASE_KEY = 'selected_database';

/** Active subscribers within the current browser document. */
const SELECTED_DATABASE_LISTENERS = new Set<() => void>();

/**
 * Return a stable no-op cleanup when subscription is requested outside a browser.
 */
function noopSubscriptionCleanup(): void {}

/**
 * Notify every subscriber that the selected database snapshot may have changed.
 */
function emitSelectedDatabaseChange(): void {
  for (const listener of SELECTED_DATABASE_LISTENERS) {
    listener();
  }
}

/**
 * Forward relevant cross-document storage events to external-store subscribers.
 *
 * @param event - Browser storage event.
 */
function handleSelectedDatabaseStorage(event: StorageEvent): void {
  if (
    event.key === null ||
    event.key === SELECTED_DATABASE_KEY ||
    event.key === LEGACY_SELECTED_DATABASE_KEY
  ) {
    emitSelectedDatabaseChange();
  }
}

/**
 * Read the current browser snapshot for React external-store comparison.
 *
 * @returns Persisted selected database or the default database.
 */
export function getSelectedDatabaseSnapshot(): string {
  return readSelectedDatabase();
}

/**
 * Return the deterministic snapshot used by server rendering and hydration.
 *
 * @returns Default database name.
 */
export function getSelectedDatabaseServerSnapshot(): string {
  return DEFAULT_DATABASE;
}

/**
 * Subscribe to same-document setters and cross-document storage changes.
 *
 * @param listener - React or test listener to notify after a possible change.
 * @returns Cleanup function that removes this subscription.
 */
export function subscribeSelectedDatabase(listener: () => void): () => void {
  if (typeof window === 'undefined') {
    return noopSubscriptionCleanup;
  }

  SELECTED_DATABASE_LISTENERS.add(listener);
  if (SELECTED_DATABASE_LISTENERS.size === 1) {
    window.addEventListener('storage', handleSelectedDatabaseStorage);
  }

  return () => {
    SELECTED_DATABASE_LISTENERS.delete(listener);
    if (SELECTED_DATABASE_LISTENERS.size === 0) {
      window.removeEventListener('storage', handleSelectedDatabaseStorage);
    }
  };
}

/**
 * Persist and publish a selected database change within the current document.
 *
 * @param dbName - Database file name.
 */
export function setSelectedDatabase(dbName: string): void {
  storeSelectedDatabase(dbName);
  emitSelectedDatabaseChange();
}

/**
 * Resolve a selected database against the currently available database list.
 *
 * @param selectedDatabase - Current stored or external-store value.
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
 * @param selectedDatabase - Current stored or external-store value.
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
 * Read the hydration-safe selected database snapshot in a Client Component.
 *
 * @returns Selected database name.
 */
export function useSelectedDatabase(): string {
  return useSyncExternalStore(
    subscribeSelectedDatabase,
    getSelectedDatabaseSnapshot,
    getSelectedDatabaseServerSnapshot,
  );
}
