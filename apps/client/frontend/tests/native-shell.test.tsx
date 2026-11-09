/**
 * Native shell coverage: first-run server setup, server switching, access-token sign-in, and
 * hidden single sign-on actions on a host without browser redirects.
 */

import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { MemoryRouter } from 'react-router';
import { beforeEach, describe, expect, test, vi } from 'vitest';

import { AccountSignInCard } from '@/components/settings/account-card';
import { MotionProvider } from '@/components/ui/motion';
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
  signInWithAccessToken: vi.fn(),
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
      importAccessToken: vi.fn(),
      logout: vi.fn(),
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
    signInWithAccessToken: shellMocks.signInWithAccessToken,
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
  shellMocks.signInWithAccessToken.mockReset();
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
  test('offers access-token sign-in instead of single sign-on', async () => {
    const user = userEvent.setup();
    shellMocks.signInWithAccessToken.mockResolvedValue(undefined);

    render(
      <MotionProvider>
        <LoginClient />
      </MotionProvider>,
      { wrapper: MemoryRouter },
    );

    await user.click(await screen.findByRole('button', { name: '使用访问令牌登录' }));
    expect(screen.queryByRole('link', { name: /使用 校园身份 登录/ })).toBeNull();
    await user.type(screen.getByLabelText('访问令牌'), '  pasted-token ');
    await user.click(screen.getByRole('button', { name: '登录' }));

    await waitFor(() =>
      expect(shellMocks.signInWithAccessToken).toHaveBeenCalledWith('pasted-token'),
    );
    expect(shellMocks.login).not.toHaveBeenCalled();
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

    expect(await screen.findByText('客户端不支持单点登录，请在网页版中绑定或解除')).toBeVisible();
    expect(screen.queryByRole('button', { name: /绑定/ })).toBeNull();
  });
});
