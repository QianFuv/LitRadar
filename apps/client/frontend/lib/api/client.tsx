/**
 * Shared openapi-fetch client, RFC 9457 error parsing, and database-selection helpers.
 */

import createFetchClient, { type Middleware } from 'openapi-fetch';
import createQueryHooks from 'openapi-react-query';
import { z } from 'zod';

import {
  readLocalStorageValue,
  removeLocalStorageValue,
  writeLocalStorageValue,
} from '@/lib/browser-storage';
import { ENDPOINT_FAILURE_MESSAGES } from '@/lib/api/endpoint-messages';
import type { paths } from '@/lib/generated/api-schema';
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
 * Create the typed error raised when one request exceeds its time budget.
 *
 * @returns Retryable timeout API error.
 */
function createTimeoutError(): ApiError {
  return new ApiError('请求超时，请稍后重试', 0, { type: REQUEST_TIMEOUT_TYPE, retryable: true });
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

/** Origin of the URLs openapi-fetch builds; the platform transport swaps in the server origin. */
const API_PLACEHOLDER_ORIGIN = 'http://litradar.api';

/** Statuses whose responses cannot carry a body. */
const NULL_BODY_STATUSES = new Set([101, 103, 204, 205, 304]);

/** Error message used when an endpoint has no specific message and the problem has no detail. */
const DEFAULT_FAILURE_MESSAGE = '请求失败';

/** Transport that sends one request to the LitRadar server, such as the platform `send`. */
export type ServerTransport = (url: string, init: RequestInit) => Promise<Response>;

/**
 * Create the `fetch` used by openapi-fetch: it targets the current server origin, sends through a
 * platform transport, and bounds the request, including reading the body, by one timeout.
 *
 * The body is buffered inside the time budget, so a stalled body becomes a timeout error rather
 * than an unbounded read, and caller cancellation propagates unchanged.
 *
 * @param transport - Transport that performs the request.
 * @returns Fetch function for openapi-fetch.
 */
export function createServerFetch(transport: ServerTransport) {
  return async function serverFetch(request: Request): Promise<Response> {
    const requestUrl = new URL(request.url);
    const url = new URL(`${requestUrl.pathname}${requestUrl.search}`, resolveApiBase()).toString();
    const timeoutSignal = AbortSignal.timeout(REQUEST_TIMEOUT_MS);
    const signal = AbortSignal.any([request.signal, timeoutSignal]);
    const body = request.body === null ? undefined : await request.text();
    try {
      const response = await transport(url, {
        method: request.method,
        headers: request.headers,
        body,
        signal,
      });
      const bytes = NULL_BODY_STATUSES.has(response.status) ? null : await response.arrayBuffer();
      return new Response(bytes, {
        status: response.status,
        statusText: response.statusText,
        headers: response.headers,
      });
    } catch (error) {
      if (timeoutSignal.aborted && !request.signal.aborted) {
        throw createTimeoutError();
      }
      throw error;
    }
  };
}

/** Raise every non-2xx response as an {@link ApiError} with its endpoint's failure message. */
const API_ERROR_MIDDLEWARE: Middleware = {
  async onResponse({ request, response, schemaPath }) {
    if (!response.ok) {
      const fallback =
        ENDPOINT_FAILURE_MESSAGES.get(`${request.method} ${schemaPath}`) ?? DEFAULT_FAILURE_MESSAGE;
      throw await createApiError(response, fallback);
    }
    return undefined;
  },
};

/**
 * Typed LitRadar API client. Every request goes through the active client platform, and every
 * non-2xx response rejects with an {@link ApiError}, so callers only see successful data.
 */
export const apiClient = createFetchClient<paths>({
  baseUrl: API_PLACEHOLDER_ORIGIN,
  fetch: createServerFetch((url, init) => CLIENT_PLATFORM.send(url, init)),
});
apiClient.use(API_ERROR_MIDDLEWARE);

/** Native sign-in endpoints whose request the host builds in Go from its own credentials. */
type NativeSessionPath = '/api/auth/native/login' | '/api/auth/oidc/native/exchange';

/** Native sign-in operations as JavaScript sees them: no request body, the server's response. */
type NativeSessionPaths = {
  [Path in NativeSessionPath]: Omit<paths[Path], 'post'> & {
    post: Omit<NonNullable<paths[Path]['post']>, 'requestBody'> & { requestBody?: never };
  };
};

/**
 * Client for native sign-in calls. Each call passes a `fetch` that runs the host bridge, which
 * sends the real request; responses still get the shared timeout and {@link ApiError} handling.
 */
export const nativeSessionClient = createFetchClient<NativeSessionPaths>({
  baseUrl: API_PLACEHOLDER_ORIGIN,
  fetch: createServerFetch((url, init) => CLIENT_PLATFORM.send(url, init)),
});
nativeSessionClient.use(API_ERROR_MIDDLEWARE);

/** TanStack Query bindings for {@link apiClient}; query keys are `[method, path, init]`. */
export const $api = createQueryHooks(apiClient);

/**
 * Read the body of a successful response, which the error middleware guarantees.
 *
 * @param result - openapi-fetch result.
 * @returns Response data.
 */
export function readData<T>(result: { data?: T }): T {
  if (result.data === undefined) {
    throw new ApiError(DEFAULT_FAILURE_MESSAGE, 0);
  }
  return result.data;
}

/**
 * Read a safe quoted attachment filename from a download response.
 *
 * @param response - Download response.
 * @returns Safe ASCII basename or null.
 */
export function readDownloadFilename(response: Response): string | null {
  return parseDownloadFilename(response.headers.get('Content-Disposition'));
}
