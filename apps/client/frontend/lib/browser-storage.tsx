/**
 * Browser storage helpers that tolerate unavailable Web Storage APIs.
 */

import type { z } from 'zod';

/**
 * Read a localStorage value without assuming browser storage is available.
 *
 * @param key - Storage key.
 * @returns Stored value or null.
 */
export function readLocalStorageValue(key: string): string | null {
  try {
    return window.localStorage.getItem(key);
  } catch {
    return null;
  }
}

/**
 * Write a localStorage value without surfacing quota or privacy-mode errors.
 *
 * @param key - Storage key.
 * @param value - Value to store.
 */
export function writeLocalStorageValue(key: string, value: string): void {
  try {
    window.localStorage.setItem(key, value);
  } catch {}
}

/**
 * Remove a localStorage value without assuming browser storage is available.
 *
 * @param key - Storage key.
 */
export function removeLocalStorageValue(key: string): void {
  try {
    window.localStorage.removeItem(key);
  } catch {}
}

/**
 * Remove a sessionStorage value without assuming browser storage is available.
 *
 * @param key - Storage key.
 */
export function removeSessionStorageValue(key: string): void {
  try {
    window.sessionStorage.removeItem(key);
  } catch {}
}

/**
 * Parse stored JSON text against a schema, treating malformed or mismatched data as absent.
 *
 * @param rawValue - Serialized value read from Web Storage.
 * @param schema - Expected shape of the stored value.
 * @returns Parsed value, or null when the text is not valid JSON or does not match.
 */
export function parseStoredJson<T>(rawValue: string, schema: z.ZodType<T>): T | null {
  try {
    const result = schema.safeParse(JSON.parse(rawValue));
    return result.success ? result.data : null;
  } catch {
    return null;
  }
}
