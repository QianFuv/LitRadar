/**
 * Platform API transport timeout, cancellation, single-attempt, and problem-details coverage, plus
 * the QueryClient retry policy that owns every retry.
 */

import { afterEach, beforeEach, describe, expect, test, vi } from 'vite-plus/test';

import {
  ApiError,
  REQUEST_TIMEOUT_TYPE,
  REQUEST_TIMEOUT_MS,
  apiClient,
  readData,
} from '@/lib/api/client';
import {
  createQueryClient,
  MAX_QUERY_RETRIES,
  NETWORK_RETRY_BASE_DELAY_MS,
} from '@/lib/query-client';

/**
 * Read the database list, an idempotent JSON endpoint.
 *
 * @param signal - Optional caller cancellation signal.
 * @returns Database names.
 */
async function readDatabases(signal?: AbortSignal): Promise<string[]> {
  return readData(await apiClient.GET('/api/meta/databases', { signal }));
}

/**
 * Create a folder, a JSON endpoint with a request body.
 *
 * @returns Created folder.
 */
async function createFolder(): Promise<unknown> {
  return readData(
    await apiClient.POST('/api/favorites/folders', { body: { name: 'f', is_tracking: false } }),
  );
}

/**
 * Download a folder export, a binary endpoint.
 *
 * @returns Export blob.
 */
async function downloadExport(): Promise<Blob> {
  return readData(
    await apiClient.GET('/api/favorites/folders/{folder_id}/export', {
      params: { path: { folder_id: 1 }, query: { format: 'ris' } },
      parseAs: 'blob',
    }),
  );
}

type FetchStep = Response | Error | 'hang';

/**
 * Create a JSON response fixture.
 *
 * @param body - JSON body.
 * @param status - HTTP status.
 * @param headers - Extra response headers.
 * @returns Response fixture.
 */
function jsonResponse(body: unknown, status = 200, headers: Record<string, string> = {}): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'Content-Type': 'application/json', ...headers },
  });
}

/**
 * Create an RFC 9457 problem response fixture in the backend's default error format.
 *
 * @param status - HTTP status.
 * @param detail - Problem detail message.
 * @param headers - Extra response headers.
 * @param extras - Extra problem members, such as `errors`.
 * @returns Problem response fixture.
 */
function problemResponse(
  status: number,
  detail: string,
  headers: Record<string, string> = {},
  extras: Record<string, unknown> = {},
): Response {
  return new Response(JSON.stringify({ title: 'Problem', status, detail, ...extras }), {
    status,
    headers: { 'Content-Type': 'application/problem+json', ...headers },
  });
}

/**
 * Replace fetch with a scripted sequence that honors abort signals like the browser does.
 *
 * @param steps - Response, thrown error, or a request that never settles on its own, per call;
 *   the last step repeats, so a repeated Response must be listed once per expected read.
 * @returns Fetch spy.
 */
function scriptFetch(steps: FetchStep[]) {
  let index = 0;
  return vi.spyOn(globalThis, 'fetch').mockImplementation((_input, init) => {
    const step = steps[Math.min(index, steps.length - 1)];
    index += 1;
    const signal = init?.signal;
    return new Promise<Response>((resolve, reject) => {
      if (signal?.aborted) {
        reject(signal.reason);
        return;
      }
      signal?.addEventListener('abort', () => reject(signal.reason), { once: true });
      if (step === 'hang') {
        return;
      }
      if (step instanceof Error) {
        reject(step);
        return;
      }
      resolve(step);
    });
  });
}

/**
 * Capture a promise rejection without leaving it unhandled while timers advance.
 *
 * @param promise - Request promise.
 * @returns Promise resolving to the rejection reason, or the resolved value.
 */
function settle(promise: Promise<unknown>): Promise<unknown> {
  return promise.then(
    (value) => value,
    (error: unknown) => error,
  );
}

/**
 * Verify one stalled attempt becomes a retryable timeout ApiError without being retried.
 */
async function timesOutStalledRequests(): Promise<void> {
  const fetchSpy = scriptFetch(['hang']);
  const outcome = settle(readDatabases());

  await vi.advanceTimersByTimeAsync(REQUEST_TIMEOUT_MS - 1);
  expect(fetchSpy).toHaveBeenCalledTimes(1);
  await vi.advanceTimersByTimeAsync(1);

  const error = await outcome;
  expect(error).toBeInstanceOf(ApiError);
  expect(error).toMatchObject({ status: 0, type: REQUEST_TIMEOUT_TYPE, retryable: true });
  expect(fetchSpy).toHaveBeenCalledTimes(1);
}

/**
 * Verify the timeout also bounds reading a response body that stops streaming.
 */
async function timesOutStalledBodies(): Promise<void> {
  vi.spyOn(globalThis, 'fetch').mockImplementation((_input, init) => {
    const stalledBody = new ReadableStream<Uint8Array>({
      start(controller) {
        init?.signal?.addEventListener('abort', () => controller.error(init.signal?.reason), {
          once: true,
        });
      },
    });
    return Promise.resolve(new Response(stalledBody, { status: 200 }));
  });
  const outcome = settle(createFolder());

  await vi.advanceTimersByTimeAsync(REQUEST_TIMEOUT_MS);

  expect(await outcome).toMatchObject({ type: REQUEST_TIMEOUT_TYPE });
}

/**
 * Verify caller cancellation propagates its abort reason instead of a timeout or ApiError.
 */
async function propagatesCallerCancellation(): Promise<void> {
  const fetchSpy = scriptFetch(['hang']);
  const controller = new AbortController();
  const outcome = settle(readDatabases(controller.signal));
  await vi.advanceTimersByTimeAsync(0);

  controller.abort();
  const error = await outcome;

  expect(error).not.toBeInstanceOf(ApiError);
  expect(error).toMatchObject({ name: 'AbortError' });
  expect(fetchSpy).toHaveBeenCalledTimes(1);
  const forwardedSignal = fetchSpy.mock.calls[0][1]?.signal;
  expect(forwardedSignal?.aborted).toBe(true);
}

/**
 * Fetch one endpoint through a QueryClient with the application retry policy.
 *
 * @returns Query promise.
 */
function fetchWithRetryPolicy(): Promise<string[]> {
  return createQueryClient().fetchQuery({
    queryKey: ['transport-test'],
    queryFn: ({ signal }) => readDatabases(signal),
  });
}

/**
 * Verify the transport itself never retries, whatever the method or failure.
 */
async function performsOneAttempt(): Promise<void> {
  const busySpy = scriptFetch([problemResponse(503, 'busy', { 'Retry-After': '1' })]);
  await expect(readDatabases()).rejects.toMatchObject({
    status: 503,
    retryable: true,
    retryAfterSeconds: 1,
  });
  expect(busySpy).toHaveBeenCalledTimes(1);
  busySpy.mockRestore();

  const networkSpy = scriptFetch([new TypeError('Failed to fetch')]);
  await expect(readDatabases()).rejects.toBeInstanceOf(TypeError);
  expect(networkSpy).toHaveBeenCalledTimes(1);
  networkSpy.mockRestore();

  const gatewaySpy = scriptFetch([problemResponse(502, 'bad gateway')]);
  await expect(downloadExport()).rejects.toMatchObject({ status: 502 });
  expect(gatewaySpy).toHaveBeenCalledTimes(1);
}

/**
 * Verify queries wait exactly for an honored Retry-After before retrying.
 */
async function honorsRetryAfterForQueries(): Promise<void> {
  const fetchSpy = scriptFetch([
    problemResponse(503, 'busy', { 'Retry-After': '1' }),
    jsonResponse(['journals.sqlite']),
  ]);
  const outcome = fetchWithRetryPolicy();

  await vi.advanceTimersByTimeAsync(999);
  expect(fetchSpy).toHaveBeenCalledTimes(1);
  await vi.advanceTimersByTimeAsync(1);

  await expect(outcome).resolves.toEqual(['journals.sqlite']);
  expect(fetchSpy).toHaveBeenCalledTimes(2);
  expect(fetchSpy.mock.calls[1][1]).toMatchObject({ credentials: 'include' });
}

/**
 * Verify query network failures retry with backoff and the attempt count stays bounded.
 */
async function boundsNetworkRetries(): Promise<void> {
  const fetchSpy = scriptFetch([new TypeError('Failed to fetch')]);
  const outcome = settle(fetchWithRetryPolicy());

  await vi.advanceTimersByTimeAsync(NETWORK_RETRY_BASE_DELAY_MS);
  expect(fetchSpy).toHaveBeenCalledTimes(2);
  await vi.advanceTimersByTimeAsync(NETWORK_RETRY_BASE_DELAY_MS * 2);

  expect(await outcome).toBeInstanceOf(TypeError);
  expect(fetchSpy).toHaveBeenCalledTimes(MAX_QUERY_RETRIES + 1);
}

/**
 * Verify the last honored Retry-After failure surfaces as the typed API error.
 */
async function surfacesExhaustedRetryAfter(): Promise<void> {
  const fetchSpy = scriptFetch(
    Array.from({ length: MAX_QUERY_RETRIES + 1 }, () =>
      problemResponse(503, 'still busy', { 'Retry-After': '0' }),
    ),
  );

  const outcome = settle(fetchWithRetryPolicy());
  await vi.runAllTimersAsync();
  const error = await outcome;

  expect(error).toBeInstanceOf(ApiError);
  expect(error).toMatchObject({
    status: 503,
    message: 'still busy',
    retryable: true,
    retryAfterSeconds: 0,
  });
  expect(fetchSpy).toHaveBeenCalledTimes(MAX_QUERY_RETRIES + 1);
}

/**
 * Verify queries do not retry long Retry-After delays, timeouts, or client and server errors.
 */
async function skipsUnhelpfulQueryRetries(): Promise<void> {
  const cases: FetchStep[][] = [
    [problemResponse(429, 'later', { 'Retry-After': '30' })],
    [problemResponse(404, 'missing')],
    [problemResponse(401, 'signed out')],
    [problemResponse(502, 'bad gateway')],
    ['hang'],
  ];
  for (const steps of cases) {
    const fetchSpy = scriptFetch(steps);
    const outcome = settle(fetchWithRetryPolicy());
    await vi.advanceTimersByTimeAsync(REQUEST_TIMEOUT_MS);
    await vi.runAllTimersAsync();
    expect(await outcome).toBeInstanceOf(ApiError);
    expect(fetchSpy).toHaveBeenCalledTimes(1);
    fetchSpy.mockRestore();
  }
}

/**
 * Verify problem details keep their detail, type, classified entries, and request id.
 */
async function preservesProblemDetails(): Promise<void> {
  scriptFetch([
    problemResponse(
      408,
      '知网登录超时',
      { 'X-Request-Id': 'request-42' },
      {
        type: 'about:blank',
        title: 'Request Timeout',
        instance: '/api/cnki/login/confirm',
        errors: [
          { location: 'code', message: 'cnki_login_timeout' },
          { location: 'phase', message: 'login' },
        ],
      },
    ),
  ]);

  const error = await settle(createFolder());

  expect(error).toBeInstanceOf(ApiError);
  expect(error).toMatchObject({
    status: 408,
    message: '知网登录超时',
    type: 'about:blank',
    title: 'Request Timeout',
    instance: '/api/cnki/login/confirm',
    requestId: 'request-42',
    retryable: false,
    retryAfterSeconds: null,
  });
  expect(error instanceof ApiError && error.findErrorDetail('code')).toBe('cnki_login_timeout');
  expect(error instanceof ApiError && error.findErrorDetail('phase')).toBe('login');
  expect(error instanceof ApiError && error.findErrorDetail('field')).toBeNull();
}

/**
 * Verify request-validation problems name each invalid field instead of the English detail.
 */
async function namesInvalidFields(): Promise<void> {
  scriptFetch([
    problemResponse(
      422,
      'validation failed',
      {},
      {
        errors: [
          { location: 'body.ttl', message: 'expected integer', value: 'nope' },
          { location: 'path.article_id', message: 'expected number >= 1' },
          { location: 'body', message: '' },
        ],
      },
    ),
  ]);

  const error = await settle(createFolder());

  expect(error).toBeInstanceOf(ApiError);
  expect(error).toMatchObject({
    status: 422,
    message: '请求参数无效：ttl（expected integer）；article_id（expected number >= 1）；请求体',
  });
  expect(error instanceof ApiError && error.findErrorDetail('body.ttl')).toBe('expected integer');
}

/**
 * Verify long validation lists keep the first entries and summarize the rest by count.
 */
async function truncatesManyInvalidFields(): Promise<void> {
  const fieldNames = ['alpha', 'beta', 'gamma', 'delta', 'epsilon'];
  scriptFetch([
    problemResponse(
      422,
      'validation failed',
      {},
      {
        errors: fieldNames.map((name) => ({ location: `query.${name}`, message: 'invalid' })),
      },
    ),
  ]);

  const error = await settle(readDatabases());

  expect(error).toMatchObject({
    message: '请求参数无效：alpha（invalid）；beta（invalid）；gamma（invalid）；…（共 5 项）',
  });
}

/**
 * Verify classified business entries and entry-free problems keep the server detail.
 */
async function keepsDetailWithoutValidationEntries(): Promise<void> {
  const businessSpy = scriptFetch([
    problemResponse(
      422,
      '会话已撤销',
      {},
      {
        errors: [
          { location: 'action', message: 'revoke_session' },
          { location: 'request_id', message: 'request-7' },
        ],
      },
    ),
  ]);
  const businessError = await settle(createFolder());
  expect(businessError).toMatchObject({ status: 422, message: '会话已撤销' });
  expect(businessError instanceof ApiError && businessError.findErrorDetail('action')).toBe(
    'revoke_session',
  );
  businessSpy.mockRestore();

  const detailSpy = scriptFetch([problemResponse(422, 'Announcement limit reached')]);
  await expect(createFolder()).rejects.toMatchObject({
    status: 422,
    message: 'Announcement limit reached',
    errors: [],
  });
  detailSpy.mockRestore();
}

/**
 * Verify only 429 and 503 responses that state Retry-After are classified as retryable.
 */
async function classifiesRetryableStatuses(): Promise<void> {
  const cases: [Response, boolean][] = [
    [problemResponse(503, 'busy'), false],
    [problemResponse(500, 'failed', { 'Retry-After': '1' }), false],
    [problemResponse(429, 'slow down', { 'Retry-After': 'invalid' }), false],
    [problemResponse(429, 'slow down', { 'Retry-After': '120' }), true],
  ];
  for (const [response, isRetryable] of cases) {
    const fetchSpy = scriptFetch([response]);
    const error = await settle(createFolder());
    expect(error).toMatchObject({ status: response.status, retryable: isRetryable });
    fetchSpy.mockRestore();
  }
}

/**
 * Verify malformed or missing problem bodies fall back without losing valid members.
 */
async function toleratesMalformedProblems(): Promise<void> {
  const textSpy = scriptFetch([new Response('upstream exploded', { status: 500 })]);
  await expect(createFolder()).rejects.toMatchObject({
    status: 500,
    message: '创建收藏夹失败',
    errors: [],
  });
  textSpy.mockRestore();

  const malformedSpy = scriptFetch([
    jsonResponse({ detail: 'kept detail', errors: 'not-a-list', status: 'bad' }, 400),
  ]);
  await expect(createFolder()).rejects.toMatchObject({
    status: 400,
    message: 'kept detail',
    errors: [],
  });
  malformedSpy.mockRestore();

  const legacySpy = scriptFetch([
    jsonResponse({ detail: { code: 'legacy', message: 'legacy' }, retryable: true }, 503),
  ]);
  await expect(createFolder()).rejects.toMatchObject({
    status: 503,
    message: '创建收藏夹失败',
    retryable: false,
  });
  legacySpy.mockRestore();
}

beforeEach(() => {
  vi.useFakeTimers();
  vi.spyOn(AbortSignal, 'timeout').mockImplementation((milliseconds) => {
    const controller = new AbortController();
    setTimeout(
      () => controller.abort(new DOMException('The operation timed out.', 'TimeoutError')),
      milliseconds,
    );
    return controller.signal;
  });
});

afterEach(() => {
  vi.useRealTimers();
});

describe('platform API transport', () => {
  test('times out stalled requests without retrying', timesOutStalledRequests);
  test('times out stalled response bodies', timesOutStalledBodies);
  test('propagates caller cancellation', propagatesCallerCancellation);
  test('performs one attempt per request', performsOneAttempt);
  test('preserves RFC 9457 problem details', preservesProblemDetails);
  test('names invalid fields of validation problems', namesInvalidFields);
  test('truncates long validation field lists', truncatesManyInvalidFields);
  test('keeps the detail without validation entries', keepsDetailWithoutValidationEntries);
  test('classifies retryable statuses by Retry-After', classifiesRetryableStatuses);
  test('tolerates malformed problem bodies', toleratesMalformedProblems);
});

describe('query retry policy', () => {
  test('honors Retry-After for queries', honorsRetryAfterForQueries);
  test('bounds network failure retries', boundsNetworkRetries);
  test('surfaces the final Retry-After failure', surfacesExhaustedRetryAfter);
  test('skips retries that cannot help', skipsUnhelpfulQueryRetries);
});
