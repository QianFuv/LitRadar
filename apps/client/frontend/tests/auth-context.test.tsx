/**
 * Authentication restore coverage using the real provider and MSW transport.
 */

import { act, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { http, HttpResponse } from 'msw';
import { describe, expect, test } from 'vite-plus/test';

import { AuthProvider, useAuth } from '@/lib/auth-context';
import { createAuthUserScenario, createLoginScenario } from '@/tests/mocks/scenarios';
import { renderWithQuery } from '@/tests/render';
import { server } from '@/tests/mocks/server';

const RESTORED_USER = createAuthUserScenario({
  id: 11,
  username: 'restored_admin',
  is_admin: true,
});

/**
 * Return the authoritative authenticated user fixture.
 *
 * @returns Current-user JSON response.
 */
function currentUserResponse(): Response {
  return HttpResponse.json(RESTORED_USER);
}

/**
 * Render the current authentication state for assertions.
 *
 * @returns Authentication state probe.
 */
function AuthProbe() {
  const { loading, logoutWarning, user } = useAuth();
  return (
    <div>
      <span>{loading ? 'loading' : 'ready'}</span>
      <span>{user?.username ?? 'anonymous'}</span>
      <span>
        {logoutWarning ? `warning:${logoutWarning.requestId ?? 'unknown'}` : 'no-warning'}
      </span>
    </div>
  );
}

/**
 * Render authentication actions and their current authoritative user.
 *
 * @returns Authentication action probe.
 */
function AuthActionProbe() {
  const { loading, login, logout, logoutWarning, recoverLogout, register, updateUser, user } =
    useAuth();
  return (
    <div>
      <span>{loading ? 'loading' : 'ready'}</span>
      <span>{user?.username ?? 'anonymous'}</span>
      <span>{`email:${user?.email ?? 'none'}`}</span>
      <span>
        {logoutWarning ? `warning:${logoutWarning.requestId ?? 'unknown'}` : 'no-warning'}
      </span>
      <button type="button" onClick={() => void login('login_user', 'login-password')}>
        Login action
      </button>
      <button
        type="button"
        onClick={() => void register('registered_user', 'register-password', 'invite-code')}
      >
        Register action
      </button>
      <button
        type="button"
        onClick={() =>
          void register(
            'registered_user',
            'register-password',
            'invite-code',
            'reader@example.test',
          )
        }
      >
        Register with email action
      </button>
      <button
        type="button"
        onClick={() =>
          updateUser(createAuthUserScenario({ ...RESTORED_USER, email: 'restored@example.test' }))
        }
      >
        Update same user action
      </button>
      <button
        type="button"
        onClick={() => updateUser(createAuthUserScenario({ id: 99, username: 'intruder' }))}
      >
        Update other user action
      </button>
      <button type="button" onClick={() => void logout().catch(() => undefined)}>
        Logout action
      </button>
      <button
        type="button"
        onClick={() =>
          void recoverLogout('recovery_user', 'recovery-password').catch(() => undefined)
        }
      >
        Recover logout action
      </button>
    </div>
  );
}

/**
 * Verify a stale local snapshot is replaced by the server session.
 */
async function restoresServerSession(): Promise<void> {
  window.localStorage.setItem(
    'litradar:v1:user',
    JSON.stringify({ id: 5, username: 'stale_user', is_admin: false }),
  );
  server.use(http.get('http://localhost/api/auth/me', currentUserResponse));

  await renderWithQuery(
    <AuthProvider>
      <AuthProbe />
    </AuthProvider>,
  );

  expect(await screen.findByText('restored_admin')).toBeInTheDocument();
  expect(screen.getByText('ready')).toBeInTheDocument();
  expect(JSON.parse(window.localStorage.getItem('litradar:v1:user') ?? '{}')).toEqual(
    RESTORED_USER,
  );
}

/**
 * Verify local metadata cannot preserve authentication without a valid server session.
 */
async function requiresAuthoritativeServerSession(): Promise<void> {
  window.localStorage.setItem('litradar:v1:user', JSON.stringify(RESTORED_USER));
  server.use(
    http.get('http://localhost/api/auth/me', () =>
      HttpResponse.json({ detail: 'Not authenticated' }, { status: 401 }),
    ),
  );

  await renderWithQuery(
    <AuthProvider>
      <AuthProbe />
    </AuthProvider>,
  );

  expect(await screen.findByText('anonymous')).toBeInTheDocument();
  expect(screen.getByText('ready')).toBeInTheDocument();
  expect(window.localStorage.getItem('litradar:v1:user')).toBeNull();
}

/**
 * Verify login persists only user metadata and clears stale query and token state.
 */
async function logsInAndClearsStaleClientState(): Promise<void> {
  let loginPayload: unknown;
  const loginScenario = createLoginScenario({
    user: createAuthUserScenario({ id: 12, username: 'login_user', is_admin: false }),
  });
  server.use(
    http.get('http://localhost/api/auth/me', () =>
      HttpResponse.json({ detail: 'Authentication required' }, { status: 401 }),
    ),
    http.post('http://localhost/api/auth/login', async ({ request }) => {
      loginPayload = await request.json();
      return HttpResponse.json(loginScenario);
    }),
  );
  window.sessionStorage.setItem('litradar:v1:session_access_token', 'stale-secret');
  const user = userEvent.setup();
  const { queryClient } = await renderWithQuery(
    <AuthProvider>
      <AuthActionProbe />
    </AuthProvider>,
  );
  queryClient.setQueryData(['stale-query'], 'stale-data');

  expect(await screen.findByText('anonymous')).toBeInTheDocument();
  await user.click(screen.getByRole('button', { name: 'Login action' }));

  expect(await screen.findByText('login_user')).toBeInTheDocument();
  expect(loginPayload).toEqual({ username: 'login_user', password: 'login-password' });
  expect(JSON.parse(window.localStorage.getItem('litradar:v1:user') ?? '{}')).toEqual(
    loginScenario.user,
  );
  expect(window.sessionStorage.getItem('litradar:v1:session_access_token')).toBeNull();
  expect(queryClient.getQueryData(['stale-query'])).toBeUndefined();
}

/**
 * Verify invited registration completes its login step and persists the returned session user.
 */
async function registersThenAuthenticates(): Promise<void> {
  let registrationPayload: unknown;
  const loginScenario = createLoginScenario({
    user: createAuthUserScenario({ id: 13, username: 'registered_user', is_admin: false }),
  });
  server.use(
    http.get('http://localhost/api/auth/me', () =>
      HttpResponse.json({ detail: 'Authentication required' }, { status: 401 }),
    ),
    http.post('http://localhost/api/auth/register', async ({ request }) => {
      registrationPayload = await request.json();
      return HttpResponse.json(loginScenario.user);
    }),
    http.post('http://localhost/api/auth/login', () => HttpResponse.json(loginScenario)),
  );
  const user = userEvent.setup();
  await renderWithQuery(
    <AuthProvider>
      <AuthActionProbe />
    </AuthProvider>,
  );

  expect(await screen.findByText('anonymous')).toBeInTheDocument();
  await user.click(screen.getByRole('button', { name: 'Register action' }));

  expect(await screen.findByText('registered_user')).toBeInTheDocument();
  expect(registrationPayload).toEqual({
    username: 'registered_user',
    password: 'register-password',
    invite_code: 'invite-code',
  });
}

/**
 * Verify an optional registration email reaches the register request unchanged.
 */
async function registersWithOptionalEmail(): Promise<void> {
  let registrationPayload: unknown;
  const loginScenario = createLoginScenario({
    user: createAuthUserScenario({
      id: 14,
      username: 'registered_user',
      is_admin: false,
      email: 'reader@example.test',
    }),
  });
  server.use(
    http.get('http://localhost/api/auth/me', () =>
      HttpResponse.json({ detail: 'Authentication required' }, { status: 401 }),
    ),
    http.post('http://localhost/api/auth/register', async ({ request }) => {
      registrationPayload = await request.json();
      return HttpResponse.json(loginScenario.user);
    }),
    http.post('http://localhost/api/auth/login', () => HttpResponse.json(loginScenario)),
  );
  const user = userEvent.setup();
  await renderWithQuery(
    <AuthProvider>
      <AuthActionProbe />
    </AuthProvider>,
  );

  expect(await screen.findByText('anonymous')).toBeInTheDocument();
  await user.click(screen.getByRole('button', { name: 'Register with email action' }));

  expect(await screen.findByText('email:reader@example.test')).toBeInTheDocument();
  expect(registrationPayload).toEqual({
    username: 'registered_user',
    password: 'register-password',
    invite_code: 'invite-code',
    email: 'reader@example.test',
  });
}

/**
 * Verify account updates replace only the verified user and are persisted as metadata.
 */
async function adoptsUpdatedProjectionForVerifiedUser(): Promise<void> {
  server.use(http.get('http://localhost/api/auth/me', currentUserResponse));
  const user = userEvent.setup();
  await renderWithQuery(
    <AuthProvider>
      <AuthActionProbe />
    </AuthProvider>,
  );

  expect(await screen.findByText('restored_admin')).toBeInTheDocument();
  await user.click(screen.getByRole('button', { name: 'Update other user action' }));
  expect(screen.getByText('restored_admin')).toBeInTheDocument();
  expect(screen.getByText('email:none')).toBeInTheDocument();

  await user.click(screen.getByRole('button', { name: 'Update same user action' }));
  expect(screen.getByText('email:restored@example.test')).toBeInTheDocument();
  expect(JSON.parse(window.localStorage.getItem('litradar:v1:user') ?? '{}')).toEqual(
    expect.objectContaining({ id: 11, email: 'restored@example.test' }),
  );
}

/**
 * Verify failed logout preserves an explicit warning across a simulated page refresh.
 */
async function preservesLogoutFailureAcrossRefresh(): Promise<void> {
  server.use(
    http.get('http://localhost/api/auth/me', currentUserResponse),
    http.post('http://localhost/api/auth/logout', () =>
      HttpResponse.json(
        {
          title: 'Service Unavailable',
          status: 503,
          detail: 'Session revocation could not be confirmed',
        },
        {
          status: 503,
          headers: {
            'Content-Type': 'application/problem+json',
            'X-Request-Id': 'logout-request-1',
          },
        },
      ),
    ),
  );
  window.sessionStorage.setItem('litradar:v1:session_access_token', 'stale-secret');
  const user = userEvent.setup();
  const view = await renderWithQuery(
    <AuthProvider>
      <AuthActionProbe />
    </AuthProvider>,
  );
  const { queryClient } = view;
  queryClient.setQueryData(['private-query'], 'private-data');

  expect(await screen.findByText('restored_admin')).toBeInTheDocument();
  await user.click(screen.getByRole('button', { name: 'Logout action' }));

  expect(await screen.findByText('anonymous')).toBeInTheDocument();
  expect(screen.getByText('warning:logout-request-1')).toBeInTheDocument();
  expect(window.localStorage.getItem('litradar:v1:user')).toBeNull();
  expect(window.sessionStorage.getItem('litradar:v1:session_access_token')).toBeNull();
  expect(queryClient.getQueryData(['private-query'])).toBeUndefined();
  expect(
    JSON.parse(window.localStorage.getItem('litradar:v1:logout_revocation_unconfirmed') ?? '{}'),
  ).toEqual(expect.objectContaining({ requestId: 'logout-request-1' }));

  view.unmount();
  server.use(
    http.get('http://localhost/api/auth/me', () =>
      HttpResponse.json({ detail: 'Authentication required' }, { status: 401 }),
    ),
  );
  await renderWithQuery(
    <AuthProvider>
      <AuthProbe />
    </AuthProvider>,
  );

  expect(await screen.findByText('warning:logout-request-1')).toBeInTheDocument();
  expect(screen.getByText('anonymous')).toBeInTheDocument();
}

/**
 * Verify a session that expired before logout is treated as an idempotent local logout.
 */
async function acceptsAlreadyExpiredSessionLogout(): Promise<void> {
  server.use(
    http.get('http://localhost/api/auth/me', currentUserResponse),
    http.post('http://localhost/api/auth/logout', () =>
      HttpResponse.json({ detail: 'Authentication required' }, { status: 401 }),
    ),
  );
  const user = userEvent.setup();
  await renderWithQuery(
    <AuthProvider>
      <AuthActionProbe />
    </AuthProvider>,
  );

  expect(await screen.findByText('restored_admin')).toBeInTheDocument();
  await user.click(screen.getByRole('button', { name: 'Logout action' }));

  expect(await screen.findByText('anonymous')).toBeInTheDocument();
  expect(screen.getByText('no-warning')).toBeInTheDocument();
  expect(window.localStorage.getItem('litradar:v1:logout_revocation_unconfirmed')).toBeNull();
}

/**
 * Verify fresh reauthentication revokes all sessions and clears the persisted warning.
 */
async function recoversLogoutAfterFreshAuthentication(): Promise<void> {
  let loginPayload: unknown;
  let logoutAllCalls = 0;
  window.localStorage.setItem(
    'litradar:v1:logout_revocation_unconfirmed',
    JSON.stringify({ occurredAt: 1234, requestId: 'previous-request' }),
  );
  server.use(
    http.get('http://localhost/api/auth/me', () =>
      HttpResponse.json({ detail: 'Authentication required' }, { status: 401 }),
    ),
    http.post('http://localhost/api/auth/login', async ({ request }) => {
      loginPayload = await request.json();
      return HttpResponse.json(
        createLoginScenario({
          user: createAuthUserScenario({ id: 19, username: 'recovery_user', is_admin: false }),
        }),
      );
    }),
    http.post('http://localhost/api/auth/logout-all', () => {
      logoutAllCalls += 1;
      return HttpResponse.json({ ok: true, user_id: 19 });
    }),
  );
  const user = userEvent.setup();
  await renderWithQuery(
    <AuthProvider>
      <AuthActionProbe />
    </AuthProvider>,
  );

  expect(await screen.findByText('warning:previous-request')).toBeInTheDocument();
  await user.click(screen.getByRole('button', { name: 'Recover logout action' }));

  await waitFor(() => expect(screen.getByText('no-warning')).toBeInTheDocument());
  expect(loginPayload).toEqual({
    username: 'recovery_user',
    password: 'recovery-password',
  });
  expect(logoutAllCalls).toBe(1);
  expect(window.localStorage.getItem('litradar:v1:logout_revocation_unconfirmed')).toBeNull();
  expect(screen.getByText('anonymous')).toBeInTheDocument();
}

/** Verify external metadata is only a revalidation signal and clears private state. */
async function reconcilesExternalSessions(): Promise<void> {
  let currentUser: typeof RESTORED_USER | null = RESTORED_USER;
  server.use(
    http.get('http://localhost/api/auth/me', () =>
      currentUser
        ? HttpResponse.json(currentUser)
        : HttpResponse.json({ detail: 'Authentication required' }, { status: 401 }),
    ),
  );
  const { queryClient } = await renderWithQuery(
    <AuthProvider>
      <AuthProbe />
    </AuthProvider>,
  );
  expect(await screen.findByText('restored_admin')).toBeInTheDocument();
  queryClient.setQueryData(['private-old-user'], 'private data');
  currentUser = createAuthUserScenario({ id: 22, username: 'replacement', is_admin: false });
  void act(() =>
    window.dispatchEvent(
      new StorageEvent('storage', {
        key: 'litradar:v1:user',
        newValue: JSON.stringify({ id: 999, username: 'untrusted' }),
      }),
    ),
  );
  expect(screen.getByText('loading')).toBeInTheDocument();
  expect(screen.queryByText('restored_admin')).not.toBeInTheDocument();
  expect(queryClient.getQueryData(['private-old-user'])).toBeUndefined();
  expect(await screen.findByText('replacement')).toBeInTheDocument();
  expect(screen.queryByText('untrusted')).not.toBeInTheDocument();
  currentUser = null;
  await act(() => document.dispatchEvent(new Event('visibilitychange')));
  await waitFor(() => expect(screen.getByText('ready')).toBeInTheDocument());
  expect(screen.getByText('anonymous')).toBeInTheDocument();
}

/** Verify an old session response cannot overwrite a newer verified identity. */
async function ignoresStaleSessionRestoration(): Promise<void> {
  let requestCount = 0;
  let releaseResponse: ((response: Response) => void) | undefined;
  const firstResponse = new Promise<Response>((resolve) => {
    releaseResponse = resolve;
  });
  server.use(
    http.get('http://localhost/api/auth/me', async () => {
      requestCount += 1;
      return requestCount === 1
        ? firstResponse
        : HttpResponse.json(
            createAuthUserScenario({ id: 22, username: 'replacement', is_admin: false }),
          );
    }),
  );
  await renderWithQuery(
    <AuthProvider>
      <AuthProbe />
    </AuthProvider>,
  );
  await waitFor(() => expect(requestCount).toBe(1));
  await act(() => window.dispatchEvent(new StorageEvent('storage', { key: 'litradar:v1:user' })));
  expect(await screen.findByText('replacement')).toBeInTheDocument();
  await act(async () => {
    releaseResponse?.(HttpResponse.json(RESTORED_USER));
  });
  expect(screen.queryByText('restored_admin')).not.toBeInTheDocument();
  expect(JSON.parse(window.localStorage.getItem('litradar:v1:user') ?? '{}').id).toBe(22);
}

/** Verify a routine visibility check does not discard unchanged account state. */
async function preservesUnchangedVisibleSession(): Promise<void> {
  let requestCount = 0;
  server.use(
    http.get('http://localhost/api/auth/me', () => {
      requestCount += 1;
      return HttpResponse.json(RESTORED_USER);
    }),
  );
  const { queryClient } = await renderWithQuery(
    <AuthProvider>
      <AuthProbe />
    </AuthProvider>,
  );
  expect(await screen.findByText('restored_admin')).toBeInTheDocument();
  queryClient.setQueryData(['private-same-user'], 'draft context');
  await act(() => document.dispatchEvent(new Event('visibilitychange')));
  expect(screen.queryByText('loading')).not.toBeInTheDocument();
  await waitFor(() => expect(requestCount).toBe(2));
  expect(queryClient.getQueryData(['private-same-user'])).toBe('draft context');
}

describe('AuthProvider restore', () => {
  test('preserves unchanged account state on visibility checks', preservesUnchangedVisibleSession);
  test('reconciles cross-tab identity and visible-page expiry', reconcilesExternalSessions);
  test('ignores stale session restoration', ignoresStaleSessionRestoration);
  test('reconciles stored metadata with the server session', restoresServerSession);
  test('requires an authoritative server session', requiresAuthoritativeServerSession);
  test('logs in and clears stale client state', logsInAndClearsStaleClientState);
  test('registers an invited user and authenticates it', registersThenAuthenticates);
  test('registers with an optional email', registersWithOptionalEmail);
  test(
    'adopts updated projections only for the verified user',
    adoptsUpdatedProjectionForVerifiedUser,
  );
  test('preserves logout failure across refresh', preservesLogoutFailureAcrossRefresh);
  test('accepts an already expired session logout', acceptsAlreadyExpiredSessionLogout);
  test('recovers logout after fresh authentication', recoversLogoutAfterFreshAuthentication);
});
