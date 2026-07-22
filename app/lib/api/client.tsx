/**
 * Shared browser API transport, error, URL, and database-selection helpers.
 */

import {
  readLocalStorageValue,
  removeLocalStorageValue,
  writeLocalStorageValue,
} from '@/lib/browser-storage';
import type { ContractParser } from '@/lib/api-contract';

export interface ApiErrorInfo {
  code: string | null;
  message: string;
  phase: string | null;
}

/**
 * Error raised for non-2xx API responses.
 */
export class ApiError extends Error {
  readonly code: string | null;
  readonly phase: string | null;
  readonly requestId: string | null;
  readonly status: number;

  /**
   * Create an API error with optional backend classification.
   *
   * @param message - Displayable error message.
   * @param status - HTTP status code.
   * @param code - Stable backend error code.
   * @param phase - Backend workflow phase that failed.
   * @param requestId - Server-generated request identifier exposed by CORS.
   */
  constructor(
    message: string,
    status: number,
    code: string | null,
    phase: string | null,
    requestId: string | null = null,
  ) {
    super(message);
    this.name = 'ApiError';
    this.status = status;
    this.code = code;
    this.phase = phase;
    this.requestId = requestId;
    Object.setPrototypeOf(this, ApiError.prototype);
  }
}
export const DEFAULT_DATABASE = 'ccf_computer_journals.sqlite';
export const DEFAULT_DB = DEFAULT_DATABASE;
export const SELECTED_DATABASE_KEY = 'litradar:v1:selected_database';
const LEGACY_SELECTED_DATABASE_KEY = 'selected_database';

const REQUEST_ID_HEADER = 'X-Request-Id';

/**
 * Resolve the backend base URL for client or server-side rendering.
 *
 * @returns Absolute backend URL.
 */
export function resolveApiBase(): string {
  if (typeof window !== 'undefined' && window.location.origin !== 'null') {
    return window.location.origin;
  }
  return 'http://localhost';
}

/**
 * Read the selected index database from local storage.
 *
 * @returns Selected database name.
 */
export function readSelectedDatabase(): string {
  if (typeof window === 'undefined') {
    return DEFAULT_DATABASE;
  }
  const selectedDatabase = readLocalStorageValue(SELECTED_DATABASE_KEY);
  if (selectedDatabase) {
    return selectedDatabase;
  }
  const legacySelectedDatabase = readLocalStorageValue(LEGACY_SELECTED_DATABASE_KEY);
  if (legacySelectedDatabase) {
    storeSelectedDatabase(legacySelectedDatabase);
    removeLocalStorageValue(LEGACY_SELECTED_DATABASE_KEY);
    return legacySelectedDatabase;
  }
  return DEFAULT_DATABASE;
}

/**
 * Store the selected index database in local storage.
 *
 * @param dbName - Database file name.
 */
export function storeSelectedDatabase(dbName: string): void {
  if (typeof window !== 'undefined') {
    writeLocalStorageValue(SELECTED_DATABASE_KEY, dbName);
    removeLocalStorageValue(LEGACY_SELECTED_DATABASE_KEY);
  }
}

/**
 * Build an absolute URL from a backend path.
 *
 * @param path - API path.
 * @param params - Query parameters to append.
 * @returns Absolute URL.
 */
export function buildApiUrl(path: string, params?: URLSearchParams): string {
  const url = new URL(path, resolveApiBase());
  params?.forEach((value, key) => {
    url.searchParams.append(key, value);
  });
  return url.toString();
}

/**
 * Build an absolute URL for a database-backed API path.
 *
 * @param path - API path.
 * @param dbName - Database name.
 * @param params - Query parameters to append.
 * @returns Absolute URL.
 */
export function buildDatabaseUrl(path: string, dbName: string, params?: URLSearchParams): string {
  const url = new URL(buildApiUrl(path, params));
  if (!url.searchParams.has('db')) {
    url.searchParams.set('db', dbName);
  }
  return url.toString();
}

/**
 * Check whether an unknown value is a string-keyed object.
 *
 * @param value - Value to inspect.
 * @returns Whether the value is a record.
 */
function isRecord(value: unknown): value is Record<string, unknown> {
  return Boolean(value && typeof value === 'object');
}

/**
 * Convert an unknown backend error payload into structured error info.
 *
 * @param payload - Parsed backend error payload.
 * @param fallback - Fallback message.
 * @returns Structured API error info.
 */
function extractErrorInfo(payload: unknown, fallback: string): ApiErrorInfo {
  if (isRecord(payload) && 'detail' in payload) {
    const detail = payload.detail;
    if (typeof detail === 'string') {
      return { code: null, message: detail, phase: null };
    }
    if (isRecord(detail)) {
      const code = typeof detail.code === 'string' ? detail.code : null;
      const message = typeof detail.message === 'string' ? detail.message : fallback;
      const phase = typeof detail.phase === 'string' ? detail.phase : null;
      return { code, message, phase };
    }
  }
  return { code: null, message: fallback, phase: null };
}

/**
 * Parse a fetch response as JSON and raise a typed error on failure.
 *
 * @param response - Fetch response.
 * @param fallback - Fallback error message.
 * @param parser - Optional runtime contract parser for control-plane responses.
 * @returns Parsed response body.
 */
async function parseJson<T>(
  response: Response,
  fallback: string,
  parser?: ContractParser<T>,
): Promise<T> {
  if (response.ok) {
    const payload: unknown = await response.json();
    return parser ? parser(payload) : (payload as T);
  }
  const payload = await response.json().catch(() => null);
  const errorInfo = extractErrorInfo(payload, fallback);
  throw new ApiError(
    errorInfo.message,
    response.status,
    errorInfo.code,
    errorInfo.phase,
    response.headers.get(REQUEST_ID_HEADER),
  );
}

/**
 * Fetch JSON from an endpoint using browser cookies and optional bearer auth.
 *
 * @param url - Absolute endpoint URL.
 * @param token - Optional explicit bearer access token.
 * @param init - Fetch options.
 * @param fallback - Fallback error message.
 * @param parser - Optional runtime contract parser for control-plane responses.
 * @returns Parsed response body.
 */
export async function requestJson<T>(
  url: string,
  token?: string | null,
  init?: RequestInit,
  fallback = '请求失败',
  parser?: ContractParser<T>,
): Promise<T> {
  const hasBody = typeof init?.body !== 'undefined';
  const headers: Record<string, string> = {
    ...(hasBody ? { 'Content-Type': 'application/json' } : {}),
    ...(token ? { Authorization: `Bearer ${token}` } : {}),
    ...(init?.headers as Record<string, string> | undefined),
  };
  const response = await fetch(url, { ...init, credentials: 'include', headers });
  return parseJson<T>(response, fallback, parser);
}
