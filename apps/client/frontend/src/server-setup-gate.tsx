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
      <main id="main-content" className="flex min-h-dvh items-center justify-center">
        <div role="status" className="text-sm text-muted-foreground">
          <LoadingText text="正在读取服务器配置…" />
        </div>
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
    <main
      id="main-content"
      className="flex min-h-dvh items-center justify-center bg-background px-6 py-12"
    >
      <div className="w-full max-w-sm">
        <div className="mb-8 flex items-center gap-3">
          <img
            src="/litradar-logo.png"
            alt=""
            width={40}
            height={40}
            className="size-10 rounded-xl object-cover"
          />
          <div>
            <p className="text-base font-bold tracking-tight">LitRadar</p>
            <p className="text-xs text-muted-foreground">文献雷达与持续追踪</p>
          </div>
        </div>
        <h1 className="text-[28px] leading-tight font-bold tracking-tight">连接服务器</h1>
        <p className="mt-2 mb-7 text-sm leading-6 text-muted-foreground">
          输入 LitRadar 服务器地址，例如 https://litradar.example.com。本机调试可使用
          http://127.0.0.1:端口。
        </p>
        <form
          aria-label="服务器设置表单"
          className="space-y-4"
          onSubmit={handleSubmit}
          aria-describedby={error ? 'server-setup-error' : undefined}
        >
          <div className="space-y-2">
            <Label htmlFor="server-address">服务器地址</Label>
            <Input
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
              className="rounded-md border border-destructive/30 bg-destructive/10 px-3 py-2 text-sm text-destructive"
            >
              {error}
            </div>
          )}
          <Button type="submit" className="w-full" disabled={isConnecting}>
            {isConnecting ? '正在连接…' : '连接'}
          </Button>
        </form>
      </div>
    </main>
  );
}
