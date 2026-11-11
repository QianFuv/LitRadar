/**
 * Application QueryClient and the single retry policy for API reads.
 *
 * The transport performs one attempt per request; TanStack Query owns every retry so a failed
 * read is never retried by two layers at once.
 */

import { QueryClient } from '@tanstack/react-query';

import { ApiError } from '@/lib/api/client';

/** Additional attempts allowed after a failed query. */
export const MAX_QUERY_RETRIES = 2;
/** First backoff delay after a network failure; doubles for each later attempt. */
export const NETWORK_RETRY_BASE_DELAY_MS = 300;
/** Longest server-requested Retry-After delay a query waits for before giving up. */
const MAX_RETRY_AFTER_MS = 5_000;

/**
 * Read the server-requested delay a failed query may wait for.
 *
 * @param error - Query failure.
 * @returns Delay in milliseconds, or null when the error carries no acceptable Retry-After.
 */
function readHonoredRetryAfterMs(error: unknown): number | null {
  if (!(error instanceof ApiError) || !error.retryable || error.retryAfterSeconds === null) {
    return null;
  }
  const delayMs = error.retryAfterSeconds * 1000;
  return delayMs <= MAX_RETRY_AFTER_MS ? delayMs : null;
}

/**
 * Decide whether a failed query is retried: only transport failures, where `fetch` and the
 * native bridge reject with a `TypeError`, and 429/503 responses with a short Retry-After.
 *
 * @param failureCount - Failures so far, starting at zero for the first failure.
 * @param error - Query failure.
 * @returns Whether TanStack Query starts another attempt.
 */
function shouldRetryQuery(failureCount: number, error: unknown): boolean {
  if (failureCount >= MAX_QUERY_RETRIES) {
    return false;
  }
  return error instanceof TypeError || readHonoredRetryAfterMs(error) !== null;
}

/**
 * Compute the wait before the next query attempt: the honored Retry-After, otherwise an
 * exponential network backoff.
 *
 * @param failureCount - Failures so far, starting at zero for the first failure.
 * @param error - Query failure.
 * @returns Delay in milliseconds.
 */
function getQueryRetryDelay(failureCount: number, error: unknown): number {
  return readHonoredRetryAfterMs(error) ?? NETWORK_RETRY_BASE_DELAY_MS * 2 ** failureCount;
}

/**
 * Create the application QueryClient with the shared retry policy.
 *
 * @returns QueryClient for the application root.
 */
export function createQueryClient(): QueryClient {
  return new QueryClient({
    defaultOptions: {
      queries: { retry: shouldRetryQuery, retryDelay: getQueryRetryDelay },
    },
  });
}
