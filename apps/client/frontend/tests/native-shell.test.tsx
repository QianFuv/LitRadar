/**
 * Native shell coverage: first-run server setup, server switching, password-only sign-in choices,
 * single sign-on through the host and the system browser, and hidden account-link actions.
 */

import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { MemoryRouter, Route, Routes, useLocation } from 'react-router';
import { beforeEach, describe, expect, test, vi } from 'vite-plus/test';

import { AccountSignInCard } from '@/components/settings/account-card';
import { MotionProvider } from '@/components/ui/motion';
import { getAuthProviders } from '@/lib/api';
import type { ClientPlatform } from '@/lib/platform';
import { PlatformBridgeError } from '@/lib/platform/errors';
import { useServerConnection } from '@/lib/server-connection';
import LoginClient from '@/src/routes/login-client';
import { ServerSetupGate } from '@/src/server-setup-gate';
import { renderWithQuery } from '@/tests/render';

const shellMocks = vi.hoisted(() => ({
  load: vi.fn(),
  connect: vi.fn(),
  disconnect: vi.fn(),
  signInWithNativeOidc: vi.fn(),
  consumeOidcInterruption: vi.fn(),
  login: vi.fn(),
  user: null as Record<string, unknown> | null,
}));

vi.mock('@/lib/platform', () => {
  const platform: ClientPlatform = {
    send: (url, init) => fetch(url, init),
    saveDownload: async () => true,
    openExternal: () => undefined,
    serverOrigin: () => 'http://localhost',
    prepareDocument: () => undefined,
    supportsSingleSignOn: false,
    nativeSession: {
      login: vi.fn(),
      logout: vi.fn(),
      loginWithOidc: vi.fn(),
      consumeOidcInterruption: shellMocks.consumeOidcInterruption,
      discardCredential: vi.fn(async () => undefined),
    },
    serverConnection: {
      load: shellMocks.load,
      connect: shellMocks.connect,
      disconnect: shellMocks.disconnect,
    },
  };
  return { CLIENT_PLATFORM: platform };
});

vi.mock('@/lib/auth-context', async (importOriginal) => ({
  ...(await importOriginal<typeof import('@/lib/auth-context')>()),
  useAuth: () => ({
    loading: false,
    login: shellMocks.login,
    logoutWarning: null,
    recoverLogout: vi.fn(),
    register: vi.fn(),
    signInWithNativeOidc: shellMocks.signInWithNativeOidc,
    updateUser: vi.fn(),
    user: shellMocks.user,
  }),
}));

vi.mock('@/lib/api', async (importOriginal) => ({
  ...(await importOriginal<typeof import('@/lib/api')>()),
  getAuthProviders: vi.fn(async () => ({ oidc_enabled: true, oidc_display_name: '校园身份' })),
  getInviteRequirement: vi.fn(async () => ({ required: true, bootstrap_required: false })),
}));

/**
 * Show the current router location after a sign-in navigation.
 *
 * @returns Location marker.
 */
function LoginLocationProbe() {
  const location = useLocation();
  return <p data-testid="login-location">{`${location.pathname}${location.search}`}</p>;
}

/**
 * Render the native login page with a route that reports where sign-in navigates.
 *
 * @param initialEntry - Initial router URL.
 */
function renderNativeLogin(initialEntry: string): void {
  render(
    <MotionProvider>
      <MemoryRouter initialEntries={[initialEntry]}>
        <Routes>
          <Route path="/login" element={<LoginClient />} />
          <Route path="/" element={<LoginLocationProbe />} />
        </Routes>
      </MemoryRouter>
    </MotionProvider>,
  );
}

/**
 * Show the connected origin and expose the switch action to the test.
 *
 * @returns Connected application stand-in.
 */
function ConnectedApplication() {
  const connection = useServerConnection();
  return (
    <div>
      <p>已连接 {connection?.origin}</p>
      <button type="button" onClick={() => void connection?.switchServer()}>
        切换
      </button>
    </div>
  );
}

beforeEach(() => {
  shellMocks.load.mockReset();
  shellMocks.connect.mockReset();
  shellMocks.disconnect.mockReset();
  shellMocks.signInWithNativeOidc.mockReset();
  shellMocks.consumeOidcInterruption.mockReset().mockResolvedValue(false);
  shellMocks.login.mockReset();
  shellMocks.user = null;
});

describe('native server setup gate', () => {
  test('connects a probed server, then switches back to setup and clears the user cache', async () => {
    const user = userEvent.setup();
    shellMocks.load.mockResolvedValue(null);
    shellMocks.connect
      .mockRejectedValueOnce(new PlatformBridgeError('insecure_origin', 'http is loopback only'))
      .mockResolvedValueOnce('http://127.0.0.1:8001');
    shellMocks.disconnect.mockResolvedValue(undefined);
    window.localStorage.setItem('litradar:v1:user', '{"id":7}');

    render(
      <ServerSetupGate>
        <ConnectedApplication />
      </ServerSetupGate>,
    );

    const address = await screen.findByLabelText('服务器地址');
    await user.type(address, 'http://example.com');
    await user.click(screen.getByRole('button', { name: '连接' }));
    expect(await screen.findByRole('alert')).toHaveTextContent('仅支持 https://');

    await user.clear(address);
    await user.type(address, 'http://127.0.0.1:8001');
    await user.click(screen.getByRole('button', { name: '连接' }));
    expect(await screen.findByText('已连接 http://127.0.0.1:8001')).toBeInTheDocument();
    expect(shellMocks.connect).toHaveBeenLastCalledWith('http://127.0.0.1:8001');

    await user.click(screen.getByRole('button', { name: '切换' }));
    expect(await screen.findByLabelText('服务器地址')).toBeInTheDocument();
    expect(shellMocks.disconnect).toHaveBeenCalledTimes(1);
    expect(window.localStorage.getItem('litradar:v1:user')).toBeNull();
  });

  test('restores a stored server without showing setup', async () => {
    shellMocks.load.mockResolvedValue('https://litradar.example.com');

    render(
      <ServerSetupGate>
        <ConnectedApplication />
      </ServerSetupGate>,
    );

    expect(await screen.findByText('已连接 https://litradar.example.com')).toBeInTheDocument();
    expect(screen.queryByLabelText('服务器地址')).toBeNull();
  });
});

describe('native sign-in surface', () => {
  test('signs in through the host single sign-on and returns to the requested page', async () => {
    const user = userEvent.setup();
    shellMocks.signInWithNativeOidc.mockResolvedValue(undefined);

    renderNativeLogin('/login?next=%2F%3Fview%3Dfavorites');

    await user.click(await screen.findByRole('button', { name: '使用 校园身份 登录' }));

    expect(await screen.findByTestId('login-location')).toHaveTextContent('/?view=favorites');
    expect(shellMocks.signInWithNativeOidc).toHaveBeenCalledWith(expect.any(AbortSignal));
  });

  test('shows the browser wait and returns to idle when cancelled', async () => {
    const user = userEvent.setup();
    const captured: { signal: AbortSignal | null } = { signal: null };
    shellMocks.signInWithNativeOidc.mockImplementation(
      (signal: AbortSignal) =>
        new Promise((_resolve, reject) => {
          captured.signal = signal;
          signal.addEventListener('abort', () => reject(signal.reason), { once: true });
        }),
    );

    renderNativeLogin('/login');
    await user.click(await screen.findByRole('button', { name: '使用 校园身份 登录' }));

    expect(await screen.findByRole('status')).toHaveTextContent('已在浏览器中打开 校园身份');
    expect(screen.getByRole('button', { name: '登录' })).toBeDisabled();
    await user.click(screen.getByRole('button', { name: '取消单点登录' }));

    expect(captured.signal?.aborted).toBe(true);
    expect(await screen.findByRole('button', { name: '使用 校园身份 登录' })).toBeEnabled();
    expect(screen.queryByRole('alert')).toBeNull();
  });

  test('announces single sign-on failures reported by the host', async () => {
    const user = userEvent.setup();
    shellMocks.signInWithNativeOidc
      .mockRejectedValueOnce(new PlatformBridgeError('oidc_error', 'denied', undefined, 'denied'))
      .mockRejectedValueOnce(new PlatformBridgeError('oidc_timeout', 'timeout'));

    renderNativeLogin('/login');
    await user.click(await screen.findByRole('button', { name: '使用 校园身份 登录' }));
    expect(await screen.findByRole('alert')).toHaveTextContent('授权已取消');

    await user.click(screen.getByRole('button', { name: '使用 校园身份 登录' }));
    await waitFor(() => expect(screen.getByRole('alert')).toHaveTextContent('5 分钟'));
  });

  test('tells the user when the app lost a sign-in while the browser was open', async () => {
    shellMocks.consumeOidcInterruption.mockResolvedValue(true);

    renderNativeLogin('/login');

    expect(await screen.findByRole('alert')).toHaveTextContent('单点登录被中断');
    expect(screen.getByRole('button', { name: '使用 校园身份 登录' })).toBeEnabled();
  });

  test('offers only password and single sign-on sign-in', async () => {
    const user = userEvent.setup();
    shellMocks.login.mockResolvedValue(undefined);

    renderNativeLogin('/login');

    expect(await screen.findByRole('button', { name: '使用 校园身份 登录' })).toBeEnabled();
    expect(screen.queryByRole('button', { name: /访问令牌/ })).toBeNull();
    expect(screen.queryByLabelText(/访问令牌/)).toBeNull();
    await user.type(screen.getByLabelText('用户名或邮箱'), ' member ');
    await user.type(screen.getByLabelText('密码'), 'correct horse battery');
    await user.click(screen.getByRole('button', { name: '登录' }));

    await waitFor(() =>
      expect(shellMocks.login).toHaveBeenCalledWith('member', 'correct horse battery'),
    );
  });

  test('offers only password sign-in when single sign-on is disabled', async () => {
    vi.mocked(getAuthProviders).mockResolvedValueOnce({
      oidc_enabled: false,
      oidc_display_name: '',
    });

    renderNativeLogin('/login');

    await waitFor(() => expect(getAuthProviders).toHaveBeenCalled());
    expect(await screen.findByRole('button', { name: '登录' })).toBeEnabled();
    expect(screen.queryByRole('button', { name: /使用 .* 登录|访问令牌/ })).toBeNull();
    expect(screen.queryByText('或')).toBeNull();
  });

  test('hides single sign-on link actions in account settings', async () => {
    shellMocks.user = {
      id: 7,
      username: 'member',
      is_admin: false,
      email: null,
      auth_kind: 'local',
      is_oidc_linked: false,
    };

    renderWithQuery(<AccountSignInCard oidcLinkResult={null} />);

    expect(
      await screen.findByText('客户端不支持绑定或解除单点登录，请在网页版中操作'),
    ).toBeVisible();
    expect(screen.queryByRole('button', { name: /绑定/ })).toBeNull();
  });
});
