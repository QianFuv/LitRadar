/**
 * Content identifiers shaped like the server's 63-bit hashes.
 */

/** Leading digits shared by fixture identifiers; every result exceeds `Number.MAX_SAFE_INTEGER`. */
const HASHED_ID_PREFIX = '922336182978630';

/**
 * Build a decimal article, journal, or issue identifier of real magnitude from a small seed, so
 * fixtures stay readable while any numeric conversion visibly corrupts the value.
 *
 * @param seed - Fixture number from 0 to 9999.
 * @returns 19-digit decimal identifier above 2^53 and below 2^63.
 */
export function hashedId(seed: number): string {
  if (!Number.isInteger(seed) || seed < 0 || seed > 9999) {
    throw new Error(`Hashed fixture id seed out of range: ${seed}`);
  }
  return `${HASHED_ID_PREFIX}${String(seed).padStart(4, '0')}`;
}
