/**
 * Runtime type guards for values that cross a trust boundary: API payloads, storage, and native
 * bridge results.
 */

/**
 * Return whether an unknown value is a non-null object record.
 *
 * @param value - Value to inspect.
 * @returns Whether the value is a record.
 */
export function isRecord(value: unknown): value is Record<string, unknown> {
  return Boolean(value && typeof value === 'object' && !Array.isArray(value));
}

/**
 * Check whether a value is text.
 *
 * @param value - Value to inspect.
 * @returns Whether the value is a string.
 */
export function isString(value: unknown): value is string {
  return typeof value === 'string';
}

/**
 * Check whether a value is a boolean flag.
 *
 * @param value - Value to inspect.
 * @returns Whether the value is a boolean.
 */
export function isBoolean(value: unknown): value is boolean {
  return typeof value === 'boolean';
}

/**
 * Check whether a value is a finite number.
 *
 * @param value - Value to inspect.
 * @returns Whether the value is a finite number.
 */
export function isFiniteNumber(value: unknown): value is number {
  return typeof value === 'number' && Number.isFinite(value);
}
