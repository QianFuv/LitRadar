/**
 * Clipboard helpers for guarded browser copy actions.
 */

/**
 * Copy text to the browser clipboard when the API is available.
 *
 * @param text - Text to copy.
 */
export async function copyTextToClipboard(text: string): Promise<void> {
  if (typeof navigator === 'undefined' || !navigator.clipboard) {
    throw new Error('Clipboard API is unavailable');
  }
  await navigator.clipboard.writeText(text);
}
