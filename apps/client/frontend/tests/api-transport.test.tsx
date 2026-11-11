/**
 * Platform API transport timeout, cancellation, bounded retry, and problem-details coverage.
 */

import { afterEach, beforeEach, describe, expect, test, vi } from 'vite-plus/test';

import {
  ApiError,
  MAX_IDEMPOTENT_RETRIES,
  NETWORK_RETRY_BASE_DELAY_MS,
  REQUEST_TIMEOUT_TYPE,
  REQUEST_TIMEOUT_MS,
  requestDownload,
  requestJson,
} from '@/lib/api/client';

const ENDPOINT = 'http://transport.example/api/resource';

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
  const outcome = settle(requestJson(ENDPOINT));

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
  const outcome = settle(requestJson(ENDPOINT, { method: 'POST', body: '{}' }));

  await vi.advanceTimersByTimeAsync(REQUEST_TIMEOUT_MS);

  expect(await outcome).toMatchObject({ type: REQUEST_TIMEOUT_TYPE });
}

/**
 * Verify caller cancellation propagates its abort reason instead of a timeout or ApiError.
 */
async function propagatesCallerCancellation(): Promise<void> {
  const fetchSpy = scriptFetch(['hang']);
  const controller = new AbortController();
  const outcome = settle(requestJson(ENDPOINT, { signal: controller.signal }));
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
 * Verify cancellation during a Retry-After wait stops further attempts.
 */
async function cancelsDuringRetryWait(): Promise<void> {
  const fetchSpy = scriptFetch([
    problemResponse(503, 'busy', { 'Retry-After': '2' }),
    jsonResponse({ ok: true }),
  ]);
  const controller = new AbortController();
  const outcome = settle(requestJson(ENDPOINT, { signal: controller.signal }));
  await vi.advanceTimersByTimeAsync(1_000);

  controller.abort();
  await vi.advanceTimersByTimeAsync(5_000);

  expect(await outcome).toMatchObject({ name: 'AbortError' });
  expect(fetchSpy).toHaveBeenCalledTimes(1);
}

/**
 * Verify idempotent requests wait exactly for an honored Retry-After before retrying.
 */
async function honorsRetryAfterForIdempotentRequests(): Promise<void> {
  const fetchSpy = scriptFetch([
    problemResponse(503, 'busy', { 'Retry-After': '1' }),
    jsonResponse({ value: 7 }),
  ]);
  const outcome = requestJson<{ value: number }>(ENDPOINT);

  await vi.advanceTimersByTimeAsync(999);
  expect(fetchSpy).toHaveBeenCalledTimes(1);
  await vi.advanceTimersByTimeAsync(1);

  await expect(outcome).resolves.toEqual({ value: 7 });
  expect(fetchSpy).toHaveBeenCalledTimes(2);
  expect(fetchSpy.mock.calls[1][1]).toMatchObject({ credentials: 'include' });
}

/**
 * Verify network failures retry with backoff and the attempt count stays bounded.
 */
async function boundsNetworkRetries(): Promise<void> {
  const fetchSpy = scriptFetch([new TypeError('Failed to fetch')]);
  const outcome = settle(requestJson(ENDPOINT));

  await vi.advanceTimersByTimeAsync(NETWORK_RETRY_BASE_DELAY_MS);
  expect(fetchSpy).toHaveBeenCalledTimes(2);
  await vi.advanceTimersByTimeAsync(NETWORK_RETRY_BASE_DELAY_MS * 2);

  expect(await outcome).toBeInstanceOf(TypeError);
  expect(fetchSpy).toHaveBeenCalledTimes(MAX_IDEMPOTENT_RETRIES + 1);
}

/**
 * Verify the last honored Retry-After failure surfaces as the typed API error.
 */
async function surfacesExhaustedRetryAfter(): Promise<void> {
  const fetchSpy = scriptFetch(
    Array.from({ length: MAX_IDEMPOTENT_RETRIES + 1 }, () =>
      problemResponse(503, 'still busy', { 'Retry-After': '0' }),
    ),
  );

  const outcome = settle(requestJson(ENDPOINT));
  await vi.runAllTimersAsync();
  const error = await outcome;

  expect(error).toBeInstanceOf(ApiError);
  expect(error).toMatchObject({
    status: 503,
    message: 'still busy',
    retryable: true,
    retryAfterSeconds: 0,
  });
  expect(fetchSpy).toHaveBeenCalledTimes(MAX_IDEMPOTENT_RETRIES + 1);
}

/**
 * Verify non-idempotent requests, long Retry-After delays, and non-retry statuses fail at once.
 */
async function skipsUnsafeRetries(): Promise<void> {
  const postSpy = scriptFetch([problemResponse(503, 'busy', { 'Retry-After': '1' })]);
  await expect(requestJson(ENDPOINT, { method: 'POST', body: '{}' })).rejects.toMatchObject({
    status: 503,
    retryable: true,
    retryAfterSeconds: 1,
  });
  expect(postSpy).toHaveBeenCalledTimes(1);
  postSpy.mockRestore();

  const longDelaySpy = scriptFetch([problemResponse(429, 'later', { 'Retry-After': '30' })]);
  await expect(requestJson(ENDPOINT)).rejects.toMatchObject({
    status: 429,
    retryable: true,
    retryAfterSeconds: 30,
  });
  expect(longDelaySpy).toHaveBeenCalledTimes(1);
  longDelaySpy.mockRestore();

  const gatewaySpy = scriptFetch([problemResponse(502, 'bad gateway')]);
  await expect(requestDownload(ENDPOINT)).rejects.toMatchObject({ status: 502 });
  expect(gatewaySpy).toHaveBeenCalledTimes(1);
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

  const error = await settle(requestJson(ENDPOINT, { method: 'POST', body: '{}' }, '请求失败'));

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

  const error = await settle(requestJson(ENDPOINT, { method: 'POST', body: '{}' }));

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

  const error = await settle(requestJson(ENDPOINT));

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
  const businessError = await settle(requestJson(ENDPOINT, { method: 'POST', body: '{}' }));
  expect(businessError).toMatchObject({ status: 422, message: '会话已撤销' });
  expect(businessError instanceof ApiError && businessError.findErrorDetail('action')).toBe(
    'revoke_session',
  );
  businessSpy.mockRestore();

  const detailSpy = scriptFetch([problemResponse(422, 'Announcement limit reached')]);
  await expect(
    requestJson(ENDPOINT, { method: 'POST', body: '{}' }, '保存失败'),
  ).rejects.toMatchObject({ status: 422, message: 'Announcement limit reached', errors: [] });
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
    const error = await settle(requestJson(ENDPOINT, { method: 'POST', body: '{}' }));
    expect(error).toMatchObject({ status: response.status, retryable: isRetryable });
    fetchSpy.mockRestore();
  }
}

/**
 * Verify malformed or missing problem bodies fall back without losing valid members.
 */
async function toleratesMalformedProblems(): Promise<void> {
  const textSpy = scriptFetch([new Response('upstream exploded', { status: 500 })]);
  await expect(
    requestJson(ENDPOINT, { method: 'POST', body: '{}' }, '保存失败'),
  ).rejects.toMatchObject({ status: 500, message: '保存失败', errors: [] });
  textSpy.mockRestore();

  const malformedSpy = scriptFetch([
    jsonResponse({ detail: 'kept detail', errors: 'not-a-list', status: 'bad' }, 400),
  ]);
  await expect(
    requestJson(ENDPOINT, { method: 'POST', body: '{}' }, '保存失败'),
  ).rejects.toMatchObject({ status: 400, message: 'kept detail', errors: [] });
  malformedSpy.mockRestore();

  const legacySpy = scriptFetch([
    jsonResponse({ detail: { code: 'legacy', message: 'legacy' }, retryable: true }, 503),
  ]);
  await expect(
    requestJson(ENDPOINT, { method: 'POST', body: '{}' }, '保存失败'),
  ).rejects.toMatchObject({ status: 503, message: '保存失败', retryable: false });
  legacySpy.mockRestore();
}

beforeEach(() => {
  vi.useFakeTimers();
});

afterEach(() => {
  vi.useRealTimers();
});

describe('platform API transport', () => {
  test('times out stalled requests without retrying', timesOutStalledRequests);
  test('times out stalled response bodies', timesOutStalledBodies);
  test('propagates caller cancellation', propagatesCallerCancellation);
  test('stops retrying when cancelled during a retry wait', cancelsDuringRetryWait);
  test('honors Retry-After for idempotent requests', honorsRetryAfterForIdempotentRequests);
  test('bounds network failure retries', boundsNetworkRetries);
  test('surfaces the final Retry-After failure', surfacesExhaustedRetryAfter);
  test('skips retries that are unsafe or too slow', skipsUnsafeRetries);
  test('preserves RFC 9457 problem details', preservesProblemDetails);
  test('names invalid fields of validation problems', namesInvalidFields);
  test('truncates long validation field lists', truncatesManyInvalidFields);
  test('keeps the detail without validation entries', keepsDetailWithoutValidationEntries);
  test('classifies retryable statuses by Retry-After', classifiesRetryableStatuses);
  test('tolerates malformed problem bodies', toleratesMalformedProblems);
});
