/**
 * Shared API transport, error, URL, and database-selection helpers.
 */

import { z } from 'zod';

import {
  readLocalStorageValue,
  removeLocalStorageValue,
  writeLocalStorageValue,
} from '@/lib/browser-storage';
import type { ContractParser } from '@/lib/api-contract';
import { CLIENT_PLATFORM } from '@/lib/platform';

/** Problem type of a request attempt that exceeded {@link REQUEST_TIMEOUT_MS}. */
export const REQUEST_TIMEOUT_TYPE = 'urn:litradar:problem:request-timeout';

const PROBLEM_ERROR_DETAIL_SCHEMA = z.object({
  message: z.string().optional().catch(undefined),
  location: z.string().optional().catch(undefined),
  value: z.unknown().optional(),
});

/**
 * RFC 9457 problem details returned with `application/problem+json` error responses.
 * Malformed members are treated as absent so one bad field keeps the rest of the problem.
 */
const PROBLEM_DETAILS_SCHEMA = z.object({
  type: z.string().optional().catch(undefined),
  title: z.string().optional().catch(undefined),
  status: z.int().optional().catch(undefined),
  detail: z.string().optional().catch(undefined),
  instance: z.string().optional().catch(undefined),
  errors: z.array(PROBLEM_ERROR_DETAIL_SCHEMA).optional().catch(undefined),
});

/** One entry of the problem `errors` array: a message and the input location it refers to. */
export type ProblemErrorDetail = z.output<typeof PROBLEM_ERROR_DETAIL_SCHEMA>;

/** RFC 9457 problem details as read from an error response body. */
export type ProblemDetails = z.output<typeof PROBLEM_DETAILS_SCHEMA>;

/** Classification of a failed request beyond its message and HTTP status. */
export type ApiErrorInit = {
  /** Problem type URI. */
  type?: string | null;
  /** Short summary of the problem type. */
  title?: string | null;
  /** URI reference that identifies this occurrence. */
  instance?: string | null;
  /** Detailed entries, such as invalid fields or classified business failures. */
  errors?: readonly ProblemErrorDetail[];
  /** Server-generated request identifier exposed by CORS. */
  requestId?: string | null;
  /** Whether retrying may succeed without user correction. */
  retryable?: boolean;
  /** Server-provided retry delay in seconds. */
  retryAfterSeconds?: number | null;
};

/** Completed browser download response. */
export interface ApiDownload {
  /** Response body retained as a browser Blob. */
  blob: Blob;
  /** Safe server-provided filename when available. */
  filename: string | null;
}

/**
 * Error raised for non-2xx API responses and failed request attempts.
 */
export class ApiError extends Error {
  readonly errors: readonly ProblemErrorDetail[];
  readonly instance: string | null;
  readonly requestId: string | null;
  readonly retryable: boolean;
  readonly retryAfterSeconds: number | null;
  readonly status: number;
  readonly title: string | null;
  readonly type: string | null;

  /**
   * Create an API error from a display message, an HTTP status, and optional problem details.
   *
   * @param message - Displayable error message.
   * @param status - HTTP status code, or 0 when no response arrived.
   * @param init - Problem details and retry classification.
   */
  constructor(message: string, status: number, init: ApiErrorInit = {}) {
    super(message);
    this.name = 'ApiError';
    this.status = status;
    this.type = init.type ?? null;
    this.title = init.title ?? null;
    this.instance = init.instance ?? null;
    this.errors = init.errors ?? [];
    this.requestId = init.requestId ?? null;
    this.retryable = init.retryable ?? false;
    this.retryAfterSeconds = init.retryAfterSeconds ?? null;
    Object.setPrototypeOf(this, ApiError.prototype);
  }

  /**
   * Read the message of the first problem detail reported at one location.
   *
   * @param location - Detail location, such as a field path or a classification key.
   * @returns Detail message, or null when no entry has that location.
   */
  findErrorDetail(location: string): string | null {
    return this.errors.find((detail) => detail.location === location)?.message ?? null;
  }
}
export const DEFAULT_DATABASE = 'ccf_computer_journals.sqlite';
export const SELECTED_DATABASE_KEY = 'litradar:v1:selected_database';
const LEGACY_SELECTED_DATABASE_KEY = 'selected_database';

const REQUEST_ID_HEADER = 'X-Request-Id';

// ponytail: one ceiling for every request; add a per-call override when a longer endpoint appears.
/** Upper bound for one request, including reading the response body. */
export const REQUEST_TIMEOUT_MS = 60_000;

/** Display labels of request-validation location roots, which also identify validation entries. */
const VALIDATION_LOCATION_LABELS = new Map([
  ['body', '请求体'],
  ['path', '路径参数'],
  ['query', '查询参数'],
  ['header', '请求头'],
  ['cookie', 'Cookie'],
]);
const VALIDATION_LOCATION_PATTERN = /^([a-z]+)(?:\.(.+)|(\[.+))?$/;
/** Validation entries listed in one message before the rest are summarized by count. */
const MAX_LISTED_VALIDATION_ERRORS = 3;

const RETRY_AFTER_STATUSES = new Set([429, 503]);

/** Transport that performs one request. */
export type RequestTransport = (url: string, init: RequestInit) => Promise<Response>;

/**
 * Resolve the backend base URL from the active client platform.
 *
 * @returns Absolute backend URL.
 */
export function resolveApiBase(): string {
  return CLIENT_PLATFORM.serverOrigin();
}

/**
 * Read the selected index database from local storage.
 *
 * @returns Selected database name.
 */
export function readSelectedDatabase(): string {
  const selectedDatabase = readLocalStorageValue(SELECTED_DATABASE_KEY);
  if (selectedDatabase) {
    return selectedDatabase;
  }
  const legacySelectedDatabase = readLocalStorageValue(LEGACY_SELECTED_DATABASE_KEY);
  if (legacySelectedDatabase) {
    storeSelectedDatabase(legacySelectedDatabase);
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
  writeLocalStorageValue(SELECTED_DATABASE_KEY, dbName);
  removeLocalStorageValue(LEGACY_SELECTED_DATABASE_KEY);
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
 * Parse a Retry-After delay expressed as seconds or an HTTP date.
 *
 * @param value - Retry-After response header.
 * @returns Non-negative whole seconds or null when invalid.
 */
function parseRetryAfterSeconds(value: string | null): number | null {
  const normalized = value?.trim();
  if (!normalized) {
    return null;
  }
  if (/^\d+$/.test(normalized)) {
    const seconds = Number(normalized);
    return Number.isSafeInteger(seconds) ? seconds : null;
  }
  const retryAt = Date.parse(normalized);
  if (Number.isNaN(retryAt)) {
    return null;
  }
  return Math.max(0, Math.ceil((retryAt - Date.now()) / 1000));
}

/**
 * Describe one request-validation entry as its field name and server message.
 *
 * @param detail - Problem error entry.
 * @returns Field description, or null when the location is not a request-validation path.
 */
function describeValidationError(detail: ProblemErrorDetail): string | null {
  const match = VALIDATION_LOCATION_PATTERN.exec(detail.location ?? '');
  const rootLabel = match ? VALIDATION_LOCATION_LABELS.get(match[1]) : undefined;
  if (!match || !rootLabel) {
    return null;
  }
  const field = match[2] ?? match[3] ?? rootLabel;
  const message = detail.message?.trim();
  return message ? `${field}（${message}）` : field;
}

/**
 * Summarize request-validation entries as one Chinese message naming the invalid fields.
 *
 * Classified business entries such as `code` or `phase` are ignored, so their problems keep
 * the server detail.
 *
 * @param errors - Problem error entries.
 * @returns Validation summary, or null when no entry refers to a request location.
 */
function formatValidationMessage(errors: readonly ProblemErrorDetail[]): string | null {
  const fields = errors.flatMap((detail) => describeValidationError(detail) ?? []);
  if (fields.length === 0) {
    return null;
  }
  const listed = fields.slice(0, MAX_LISTED_VALIDATION_ERRORS).join('；');
  const overflow =
    fields.length > MAX_LISTED_VALIDATION_ERRORS ? `；…（共 ${fields.length} 项）` : '';
  return `请求参数无效：${listed}${overflow}`;
}

/**
 * Build one typed API error from a failed response and its RFC 9457 problem details.
 *
 * Request-validation failures name their invalid fields; other problems show their detail.
 * Retrying is worthwhile only for 429 and 503 responses that state a Retry-After delay.
 *
 * @param response - Failed fetch response.
 * @param fallback - Display message when the problem carries no detail.
 * @returns Typed API error with request correlation metadata.
 */
async function createApiError(response: Response, fallback: string): Promise<ApiError> {
  const payload: unknown = await response.json().catch(() => null);
  const problem = PROBLEM_DETAILS_SCHEMA.catch({}).parse(payload);
  const retryAfterSeconds = parseRetryAfterSeconds(response.headers.get('Retry-After'));
  const message = formatValidationMessage(problem.errors ?? []) ?? (problem.detail || fallback);
  return new ApiError(message, response.status, {
    type: problem.type,
    title: problem.title,
    instance: problem.instance,
    errors: problem.errors,
    requestId: response.headers.get(REQUEST_ID_HEADER),
    retryable: RETRY_AFTER_STATUSES.has(response.status) && retryAfterSeconds !== null,
    retryAfterSeconds,
  });
}

/**
 * Build request options with the shared JSON content type and caller headers.
 *
 * @param init - Caller request options.
 * @returns Request options passed to the platform transport.
 */
function buildRequestInit(init?: RequestInit): RequestInit {
  const headers = new Headers(init?.headers);
  if (init?.body !== undefined && !headers.has('Content-Type')) {
    headers.set('Content-Type', 'application/json');
  }
  return { ...init, headers };
}

/**
 * Create the typed error raised when one request exceeds its time budget.
 *
 * @returns Retryable timeout API error.
 */
function createTimeoutError(): ApiError {
  return new ApiError('请求超时，请稍后重试', 0, { type: REQUEST_TIMEOUT_TYPE, retryable: true });
}

/**
 * Send one request through a transport with a timeout that also covers reading the response
 * body, propagating caller cancellation unchanged. Retries belong to TanStack Query.
 *
 * @param transport - Transport that performs the request.
 * @param url - Absolute endpoint URL.
 * @param init - Caller request options, including an optional cancellation signal.
 * @param consume - Response reader that returns the result or throws a typed API error.
 * @returns Consumed response result.
 */
async function executeRequest<T>(
  transport: RequestTransport,
  url: string,
  init: RequestInit | undefined,
  consume: (response: Response) => Promise<T>,
): Promise<T> {
  const requestInit = buildRequestInit(init);
  const callerSignal = requestInit.signal;
  const timeoutSignal = AbortSignal.timeout(REQUEST_TIMEOUT_MS);
  const signal = callerSignal ? AbortSignal.any([callerSignal, timeoutSignal]) : timeoutSignal;
  try {
    return await consume(await transport(url, { ...requestInit, signal }));
  } catch (error) {
    if (timeoutSignal.aborted && !callerSignal?.aborted) {
      throw createTimeoutError();
    }
    throw error;
  }
}

/**
 * Parse a safe quoted attachment filename.
 *
 * @param contentDisposition - Content-Disposition header value.
 * @returns Safe ASCII basename or null.
 */
function parseDownloadFilename(contentDisposition: string | null): string | null {
  const match = contentDisposition?.match(/(?:^|;)\s*filename="([A-Za-z0-9._-]{1,255})"(?:;|$)/i);
  return match?.[1] ?? null;
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
  throw await createApiError(response, fallback);
}

/**
 * Send one request through the active client platform.
 *
 * @param url - Absolute endpoint URL.
 * @param init - Request options.
 * @returns Raw server response.
 */
function sendThroughPlatform(url: string, init: RequestInit): Promise<Response> {
  return CLIENT_PLATFORM.send(url, init);
}

/**
 * Fetch JSON from an endpoint through the active client platform.
 *
 * @param url - Absolute endpoint URL.
 * @param init - Fetch options.
 * @param fallback - Fallback error message.
 * @param parser - Optional runtime contract parser for control-plane responses.
 * @returns Parsed response body.
 */
export async function requestJson<T>(
  url: string,
  init?: RequestInit,
  fallback = '请求失败',
  parser?: ContractParser<T>,
): Promise<T> {
  return requestJsonVia(sendThroughPlatform, url, init, fallback, parser);
}

/**
 * Fetch JSON through a specific transport with the shared timeout, retry, and error handling.
 *
 * @param transport - Transport that performs each attempt, such as a native session call.
 * @param url - Absolute endpoint URL the transport targets.
 * @param init - Request options.
 * @param fallback - Fallback error message.
 * @param parser - Optional runtime contract parser for control-plane responses.
 * @returns Parsed response body.
 */
export async function requestJsonVia<T>(
  transport: RequestTransport,
  url: string,
  init?: RequestInit,
  fallback = '请求失败',
  parser?: ContractParser<T>,
): Promise<T> {
  return executeRequest(transport, url, init, (response) =>
    parseJson<T>(response, fallback, parser),
  );
}

/**
 * Fetch a download while reusing the shared API error parser.
 *
 * @param url - Absolute endpoint URL.
 * @param init - Fetch options.
 * @param fallback - Fallback error message.
 * @returns Download Blob and optional safe server filename.
 */
export async function requestDownload(
  url: string,
  init?: RequestInit,
  fallback = '下载失败',
): Promise<ApiDownload> {
  return executeRequest(sendThroughPlatform, url, init, async (response) => {
    if (!response.ok) {
      throw await createApiError(response, fallback);
    }
    return {
      blob: await response.blob(),
      filename: parseDownloadFilename(response.headers.get('Content-Disposition')),
    };
  });
}
