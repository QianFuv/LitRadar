/**
 * Authentication restore coverage using the real provider and MSW transport.
 */

import { screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { http, HttpResponse } from 'msw';
import { describe, expect, test } from 'vitest';

import { AuthProvider, useAuth } from '@/lib/auth-context';
import { createLoginScenario } from '@/tests/mocks/scenarios';
import { renderWithQuery } from '@/tests/render';
import { server } from '@/tests/mocks/server';

const RESTORED_USER = { id: 11, username: 'restored_admin', is_admin: true };

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
  const { loading, user } = useAuth();
  return (
    <div>
      <span>{loading ? 'loading' : 'ready'}</span>
      <span>{user?.username ?? 'anonymous'}</span>
    </div>
  );
}

/**
 * Render authentication actions and their current authoritative user.
 *
 * @returns Authentication action probe.
 */
function AuthActionProbe() {
  const { loading, login, logout, register, user } = useAuth();
  return (
    <div>
      <span>{loading ? 'loading' : 'ready'}</span>
      <span>{user?.username ?? 'anonymous'}</span>
      <button type="button" onClick={() => void login('login_user', 'login-password')}>
        Login action
      </button>
      <button
        type="button"
        onClick={() => void register('registered_user', 'register-password', 'invite-code')}
      >
        Register action
      </button>
      <button type="button" onClick={() => void logout()}>
        Logout action
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

  renderWithQuery(
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

  renderWithQuery(
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
    user: { id: 12, username: 'login_user', is_admin: false },
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
  const { queryClient } = renderWithQuery(
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
    user: { id: 13, username: 'registered_user', is_admin: false },
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
  renderWithQuery(
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
 * Verify logout clears authoritative and cached state even when the server returns an error.
 */
async function clearsSessionWhenLogoutFails(): Promise<void> {
  server.use(
    http.get('http://localhost/api/auth/me', currentUserResponse),
    http.post('http://localhost/api/auth/logout', () =>
      HttpResponse.json({ detail: 'logout unavailable' }, { status: 500 }),
    ),
  );
  window.sessionStorage.setItem('litradar:v1:session_access_token', 'stale-secret');
  const user = userEvent.setup();
  const { queryClient } = renderWithQuery(
    <AuthProvider>
      <AuthActionProbe />
    </AuthProvider>,
  );
  queryClient.setQueryData(['private-query'], 'private-data');

  expect(await screen.findByText('restored_admin')).toBeInTheDocument();
  await user.click(screen.getByRole('button', { name: 'Logout action' }));

  expect(await screen.findByText('anonymous')).toBeInTheDocument();
  expect(window.localStorage.getItem('litradar:v1:user')).toBeNull();
  expect(window.sessionStorage.getItem('litradar:v1:session_access_token')).toBeNull();
  expect(queryClient.getQueryData(['private-query'])).toBeUndefined();
}

describe('AuthProvider restore', () => {
  test('reconciles stored metadata with the server session', restoresServerSession);
  test('requires an authoritative server session', requiresAuthoritativeServerSession);
  test('logs in and clears stale client state', logsInAndClearsStaleClientState);
  test('registers an invited user and authenticates it', registersThenAuthenticates);
  test('clears session state when logout fails', clearsSessionWhenLogoutFails);
});
