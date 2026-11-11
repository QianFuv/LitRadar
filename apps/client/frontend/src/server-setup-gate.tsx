/**
 * First-run server selection for native hosts, rendered outside the authentication provider.
 */

import { useCallback, useEffect, useMemo, useState, type FormEvent } from 'react';
import type { ReactNode } from 'react';

import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import { Label } from '@/components/ui/label';
import { LoadingText } from '@/components/ui/loading-text';
import { clearStoredSession } from '@/lib/auth-context';
import { CLIENT_PLATFORM, type ServerConnection } from '@/lib/platform';
import { ServerConnectionContext, getServerConnectionErrorMessage } from '@/lib/server-connection';
import * as stylex from '@stylexjs/stylex';
import { colors, lineHeights } from '@/lib/theme.stylex';

type GatePhase = { kind: 'loading' } | { kind: 'setup' } | { kind: 'ready'; origin: string };

/**
 * Render children directly in the browser, or after a server is selected on native hosts.
 *
 * @param props - Application tree that requires a server.
 * @returns Children, or the native server setup flow.
 */
export function ServerSetupGate({ children }: { children: ReactNode }) {
  const connection = CLIENT_PLATFORM.serverConnection;
  if (!connection) {
    return children;
  }
  return <NativeServerGate connection={connection}>{children}</NativeServerGate>;
}

/**
 * Load the stored server and show the setup form until one is connected.
 *
 * @param props - Host server connection and the application tree.
 * @returns Loading state, setup form, or the connected application.
 */
function NativeServerGate({
  connection,
  children,
}: {
  connection: ServerConnection;
  children: ReactNode;
}) {
  const [phase, setPhase] = useState<GatePhase>({ kind: 'loading' });

  useEffect(() => {
    let didCancel = false;
    connection
      .load()
      .then((origin) => {
        if (!didCancel) {
          setPhase(origin ? { kind: 'ready', origin } : { kind: 'setup' });
        }
      })
      .catch(() => {
        if (!didCancel) {
          setPhase({ kind: 'setup' });
        }
      });
    return () => {
      didCancel = true;
    };
  }, [connection]);

  const switchServer = useCallback(async () => {
    await connection.disconnect();
    clearStoredSession();
    setPhase({ kind: 'setup' });
  }, [connection]);

  const origin = phase.kind === 'ready' ? phase.origin : null;
  const value = useMemo(
    () => (origin === null ? null : { origin, switchServer }),
    [origin, switchServer],
  );

  if (phase.kind === 'loading') {
    return (
      <main id="main-content" {...stylex.props(styles.nativeServerGateMain)}>
        <output {...stylex.props(styles.statusOutput, styles.nativeServerGateDiv)}>
          <LoadingText text="正在读取服务器配置…" />
        </output>
      </main>
    );
  }
  if (value === null) {
    return (
      <ServerSetupForm
        connection={connection}
        onConnected={(connectedOrigin) => setPhase({ kind: 'ready', origin: connectedOrigin })}
      />
    );
  }
  return (
    <ServerConnectionContext.Provider value={value}>{children}</ServerConnectionContext.Provider>
  );
}

/**
 * Ask for a server address, probe it through the host, and store it.
 *
 * @param props - Host server connection and the success callback.
 * @returns Server setup page.
 */
function ServerSetupForm({
  connection,
  onConnected,
}: {
  connection: ServerConnection;
  onConnected: (origin: string) => void;
}) {
  const [address, setAddress] = useState('');
  const [error, setError] = useState<string | null>(null);
  const [isConnecting, setIsConnecting] = useState(false);

  const handleSubmit = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    setError(null);
    setIsConnecting(true);
    try {
      onConnected(await connection.connect(address.trim()));
    } catch (connectError) {
      setError(getServerConnectionErrorMessage(connectError));
      setIsConnecting(false);
    }
  };

  return (
    <main id="main-content" {...stylex.props(styles.serverSetupFormMain)}>
      <div {...stylex.props(styles.serverSetupFormDiv)}>
        <div {...stylex.props(styles.serverSetupFormDiv2)}>
          <img
            src="/litradar-logo.png"
            alt=""
            width={40}
            height={40}
            {...stylex.props(styles.serverSetupFormImg)}
          />
          <div>
            <p {...stylex.props(styles.serverSetupFormP)}>LitRadar</p>
            <p {...stylex.props(styles.serverSetupFormP2)}>文献雷达与持续追踪</p>
          </div>
        </div>
        <h1 {...stylex.props(styles.serverSetupFormH1)}>连接服务器</h1>
        <p {...stylex.props(styles.serverSetupFormP3)}>
          输入 LitRadar 服务器地址，例如 https://litradar.example.com。本机调试可使用
          http://127.0.0.1:端口。
        </p>
        <form
          aria-label="服务器设置表单"

          onSubmit={handleSubmit}
          aria-describedby={error ? 'server-setup-error' : undefined}
        >
          <div {...stylex.props(styles.serverSetupFormFormItem)}>
            <Label xstyle={styles.serverSetupFormDivItem} htmlFor="server-address">
              服务器地址
            </Label>
            <Input
              xstyle={styles.serverSetupFormDivItem}
              id="server-address"
              name="server_address"
              type="url"
              inputMode="url"
              autoComplete="url"
              autoFocus
              spellCheck={false}
              value={address}
              onChange={(event) => setAddress(event.target.value)}
              placeholder="https://litradar.example.com"
              aria-invalid={Boolean(error)}
              aria-describedby={error ? 'server-setup-error' : undefined}
              required
            />
          </div>
          {error && (
            <div
              id="server-setup-error"
              role="alert"
              {...stylex.props(styles.serverSetupFormFormItem, styles.serverSetupFormDiv3)}
            >
              {error}
            </div>
          )}
          <Button
            type="submit"
            xstyle={[styles.serverSetupFormFormItem, styles.serverSetupFormButton]}
            disabled={isConnecting}
          >
            {isConnecting ? '正在连接…' : '连接'}
          </Button>
        </form>
      </div>
    </main>
  );
}

const styles = stylex.create({
  statusOutput: {
    display: 'block',
  },
  nativeServerGateMain: {
    display: 'flex',
    minHeight: '100dvh',
    alignItems: 'center',
    justifyContent: 'center',
  },
  nativeServerGateDiv: {
    fontSize: '0.875rem',
    lineHeight: lineHeights.textSm,
    color: colors.mutedForeground,
  },
  serverSetupFormMain: {
    display: 'flex',
    minHeight: '100dvh',
    alignItems: 'center',
    justifyContent: 'center',
    backgroundColor: colors.background,
    paddingInline: '1.5rem',
    paddingBlock: '3rem',
  },
  serverSetupFormDiv: {
    width: '100%',
    maxWidth: '24rem',
  },
  serverSetupFormDiv2: {
    marginBottom: '2rem',
    display: 'flex',
    alignItems: 'center',
    gap: '0.75rem',
  },
  serverSetupFormImg: {
    width: '2.5rem',
    height: '2.5rem',
    borderRadius: '12px',
    objectFit: 'cover',
  },
  serverSetupFormP: {
    fontSize: '1rem',
    lineHeight: 1.5,
    fontWeight: 700,
    letterSpacing: '-0.025em',
  },
  serverSetupFormP2: {
    fontSize: '0.75rem',
    lineHeight: lineHeights.textXs,
    color: colors.mutedForeground,
  },
  serverSetupFormH1: {
    fontSize: '28px',
    lineHeight: 1.25,
    fontWeight: 700,
    letterSpacing: '-0.025em',
  },
  serverSetupFormP3: {
    marginTop: '0.5rem',
    marginBottom: '1.75rem',
    fontSize: '0.875rem',
    lineHeight: '1.5rem',
    color: colors.mutedForeground,
  },
  serverSetupFormFormItem: {
    marginBottom: {
      default: null,
      ':not(:last-child)': '1rem',
    },
  },
  serverSetupFormDivItem: {
    marginBottom: {
      default: null,
      ':not(:last-child)': '0.5rem',
    },
  },
  serverSetupFormDiv3: {
    borderRadius: '6px',
    borderStyle: 'solid',
    borderWidth: '1px',
    borderColor: `color-mix(in oklab, ${colors.destructive} 30%, transparent)`,
    backgroundColor: `color-mix(in oklab, ${colors.destructive} 10%, transparent)`,
    paddingInline: '0.75rem',
    paddingBlock: '0.5rem',
    fontSize: '0.875rem',
    lineHeight: lineHeights.textSm,
    color: colors.destructive,
  },
  serverSetupFormButton: {
    width: '100%',
  },
});
