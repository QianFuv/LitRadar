/**
 * Browser-local structured logging, listener lifecycle, privacy, and request correlation tests.
 */

import { render } from '@testing-library/react';
import { StrictMode, type ReactNode } from 'react';
import { afterEach, expect, test, vi } from 'vitest';

import RouteError from '@/app/error';
import GlobalError from '@/app/global-error';
import Providers from '@/app/providers';
import { reportClientError, type ClientErrorEvent } from '@/lib/client-logger';
import { ApiError, requestJson } from '@/lib/api/client';

vi.mock('@/lib/auth-context', () => {
  /**
   * Keep the root provider test isolated from authentication network activity.
   *
   * @param props - Mock provider children.
   * @returns Children without an authentication context.
   */
  function MockAuthProvider({ children }: { children: ReactNode }) {
    return <>{children}</>;
  }

  return { AuthProvider: MockAuthProvider };
});

vi.mock('next-themes', () => {
  /**
   * Keep the listener lifecycle test independent of JSDOM media-query support.
   *
   * @param props - Mock provider children.
   * @returns Children without theme behavior.
   */
  function MockThemeProvider({ children }: { children: ReactNode }) {
    return <>{children}</>;
  }

  return { ThemeProvider: MockThemeProvider };
});

vi.mock('nuqs/adapters/next/app', () => {
  /**
   * Keep the listener lifecycle test independent of Next router state.
   *
   * @param props - Mock adapter children.
   * @returns Children without URL-state behavior.
   */
  function MockNuqsAdapter({ children }: { children: ReactNode }) {
    return <>{children}</>;
  }

  return { NuqsAdapter: MockNuqsAdapter };
});

/**
 * Restore the browser route changed by privacy tests.
 */
function resetBrowserLocation(): void {
  window.history.replaceState({}, '', '/');
}

/**
 * Count listener spy calls for one DOM event type without inspecting callbacks.
 *
 * @param calls - Listener spy calls.
 * @param eventType - DOM event type.
 * @returns Number of matching calls.
 */
function countListenerCalls(calls: readonly unknown[][], eventType: string): number {
  let count = 0;
  for (const call of calls) {
    if (call[0] === eventType) {
      count += 1;
    }
  }
  return count;
}

/**
 * Select structured client error payloads from console calls while ignoring framework warnings.
 *
 * @param calls - Console spy calls.
 * @returns Client error events in call order.
 */
function clientErrorPayloads(calls: readonly unknown[][]): ClientErrorEvent[] {
  const events: ClientErrorEvent[] = [];
  for (const call of calls) {
    const value = call[0];
    if (
      typeof value === 'object' &&
      value !== null &&
      Reflect.get(value, 'event') === 'client.error'
    ) {
      events.push(value as ClientErrorEvent);
    }
  }
  return events;
}

/**
 * Verify event fields are allowlisted, bounded, deduplicated, and local-only.
 */
function emitsAllowlistedLocalEvents(): void {
  const messageSentinel = 'message-secret-never-log';
  const stackSentinel = 'stack-secret-never-log';
  const querySentinel = 'query-secret-never-log';
  const storageSentinel = 'storage-secret-never-log';
  window.history.replaceState({}, '', `/safe/path?token=${querySentinel}#fragment-secret`);
  window.localStorage.setItem('private-test-value', storageSentinel);
  const consoleReport = vi.spyOn(console, 'error').mockImplementation(() => undefined);
  const fetchReport = vi.spyOn(globalThis, 'fetch');
  const error = Object.assign(new TypeError(messageSentinel), {
    digest: 'digest-123',
    requestId: 'request-123',
    stack: stackSentinel,
  });

  expect(reportClientError('global_boundary', error)).toBe(true);
  expect(reportClientError('global_boundary', error)).toBe(false);

  const events = clientErrorPayloads(consoleReport.mock.calls);
  expect(events).toHaveLength(1);
  expect(events[0]).toEqual({
    component: 'browser',
    digest: 'digest-123',
    error_kind: 'type_error',
    event: 'client.error',
    level: 'error',
    request_id: 'request-123',
    route: '/safe/path',
    source: 'global_boundary',
    timestamp: expect.any(String),
  });
  const serialized = JSON.stringify(events[0]);
  expect(serialized).not.toContain(messageSentinel);
  expect(serialized).not.toContain(stackSentinel);
  expect(serialized).not.toContain(querySentinel);
  expect(serialized).not.toContain('fragment-secret');
  expect(serialized).not.toContain(storageSentinel);
  expect(fetchReport).not.toHaveBeenCalled();
}

/**
 * Verify failed API responses preserve only the exposed request identifier for correlation.
 */
async function capturesFailedResponseRequestIds(): Promise<void> {
  const bodySentinel = 'request-body-secret-never-log';
  const detailSentinel = 'response-detail-secret-never-log';
  const querySentinel = 'request-query-secret-never-log';
  const requestId = 'request-correlation-456';
  vi.spyOn(globalThis, 'fetch')
    .mockResolvedValueOnce(
      new Response(JSON.stringify({ detail: detailSentinel }), {
        status: 500,
        headers: {
          'Content-Type': 'application/json',
          'X-Request-Id': requestId,
        },
      }),
    )
    .mockResolvedValueOnce(
      new Response(JSON.stringify({ detail: 'second failure' }), {
        status: 400,
        headers: { 'Content-Type': 'application/json' },
      }),
    );

  let correlatedError: unknown;
  try {
    await requestJson(
      `http://remote.example/failure?token=${querySentinel}`,
      'authorization-token-never-log',
      { method: 'POST', body: JSON.stringify({ secret: bodySentinel }) },
    );
  } catch (error) {
    correlatedError = error;
  }
  expect(correlatedError).toBeInstanceOf(ApiError);
  if (!(correlatedError instanceof ApiError)) {
    throw new TypeError('expected ApiError fixture');
  }
  expect(correlatedError.requestId).toBe(requestId);

  const consoleReport = vi.spyOn(console, 'error').mockImplementation(() => undefined);
  reportClientError('window_error', correlatedError);
  const [event] = clientErrorPayloads(consoleReport.mock.calls);
  expect(event).toMatchObject({
    error_kind: 'api_error',
    request_id: requestId,
    source: 'window_error',
  });
  const serialized = JSON.stringify(event);
  expect(serialized).not.toContain(detailSentinel);
  expect(serialized).not.toContain(bodySentinel);
  expect(serialized).not.toContain(querySentinel);
  expect(serialized).not.toContain('authorization-token-never-log');

  let uncorrelatedError: unknown;
  try {
    await requestJson('http://remote.example/failure-without-id');
  } catch (error) {
    uncorrelatedError = error;
  }
  expect(uncorrelatedError).toBeInstanceOf(ApiError);
  if (!(uncorrelatedError instanceof ApiError)) {
    throw new TypeError('expected ApiError fixture without request id');
  }
  expect(uncorrelatedError.requestId).toBeNull();
}

/**
 * Verify Strict Mode leaves one listener pair mounted, removes it deterministically, and
 * normalizes window and promise payloads without duplicate reports.
 */
function managesGlobalListenerLifecycle(): void {
  const addListener = vi.spyOn(window, 'addEventListener');
  const removeListener = vi.spyOn(window, 'removeEventListener');
  const consoleReport = vi.spyOn(console, 'error').mockImplementation(() => undefined);
  const view = render(
    <StrictMode>
      <Providers>
        <div>fixture</div>
      </Providers>
    </StrictMode>,
  );

  const errorAdds = countListenerCalls(addListener.mock.calls, 'error');
  const errorRemoves = countListenerCalls(removeListener.mock.calls, 'error');
  const rejectionAdds = countListenerCalls(addListener.mock.calls, 'unhandledrejection');
  const rejectionRemoves = countListenerCalls(removeListener.mock.calls, 'unhandledrejection');
  expect(errorAdds - errorRemoves).toBe(1);
  expect(rejectionAdds - rejectionRemoves).toBe(1);

  const windowError = new TypeError('window-message-secret-never-log');
  window.dispatchEvent(new ErrorEvent('error', { error: windowError }));
  window.dispatchEvent(new ErrorEvent('error', { error: windowError }));
  const rejection = new Event('unhandledrejection');
  Object.defineProperty(rejection, 'reason', {
    value: { payload: 'promise-reason-secret-never-log' },
  });
  window.dispatchEvent(rejection);

  const events = clientErrorPayloads(consoleReport.mock.calls);
  expect(events).toHaveLength(2);
  expect(events[0]).toMatchObject({
    error_kind: 'type_error',
    source: 'window_error',
  });
  expect(events[1]).toMatchObject({
    error_kind: 'non_error_rejection',
    source: 'unhandled_rejection',
  });
  expect(JSON.stringify(events)).not.toContain('window-message-secret-never-log');
  expect(JSON.stringify(events)).not.toContain('promise-reason-secret-never-log');

  view.unmount();
  expect(countListenerCalls(addListener.mock.calls, 'error')).toBe(
    countListenerCalls(removeListener.mock.calls, 'error'),
  );
  expect(countListenerCalls(addListener.mock.calls, 'unhandledrejection')).toBe(
    countListenerCalls(removeListener.mock.calls, 'unhandledrejection'),
  );
}

/**
 * Verify a route boundary emits once when React Strict Mode replays its effect.
 */
function deduplicatesRouteBoundaryEffects(): void {
  const consoleReport = vi.spyOn(console, 'error').mockImplementation(() => undefined);
  const error = Object.assign(new Error('route-message-secret-never-log'), {
    digest: 'route-digest-789',
  });

  render(
    <StrictMode>
      <RouteError error={error} reset={vi.fn()} />
    </StrictMode>,
  );

  const events = clientErrorPayloads(consoleReport.mock.calls);
  expect(events).toHaveLength(1);
  expect(events[0]).toMatchObject({
    digest: 'route-digest-789',
    error_kind: 'error',
    source: 'route_boundary',
  });
  expect(JSON.stringify(events[0])).not.toContain('route-message-secret-never-log');
}

/**
 * Verify the self-contained global boundary uses the same safe reporting contract.
 */
function reportsGlobalBoundaryEffects(): void {
  const consoleReport = vi.spyOn(console, 'error').mockImplementation(() => undefined);
  const error = Object.assign(new Error('global-message-secret-never-log'), {
    digest: 'global-digest-987',
  });

  render(<GlobalError error={error} reset={vi.fn()} />);

  const events = clientErrorPayloads(consoleReport.mock.calls);
  expect(events).toHaveLength(1);
  expect(events[0]).toMatchObject({
    digest: 'global-digest-987',
    error_kind: 'error',
    source: 'global_boundary',
  });
  expect(JSON.stringify(events[0])).not.toContain('global-message-secret-never-log');
}

afterEach(resetBrowserLocation);
test('emits allowlisted deduplicated local events', emitsAllowlistedLocalEvents);
test('captures exposed failed-response request IDs', capturesFailedResponseRequestIds);
test('manages Strict Mode listener lifecycle', managesGlobalListenerLifecycle);
test('deduplicates Strict Mode route boundary effects', deduplicatesRouteBoundaryEffects);
test('reports global boundary effects safely', reportsGlobalBoundaryEffects);
