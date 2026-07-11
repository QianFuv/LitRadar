/**
 * Authentication restore coverage using the real provider and MSW transport.
 */

import { screen } from '@testing-library/react';
import { http, HttpResponse } from 'msw';
import { describe, expect, test } from 'vitest';

import { AuthProvider, useAuth } from '@/lib/auth-context';
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

describe('AuthProvider restore', () => {
  test('reconciles stored metadata with the server session', restoresServerSession);
  test('requires an authoritative server session', requiresAuthoritativeServerSession);
});
