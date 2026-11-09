/**
 * Browser implementation of the client platform: same-origin fetch with session cookies.
 */

import type { ClientPlatform } from '@/lib/platform/types';

/**
 * Send one request with the browser session cookie.
 *
 * @param url - Absolute request URL.
 * @param init - Request options.
 * @returns Raw fetch response.
 */
function sendBrowserRequest(url: string, init: RequestInit): Promise<Response> {
  return fetch(url, { ...init, credentials: 'include' });
}

/**
 * Trigger one browser download and release its temporary object URL.
 *
 * @param blob - Download response body.
 * @param filename - Safe attachment filename.
 * @returns True, because the browser owns any later save prompt.
 */
async function saveBrowserDownload(blob: Blob, filename: string): Promise<boolean> {
  const objectUrl = window.URL.createObjectURL(blob);
  const anchor = document.createElement('a');
  anchor.href = objectUrl;
  anchor.download = filename;
  anchor.hidden = true;
  document.body.append(anchor);
  try {
    anchor.click();
  } finally {
    anchor.remove();
    window.URL.revokeObjectURL(objectUrl);
  }
  return true;
}

/**
 * Open an external URL in a new browsing context without exposing the opener.
 *
 * @param url - Absolute external URL.
 */
function openBrowserExternal(url: string): void {
  window.open(url, '_blank', 'noopener,noreferrer');
}

/**
 * Resolve the page origin, which serves the API behind the single-origin reverse proxy.
 *
 * @returns Page origin, or a loopback placeholder outside a browsing context.
 */
function resolveBrowserServerOrigin(): string {
  if (typeof window !== 'undefined' && window.location.origin !== 'null') {
    return window.location.origin;
  }
  return 'http://localhost';
}

/**
 * Leave the document unchanged; browsers handle links natively.
 */
function prepareBrowserDocument(): void {}

/** Browser platform used by the web build. */
export const WEB_PLATFORM: ClientPlatform = {
  send: sendBrowserRequest,
  saveDownload: saveBrowserDownload,
  openExternal: openBrowserExternal,
  serverOrigin: resolveBrowserServerOrigin,
  prepareDocument: prepareBrowserDocument,
  supportsSingleSignOn: true,
  nativeSession: null,
  serverConnection: null,
};
