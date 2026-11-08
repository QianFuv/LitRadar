/**
 * Platform API transport timeout, cancellation, bounded retry, and error-envelope coverage.
 */

import { afterEach, beforeEach, describe, expect, test, vi } from 'vitest';

import {
  ApiError,
  MAX_IDEMPOTENT_RETRIES,
  NETWORK_RETRY_BASE_DELAY_MS,
  REQUEST_TIMEOUT_CODE,
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
  expect(error).toMatchObject({ status: 0, code: REQUEST_TIMEOUT_CODE, retryable: true });
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

  expect(await outcome).toMatchObject({ code: REQUEST_TIMEOUT_CODE });
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
    jsonResponse({ detail: 'busy' }, 503, { 'Retry-After': '2' }),
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
    jsonResponse({ detail: 'busy' }, 503, { 'Retry-After': '1' }),
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
      jsonResponse({ detail: 'still busy', code: 'service_unavailable' }, 503, {
        'Retry-After': '0',
      }),
    ),
  );

  const outcome = settle(requestJson(ENDPOINT));
  await vi.runAllTimersAsync();
  const error = await outcome;

  expect(error).toBeInstanceOf(ApiError);
  expect(error).toMatchObject({ status: 503, code: 'service_unavailable', retryAfterSeconds: 0 });
  expect(fetchSpy).toHaveBeenCalledTimes(MAX_IDEMPOTENT_RETRIES + 1);
}

/**
 * Verify non-idempotent requests, long Retry-After delays, and non-retry statuses fail at once.
 */
async function skipsUnsafeRetries(): Promise<void> {
  const postSpy = scriptFetch([jsonResponse({ detail: 'busy' }, 503, { 'Retry-After': '1' })]);
  await expect(requestJson(ENDPOINT, { method: 'POST', body: '{}' })).rejects.toMatchObject({
    status: 503,
    retryAfterSeconds: 1,
  });
  expect(postSpy).toHaveBeenCalledTimes(1);
  postSpy.mockRestore();

  const longDelaySpy = scriptFetch([
    jsonResponse({ detail: 'later' }, 429, { 'Retry-After': '30' }),
  ]);
  await expect(requestJson(ENDPOINT)).rejects.toMatchObject({
    status: 429,
    retryAfterSeconds: 30,
  });
  expect(longDelaySpy).toHaveBeenCalledTimes(1);
  longDelaySpy.mockRestore();

  const gatewaySpy = scriptFetch([jsonResponse({ detail: 'bad gateway' }, 502)]);
  await expect(requestDownload(ENDPOINT)).rejects.toMatchObject({ status: 502 });
  expect(gatewaySpy).toHaveBeenCalledTimes(1);
}

/**
 * Verify structured error envelopes keep their code, phase, message, and request id.
 */
async function preservesErrorEnvelope(): Promise<void> {
  scriptFetch([
    jsonResponse(
      { detail: { code: 'cnki_login_timeout', message: '知网登录超时', phase: 'login' } },
      408,
      { 'X-Request-Id': 'request-42' },
    ),
  ]);

  const error = await settle(requestJson(ENDPOINT, { method: 'POST', body: '{}' }, '请求失败'));

  expect(error).toBeInstanceOf(ApiError);
  expect(error).toMatchObject({
    status: 408,
    code: 'cnki_login_timeout',
    message: '知网登录超时',
    phase: 'login',
    requestId: 'request-42',
    retryable: false,
    retryAfterSeconds: null,
  });
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
  test('preserves the structured error envelope', preservesErrorEnvelope);
});
