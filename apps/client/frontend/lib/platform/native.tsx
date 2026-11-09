/**
 * Native implementation of the client platform for the Wails desktop and Android hosts.
 *
 * The Go `Bridge` service owns server traffic: it restricts requests to the configured
 * server origin and API paths and attaches the bearer token from OS secure storage, so
 * JavaScript never handles the token. This module is bundled only in the `native` Vite mode.
 */

import { Call, type CancellablePromise } from '@wailsio/runtime';

import { PlatformBridgeError } from '@/lib/platform/errors';
import type { ClientPlatform, NativeSession, ServerConnection } from '@/lib/platform/types';

/** Fully qualified Wails name of the Go bridge service. */
export const BRIDGE_SERVICE = 'github.com/QianFuv/LitRadar/apps/client/internal/nativehost.Bridge';

/** Upper bound for probing a server address during setup. */
export const SERVER_PROBE_TIMEOUT_MS = 15_000;

/** Origin reported before setup; the host rejects requests to it. */
const UNCONFIGURED_ORIGIN = 'https://litradar.invalid';

const NULL_BODY_STATUSES = new Set([101, 103, 204, 205, 304]);
const BASE64_CHUNK_SIZE = 0x8000;

/** Response shape returned by the Go bridge; Wails encodes the body bytes as base64. */
interface BridgeResponse {
  status: number;
  headers: Record<string, string> | null;
  body: string | null;
}

/** JavaScript interface the Android host exposes for saving into public Downloads. */
interface AndroidDownloads {
  save(filename: string, mimeType: string, base64Data: string): string;
}

declare global {
  interface Window {
    litradarDownloads?: AndroidDownloads;
  }
}

let activeOrigin: string | null = null;

/**
 * Call one bridge method and cancel the Go context when the signal aborts.
 *
 * @param method - Bridge method name.
 * @param signal - Optional cancellation signal.
 * @param args - Method arguments.
 * @returns Method result, or a rejection with the abort reason after cancellation.
 */
function invokeBridge<T>(
  method: string,
  signal: AbortSignal | null | undefined,
  ...args: unknown[]
): Promise<T> {
  if (signal?.aborted) {
    return Promise.reject(signal.reason);
  }
  const call = Call.ByName(`${BRIDGE_SERVICE}.${method}`, ...args) as CancellablePromise<T>;
  if (!signal) {
    return call;
  }
  /** Cancel the pending Go call with the caller's reason. */
  function cancelCall(): void {
    call.cancel(signal?.reason).catch(() => undefined);
  }
  signal.addEventListener('abort', cancelCall, { once: true });
  return call.then(
    (value) => {
      signal.removeEventListener('abort', cancelCall);
      return value;
    },
    (error: unknown) => {
      signal.removeEventListener('abort', cancelCall);
      throw signal.aborted ? signal.reason : error;
    },
  );
}

/**
 * Read the stable code a Go bridge error carries in its runtime error cause.
 *
 * @param error - Rejection from a bridge call.
 * @returns Host error code, or `unknown`.
 */
function readBridgeErrorCode(error: unknown): string {
  if (error instanceof Error && error.cause && typeof error.cause === 'object') {
    const code = (error.cause as Record<string, unknown>).code;
    if (typeof code === 'string' && code) {
      return code;
    }
  }
  return 'unknown';
}

/**
 * Convert a bridge rejection into a typed host error.
 *
 * @param error - Rejection from a bridge call.
 * @returns Typed host error with the original cause.
 */
function toPlatformError(error: unknown): PlatformBridgeError {
  const message = error instanceof Error ? error.message : String(error);
  return new PlatformBridgeError(readBridgeErrorCode(error), message, { cause: error });
}

/**
 * Call a bridge method that performs a server request, rejecting like `fetch`: with the
 * abort reason after cancellation and with a `TypeError` after transport failures.
 *
 * @param method - Bridge method name.
 * @param signal - Optional cancellation signal.
 * @param args - Method arguments.
 * @returns Server response.
 */
async function invokeTransport(
  method: string,
  signal: AbortSignal | null | undefined,
  ...args: unknown[]
): Promise<Response> {
  let result: BridgeResponse;
  try {
    result = await invokeBridge<BridgeResponse>(method, signal, ...args);
  } catch (error) {
    if (signal?.aborted) {
      throw signal.reason;
    }
    throw new TypeError('无法连接 LitRadar 服务器', { cause: toPlatformError(error) });
  }
  return toResponse(result);
}

/**
 * Decode a base64 string into bytes.
 *
 * @param value - Base64 text, or null for an empty body.
 * @returns Decoded bytes.
 */
export function decodeBase64(value: string | null): Uint8Array<ArrayBuffer> {
  const binary = atob(value ?? '');
  const bytes = new Uint8Array(binary.length);
  for (let index = 0; index < binary.length; index += 1) {
    bytes[index] = binary.charCodeAt(index);
  }
  return bytes;
}

/**
 * Encode a blob as base64 for the bridge.
 *
 * @param blob - Binary content.
 * @returns Base64 text.
 */
export async function encodeBase64(blob: Blob): Promise<string> {
  const bytes = new Uint8Array(await blob.arrayBuffer());
  let binary = '';
  for (let offset = 0; offset < bytes.length; offset += BASE64_CHUNK_SIZE) {
    binary += String.fromCharCode(...bytes.subarray(offset, offset + BASE64_CHUNK_SIZE));
  }
  return btoa(binary);
}

/**
 * Rebuild a standard response from the bridge result.
 *
 * @param result - Bridge response.
 * @returns Equivalent fetch response.
 */
function toResponse(result: BridgeResponse): Response {
  const body = NULL_BODY_STATUSES.has(result.status) ? null : decodeBase64(result.body);
  return new Response(body, { status: result.status, headers: result.headers ?? {} });
}

/**
 * Send one API request through the Go bridge.
 *
 * @param url - Absolute request URL on the configured server.
 * @param init - Request options with a text body.
 * @returns Server response.
 */
async function sendNativeRequest(url: string, init: RequestInit): Promise<Response> {
  const body = init.body;
  if (body !== undefined && body !== null && typeof body !== 'string') {
    throw new TypeError('The native transport sends only text request bodies');
  }
  const headers: Record<string, string> = {};
  new Headers(init.headers).forEach((value, name) => {
    headers[name] = value;
  });
  return invokeTransport('Send', init.signal, {
    method: init.method ?? 'GET',
    url,
    headers,
    body: body ?? '',
  });
}

/**
 * Save a download into Android Downloads or through the desktop save dialog.
 *
 * @param blob - File contents.
 * @param filename - Suggested file name.
 * @returns Whether the file was saved.
 */
async function saveNativeDownload(blob: Blob, filename: string): Promise<boolean> {
  const data = await encodeBase64(blob);
  const androidDownloads = window.litradarDownloads;
  if (androidDownloads) {
    const result: unknown = JSON.parse(
      androidDownloads.save(filename, blob.type || 'application/octet-stream', data),
    );
    if (!result || typeof result !== 'object' || (result as { ok?: unknown }).ok !== true) {
      throw new Error('保存到“下载”失败');
    }
    return true;
  }
  try {
    return await invokeBridge<boolean>('SaveDownload', null, filename, data);
  } catch (error) {
    throw new Error('保存文件失败', { cause: toPlatformError(error) });
  }
}

/**
 * Open an external URL in the system browser.
 *
 * @param url - Absolute http or https URL.
 */
function openNativeExternal(url: string): void {
  invokeBridge<void>('OpenExternal', null, url).catch(() => undefined);
}

/**
 * Resolve the configured server origin.
 *
 * @returns Server origin, or an invalid placeholder before setup.
 */
function resolveNativeServerOrigin(): string {
  return activeOrigin ?? UNCONFIGURED_ORIGIN;
}

/**
 * Route clicks on external links to the system browser instead of the app WebView.
 *
 * @param event - Click or middle-click event.
 */
export function handleExternalNavigation(event: MouseEvent): void {
  const isPrimaryClick = event.type === 'click' && event.button === 0;
  const isMiddleClick = event.type === 'auxclick' && event.button === 1;
  if (event.defaultPrevented || (!isPrimaryClick && !isMiddleClick)) {
    return;
  }
  const anchor = event.target instanceof Element ? event.target.closest('a[href]') : null;
  if (!(anchor instanceof HTMLAnchorElement)) {
    return;
  }
  const destination = new URL(anchor.href, window.location.href);
  const isWebLink = destination.protocol === 'https:' || destination.protocol === 'http:';
  if (!isWebLink || destination.origin === window.location.origin) {
    return;
  }
  event.preventDefault();
  openNativeExternal(destination.href);
}

/**
 * Install the external-link handler on the document.
 */
function prepareNativeDocument(): void {
  document.addEventListener('click', handleExternalNavigation);
  document.addEventListener('auxclick', handleExternalNavigation);
}

/** Credential operations backed by the Go bridge. */
const NATIVE_SESSION: NativeSession = {
  login: (username, password, signal) => invokeTransport('Login', signal, username, password),
  importAccessToken: (token, signal) => invokeTransport('ImportToken', signal, token),
  logout: (signal) => invokeTransport('Logout', signal),
  async discardCredential() {
    await invokeBridge<void>('ForgetToken', null);
  },
};

/** Server selection backed by the Go host configuration file. */
const NATIVE_SERVER_CONNECTION: ServerConnection = {
  async load() {
    const origin = await invokeBridge<string>('ServerOrigin', null);
    activeOrigin = origin || null;
    return activeOrigin;
  },
  async connect(address) {
    try {
      const origin = await invokeBridge<string>(
        'ConfigureServer',
        AbortSignal.timeout(SERVER_PROBE_TIMEOUT_MS),
        address,
      );
      activeOrigin = origin;
      return origin;
    } catch (error) {
      if (error instanceof DOMException && error.name === 'TimeoutError') {
        throw new PlatformBridgeError('server_unreachable', 'server probe timed out', {
          cause: error,
        });
      }
      throw toPlatformError(error);
    }
  },
  async disconnect() {
    await invokeBridge<void>('ForgetServer', null);
    activeOrigin = null;
  },
};

/** Native platform used by the Wails desktop and Android builds. */
export const NATIVE_PLATFORM: ClientPlatform = {
  send: sendNativeRequest,
  saveDownload: saveNativeDownload,
  openExternal: openNativeExternal,
  serverOrigin: resolveNativeServerOrigin,
  prepareDocument: prepareNativeDocument,
  supportsSingleSignOn: false,
  nativeSession: NATIVE_SESSION,
  serverConnection: NATIVE_SERVER_CONNECTION,
};

export { NATIVE_PLATFORM as ACTIVE_PLATFORM };
