'use client';

import { Eye, EyeOff } from 'lucide-react';
import { useRouter, useSearchParams } from 'next/navigation';
import { useEffect, useState, type FormEvent } from 'react';
import { Button } from '@/components/ui/button';
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/components/ui/card';
import { Input } from '@/components/ui/input';
import { Label } from '@/components/ui/label';
import { getInviteRequirement } from '@/lib/api';
import { getAuthErrorMessage, type AuthFormMode } from '@/lib/auth-error';
import { useAuth } from '@/lib/auth-context';

/**
 * Render the restored login and registration form.
 *
 * @returns Login client component.
 */
export default function LoginClient() {
  const router = useRouter();
  const searchParams = useSearchParams();
  const { loading, login, register, user } = useAuth();
  const nextParam = searchParams.get('next') || '';
  const nextPath = nextParam.startsWith('/') && !nextParam.startsWith('//') ? nextParam : '/';

  const [username, setUsername] = useState('');
  const [password, setPassword] = useState('');
  const [inviteCode, setInviteCode] = useState('');
  const [error, setError] = useState<string | null>(null);
  const [isSubmitting, setIsSubmitting] = useState(false);
  const [isPasswordVisible, setIsPasswordVisible] = useState(false);
  const [mode, setMode] = useState<AuthFormMode>('login');
  const [inviteRequired, setInviteRequired] = useState(true);
  const [bootstrapRequired, setBootstrapRequired] = useState(false);

  useEffect(() => {
    if (!loading && user) {
      router.replace(nextPath);
    }
  }, [loading, nextPath, router, user]);

  useEffect(() => {
    if (loading || user) {
      return;
    }
    let didCancel = false;
    getInviteRequirement()
      .then((data) => {
        if (!didCancel) {
          setInviteRequired(data.required);
          setBootstrapRequired(data.bootstrap_required);
        }
      })
      .catch(() => {});
    return () => {
      didCancel = true;
    };
  }, [loading, user]);

  const handleSubmit = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    setError(null);
    setIsSubmitting(true);

    try {
      if (mode === 'register') {
        await register(username.trim(), password, inviteCode.trim());
      } else {
        await login(username.trim(), password);
      }
      router.replace(nextPath);
    } catch (err) {
      setError(getAuthErrorMessage(err, mode));
    } finally {
      setIsSubmitting(false);
    }
  };

  if (loading || user) {
    return (
      <main
        id="main-content"
        className="flex min-h-dvh items-center justify-center bg-background px-6"
      >
        <div role="status" className="text-sm text-muted-foreground">
          正在检查登录状态…
        </div>
      </main>
    );
  }

  return (
    <main
      id="main-content"
      className="flex min-h-dvh items-center justify-center bg-background px-6"
    >
      <Card className="w-full max-w-md">
        <CardHeader>
          <CardTitle>{mode === 'login' ? '登录' : '注册'}</CardTitle>
          <CardDescription>
            {mode === 'login' ? '输入账号和密码登录' : '创建一个新账号'}
          </CardDescription>
        </CardHeader>
        <CardContent>
          <form
            onSubmit={handleSubmit}
            className="space-y-4"
            aria-describedby={error ? 'login-error' : undefined}
          >
            <div className="space-y-2">
              <Label htmlFor="username">用户名</Label>
              <Input
                id="username"
                name="username"
                type="text"
                value={username}
                autoComplete="username"
                autoFocus
                spellCheck={false}
                onChange={(event) => setUsername(event.target.value)}
                placeholder="3-32位字母数字下划线"
                aria-invalid={Boolean(error)}
                aria-describedby={error ? 'login-error' : undefined}
                required
              />
            </div>
            <div className="space-y-2">
              <Label htmlFor="password">密码</Label>
              <div className="relative">
                <Input
                  id="password"
                  name="password"
                  type={isPasswordVisible ? 'text' : 'password'}
                  value={password}
                  autoComplete={mode === 'register' ? 'new-password' : 'current-password'}
                  onChange={(event) => setPassword(event.target.value)}
                  placeholder={mode === 'register' ? '至少12位' : '输入当前密码'}
                  minLength={mode === 'register' ? 12 : undefined}
                  className="pr-10"
                  aria-invalid={Boolean(error)}
                  aria-describedby={error ? 'login-error' : undefined}
                  required
                />
                <Button
                  type="button"
                  variant="ghost"
                  size="icon"
                  className="absolute inset-y-0 right-0 h-full rounded-l-none text-muted-foreground hover:text-foreground"
                  aria-label={isPasswordVisible ? '隐藏密码' : '显示密码'}
                  aria-pressed={isPasswordVisible}
                  onClick={() => setIsPasswordVisible((current) => !current)}
                >
                  {isPasswordVisible ? (
                    <EyeOff className="h-4 w-4" aria-hidden="true" />
                  ) : (
                    <Eye className="h-4 w-4" aria-hidden="true" />
                  )}
                </Button>
              </div>
            </div>
            {mode === 'register' && inviteRequired && (
              <div className="space-y-2">
                <Label htmlFor="invite-code">邀请码</Label>
                <Input
                  id="invite-code"
                  name="invite_code"
                  type="text"
                  value={inviteCode}
                  autoComplete="one-time-code"
                  spellCheck={false}
                  onChange={(event) => setInviteCode(event.target.value)}
                  placeholder="输入邀请码"
                  aria-invalid={Boolean(error)}
                  aria-describedby={error ? 'login-error' : undefined}
                  required
                />
              </div>
            )}
            {mode === 'register' && bootstrapRequired && (
              <div
                role="status"
                className="rounded-md border border-amber-500/30 bg-amber-500/10 px-3 py-2 text-sm text-foreground"
              >
                系统管理员尚未完成本机初始化。请先在服务器上运行{' '}
                <code>admin bootstrap --username NAME --password-stdin</code>
                ，再使用管理员生成的邀请码注册。
              </div>
            )}
            {error && (
              <div
                id="login-error"
                role="alert"
                className="rounded-md border border-destructive/30 bg-destructive/10 px-3 py-2 text-sm text-destructive"
              >
                {error}
              </div>
            )}
            <Button
              type="submit"
              className="w-full"
              disabled={isSubmitting || (mode === 'register' && bootstrapRequired)}
            >
              {isSubmitting ? '请稍候…' : mode === 'login' ? '登录' : '注册'}
            </Button>
          </form>
          <div className="mt-4 text-center text-sm text-muted-foreground">
            {mode === 'login' ? (
              <>
                没有账号？{' '}
                <button
                  type="button"
                  className="underline text-primary hover:text-primary/80"
                  onClick={() => {
                    setMode('register');
                    setError(null);
                  }}
                >
                  注册
                </button>
              </>
            ) : (
              <>
                已有账号？{' '}
                <button
                  type="button"
                  className="underline text-primary hover:text-primary/80"
                  onClick={() => {
                    setMode('login');
                    setError(null);
                  }}
                >
                  登录
                </button>
              </>
            )}
          </div>
        </CardContent>
      </Card>
    </main>
  );
}
