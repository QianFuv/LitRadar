/**
 * Browser storage helpers that tolerate unavailable Web Storage APIs.
 */

/**
 * Read a localStorage value without assuming browser storage is available.
 *
 * @param key - Storage key.
 * @returns Stored value or null.
 */
export function readLocalStorageValue(key: string): string | null {
  if (typeof window === 'undefined') {
    return null;
  }
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
  if (typeof window === 'undefined') {
    return;
  }
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
  if (typeof window === 'undefined') {
    return;
  }
  try {
    window.localStorage.removeItem(key);
  } catch {}
}

/**
 * Read a sessionStorage value without assuming browser storage is available.
 *
 * @param key - Storage key.
 * @returns Stored value or null.
 */
export function readSessionStorageValue(key: string): string | null {
  if (typeof window === 'undefined') {
    return null;
  }
  try {
    return window.sessionStorage.getItem(key);
  } catch {
    return null;
  }
}

/**
 * Write a sessionStorage value without surfacing quota or privacy-mode errors.
 *
 * @param key - Storage key.
 * @param value - Value to store.
 */
export function writeSessionStorageValue(key: string, value: string): void {
  if (typeof window === 'undefined') {
    return;
  }
  try {
    window.sessionStorage.setItem(key, value);
  } catch {}
}

/**
 * Remove a sessionStorage value without assuming browser storage is available.
 *
 * @param key - Storage key.
 */
export function removeSessionStorageValue(key: string): void {
  if (typeof window === 'undefined') {
    return;
  }
  try {
    window.sessionStorage.removeItem(key);
  } catch {}
}
