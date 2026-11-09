/**
 * Native platform coverage: Wails bridge transport, cancellation, credential calls, downloads,
 * external links, and server setup errors, with the Wails runtime replaced by a fake.
 */

import { afterEach, beforeEach, describe, expect, test, vi } from 'vitest';

import { ApiError } from '@/lib/api/client';
import { importAccessToken, loginUser, logoutUser } from '@/lib/api/auth';
import { getAuthErrorMessage } from '@/lib/auth-error';
import { CLIENT_PLATFORM } from '@/lib/platform';
import { PlatformBridgeError } from '@/lib/platform/errors';
import {
  BRIDGE_SERVICE,
  NATIVE_PLATFORM,
  decodeBase64,
  encodeBase64,
  handleExternalNavigation,
} from '@/lib/platform/native';
import { getServerConnectionErrorMessage } from '@/lib/server-connection';

type FakeCall = Promise<unknown> & { cancel: ReturnType<typeof vi.fn> };

const runtimeMocks = vi.hoisted(() => ({
  byName: vi.fn(),
}));

vi.mock('@wailsio/runtime', () => ({
  Call: { ByName: runtimeMocks.byName },
}));

vi.mock('@/lib/platform/active', async () => ({
  ACTIVE_PLATFORM: (await import('@/lib/platform/native')).NATIVE_PLATFORM,
}));

/**
 * Build a settled fake Wails call.
 *
 * @param outcome - Resolved value, or an error to reject with.
 * @returns Fake cancellable call.
 */
function settledCall(outcome: unknown): FakeCall {
  const promise = (
    outcome instanceof Error ? Promise.reject(outcome) : Promise.resolve(outcome)
  ) as FakeCall;
  promise.catch(() => undefined);
  promise.cancel = vi.fn(() => Promise.resolve());
  return promise;
}

/**
 * Build a pending fake Wails call that rejects when cancelled.
 *
 * @returns Fake cancellable call.
 */
function pendingCall(): FakeCall {
  let rejectCall: (reason: unknown) => void = () => undefined;
  const promise = new Promise((_resolve, reject) => {
    rejectCall = reject;
  }) as FakeCall;
  promise.cancel = vi.fn(() => {
    rejectCall(new Error('cancelled'));
    return Promise.resolve();
  });
  return promise;
}

/**
 * Encode text as the base64 body the Go bridge returns.
 *
 * @param text - Body text.
 * @returns Base64 text.
 */
function base64Text(text: string): string {
  return btoa(String.fromCharCode(...new TextEncoder().encode(text)));
}

/**
 * Build a Go bridge error as the Wails runtime surfaces it.
 *
 * @param code - Bridge error code.
 * @returns Runtime-like error carrying the code in its cause.
 */
function bridgeFailure(code: string): Error {
  return new Error(`${code}: failure`, { cause: { code, message: 'failure' } });
}

/**
 * Configure the active origin through the server connection.
 */
async function connectToFixtureServer(): Promise<void> {
  runtimeMocks.byName.mockReturnValueOnce(settledCall('https://litradar.example.com'));
  await NATIVE_PLATFORM.serverConnection?.load();
}

beforeEach(async () => {
  runtimeMocks.byName.mockReset();
  await connectToFixtureServer();
});

afterEach(() => {
  delete window.litradarDownloads;
});

describe('native platform transport', () => {
  test('is the active platform in native mode', () => {
    expect(CLIENT_PLATFORM).toBe(NATIVE_PLATFORM);
    expect(CLIENT_PLATFORM.supportsSingleSignOn).toBe(false);
    expect(CLIENT_PLATFORM.serverOrigin()).toBe('https://litradar.example.com');
  });

  test('sends requests through the bridge and rebuilds the response', async () => {
    runtimeMocks.byName.mockReturnValueOnce(
      settledCall({
        status: 201,
        headers: { 'Content-Type': 'application/json', 'X-Request-Id': 'request-7' },
        body: base64Text('{"name":"阅读"}'),
      }),
    );

    const response = await NATIVE_PLATFORM.send('https://litradar.example.com/api/folders', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: '{"name":"阅读"}',
    });

    expect(runtimeMocks.byName).toHaveBeenLastCalledWith(`${BRIDGE_SERVICE}.Send`, {
      method: 'POST',
      url: 'https://litradar.example.com/api/folders',
      headers: { 'content-type': 'application/json' },
      body: '{"name":"阅读"}',
    });
    expect(response.status).toBe(201);
    expect(response.headers.get('X-Request-Id')).toBe('request-7');
    await expect(response.json()).resolves.toEqual({ name: '阅读' });
  });

  test('keeps null-body statuses empty', async () => {
    runtimeMocks.byName.mockReturnValueOnce(
      settledCall({ status: 204, headers: null, body: null }),
    );

    const response = await NATIVE_PLATFORM.send('https://litradar.example.com/api/x', {});

    expect(response.status).toBe(204);
    await expect(response.text()).resolves.toBe('');
  });

  test('cancels the Go call and rejects with the abort reason', async () => {
    const call = pendingCall();
    runtimeMocks.byName.mockReturnValueOnce(call);
    const controller = new AbortController();
    const reason = new DOMException('stop', 'AbortError');

    const sending = NATIVE_PLATFORM.send('https://litradar.example.com/api/slow', {
      signal: controller.signal,
    });
    controller.abort(reason);

    await expect(sending).rejects.toBe(reason);
    expect(call.cancel).toHaveBeenCalledTimes(1);
  });

  test('reports host failures as fetch-style network errors', async () => {
    runtimeMocks.byName.mockReturnValueOnce(settledCall(bridgeFailure('server_unreachable')));

    const sending = NATIVE_PLATFORM.send('https://litradar.example.com/api/x', {});

    await expect(sending).rejects.toBeInstanceOf(TypeError);
  });

  test('rejects binary request bodies before calling the host', async () => {
    await expect(
      NATIVE_PLATFORM.send('https://litradar.example.com/api/x', {
        method: 'POST',
        body: new Blob(['x']),
      }),
    ).rejects.toBeInstanceOf(TypeError);
    expect(runtimeMocks.byName).toHaveBeenCalledTimes(1);
  });

  test('round-trips base64 bodies', async () => {
    const bytes = new Uint8Array([0, 1, 254, 255, 128]);
    const encoded = await encodeBase64(new Blob([bytes]));
    expect(Array.from(decodeBase64(encoded))).toEqual(Array.from(bytes));
  });
});

describe('native credential flows', () => {
  test('signs in without exposing a token to JavaScript', async () => {
    runtimeMocks.byName.mockReturnValueOnce(
      settledCall({
        status: 200,
        headers: { 'Content-Type': 'application/json' },
        body: base64Text(
          JSON.stringify({
            expires_at: 1_900_000_000,
            user: {
              id: 7,
              username: 'member',
              is_admin: false,
              email: null,
              auth_kind: 'local',
              is_oidc_linked: false,
            },
          }),
        ),
      }),
    );

    const response = await loginUser('member', 'secret-password');

    expect(runtimeMocks.byName).toHaveBeenLastCalledWith(
      `${BRIDGE_SERVICE}.Login`,
      'member',
      'secret-password',
    );
    expect(response.user.username).toBe('member');
    expect(JSON.stringify(response)).not.toContain('token');
  });

  test('maps a rejected access token to a token-specific message', async () => {
    runtimeMocks.byName.mockReturnValueOnce(
      settledCall({
        status: 401,
        headers: { 'Content-Type': 'application/json' },
        body: base64Text('{"detail":"Not authenticated"}'),
      }),
    );

    const importing = importAccessToken('revoked-token');

    await expect(importing).rejects.toBeInstanceOf(ApiError);
    const error = await importing.catch((rejection: unknown) => rejection);
    expect(runtimeMocks.byName).toHaveBeenLastCalledWith(
      `${BRIDGE_SERVICE}.ImportToken`,
      'revoked-token',
    );
    expect(getAuthErrorMessage(error, 'token')).toBe('访问令牌无效、已过期或已被撤销。');
  });

  test('logs out through the host and treats an already rejected token as signed out', async () => {
    runtimeMocks.byName.mockReturnValueOnce(
      settledCall({ status: 401, headers: null, body: base64Text('{}') }),
    );

    await expect(logoutUser()).resolves.toBeUndefined();
    expect(runtimeMocks.byName).toHaveBeenLastCalledWith(`${BRIDGE_SERVICE}.Logout`);
  });

  test('discards the stored credential through the host', async () => {
    runtimeMocks.byName.mockReturnValueOnce(settledCall(null));

    await NATIVE_PLATFORM.nativeSession?.discardCredential();

    expect(runtimeMocks.byName).toHaveBeenLastCalledWith(`${BRIDGE_SERVICE}.ForgetToken`);
  });
});

describe('native server connection', () => {
  test('stores the canonical origin after the host probe succeeds', async () => {
    runtimeMocks.byName.mockReturnValueOnce(settledCall('http://127.0.0.1:8001'));

    await expect(NATIVE_PLATFORM.serverConnection?.connect('http://127.0.0.1:8001/')).resolves.toBe(
      'http://127.0.0.1:8001',
    );
    expect(NATIVE_PLATFORM.serverOrigin()).toBe('http://127.0.0.1:8001');
  });

  test('surfaces the host error code for rejected addresses', async () => {
    runtimeMocks.byName.mockReturnValueOnce(settledCall(bridgeFailure('insecure_origin')));

    const connecting = NATIVE_PLATFORM.serverConnection?.connect('http://example.com');

    await expect(connecting).rejects.toBeInstanceOf(PlatformBridgeError);
    const error = await connecting?.catch((rejection: unknown) => rejection);
    expect((error as PlatformBridgeError).code).toBe('insecure_origin');
    expect(getServerConnectionErrorMessage(error)).toContain('仅支持 https://');
  });

  test('forgets the origin when disconnecting', async () => {
    runtimeMocks.byName.mockReturnValueOnce(settledCall(null));

    await NATIVE_PLATFORM.serverConnection?.disconnect();

    expect(runtimeMocks.byName).toHaveBeenLastCalledWith(`${BRIDGE_SERVICE}.ForgetServer`);
    expect(NATIVE_PLATFORM.serverOrigin()).toBe('https://litradar.invalid');
  });
});

describe('native host integration', () => {
  test('saves downloads into Android Downloads when the host exposes it', async () => {
    const save = vi.fn(() => '{"ok":true}');
    window.litradarDownloads = { save };

    await expect(
      NATIVE_PLATFORM.saveDownload(new Blob(['@article{}'], { type: 'text/plain' }), 'a.bib'),
    ).resolves.toBe(true);

    expect(save).toHaveBeenCalledWith('a.bib', 'text/plain', btoa('@article{}'));
    expect(runtimeMocks.byName).toHaveBeenCalledTimes(1);
  });

  test('uses the desktop save dialog and reports cancellation', async () => {
    runtimeMocks.byName.mockReturnValueOnce(settledCall(false));

    await expect(NATIVE_PLATFORM.saveDownload(new Blob(['x']), 'a.bib')).resolves.toBe(false);

    expect(runtimeMocks.byName).toHaveBeenLastCalledWith(
      `${BRIDGE_SERVICE}.SaveDownload`,
      'a.bib',
      btoa('x'),
    );
  });

  test('opens external links in the system browser', () => {
    runtimeMocks.byName.mockReturnValue(settledCall(null));
    const external = document.createElement('a');
    external.href = 'https://doi.org/10.1000/1';
    const internal = document.createElement('a');
    internal.href = '#results';
    document.body.append(external, internal);
    document.addEventListener('click', handleExternalNavigation);
    try {
      const externalClick = new MouseEvent('click', { bubbles: true, cancelable: true, button: 0 });
      external.dispatchEvent(externalClick);
      const internalClick = new MouseEvent('click', { bubbles: true, cancelable: true, button: 0 });
      internal.dispatchEvent(internalClick);

      expect(externalClick.defaultPrevented).toBe(true);
      expect(internalClick.defaultPrevented).toBe(false);
      expect(runtimeMocks.byName).toHaveBeenLastCalledWith(
        `${BRIDGE_SERVICE}.OpenExternal`,
        'https://doi.org/10.1000/1',
      );
      expect(runtimeMocks.byName).toHaveBeenCalledTimes(2);
    } finally {
      document.removeEventListener('click', handleExternalNavigation);
      external.remove();
      internal.remove();
    }
  });
});
