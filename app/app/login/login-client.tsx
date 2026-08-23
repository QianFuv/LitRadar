'use client';

/**
 * Public authentication surface for login, registration, and logout recovery.
 */

import { Eye, EyeOff, Radar } from 'lucide-react';
import Link from 'next/link';
import { useRouter, useSearchParams } from 'next/navigation';
import { useEffect, useState, type FormEvent } from 'react';
import { Button } from '@/components/ui/button';
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/components/ui/card';
import { Input } from '@/components/ui/input';
import { Label } from '@/components/ui/label';
import {
  COLLAPSE_VARIANTS,
  FADE_UP_VARIANTS,
  MOTION_DURATION_SECONDS,
  MotionDiv,
  MotionForm,
  MotionPresence,
  MotionSpan,
  useMotionTransition,
} from '@/components/ui/motion';
import { getInviteRequirement } from '@/lib/api';
import { getAuthErrorMessage, type AuthFormMode } from '@/lib/auth-error';
import { useAuth } from '@/lib/auth-context';

const LOGIN_RETURN_ORIGIN = 'https://litradar.invalid';

/**
 * Render the quiet product identity shared by loading and editable authentication states.
 *
 * @returns Compact grayscale LitRadar brand mark.
 */
function AuthBrand() {
  return (
    <div className="mb-6 flex items-center gap-3" aria-label="LitRadar">
      <div className="flex size-10 items-center justify-center rounded-lg border bg-card shadow-vercel-ring">
        <Radar className="size-5" aria-hidden="true" />
      </div>
      <div>
        <p className="font-mono text-base font-semibold tracking-tight">LitRadar</p>
        <p className="text-xs text-muted-foreground">文献雷达与持续追踪</p>
      </div>
    </div>
  );
}

/**
 * Normalize one untrusted post-login return value to a same-origin application path.
 *
 * @param candidate - Decoded next query parameter.
 * @returns Canonical internal path, query, and hash or the application root.
 */
function normalizeLoginReturnPath(candidate: string): string {
  const hasControlCharacter = Array.from(candidate).some((character) => {
    const codePoint = character.codePointAt(0) ?? 0;
    return codePoint <= 0x1f || codePoint === 0x7f;
  });
  if (!candidate.startsWith('/') || candidate.includes('\\') || hasControlCharacter) {
    return '/';
  }
  try {
    const target = new URL(candidate, LOGIN_RETURN_ORIGIN);
    if (target.origin !== LOGIN_RETURN_ORIGIN) {
      return '/';
    }
    return `${target.pathname}${target.search}${target.hash}`;
  } catch {
    return '/';
  }
}

/**
 * Render the restored login and registration form.
 *
 * @returns Login client component.
 */
export default function LoginClient() {
  const router = useRouter();
  const searchParams = useSearchParams();
  const { loading, login, logoutWarning, recoverLogout, register, user } = useAuth();
  const nextParam = searchParams.get('next') || '';
  const nextPath = normalizeLoginReturnPath(nextParam);
  const isLogoutRecovery = searchParams.get('logout_recovery') === '1';

  const [username, setUsername] = useState('');
  const [password, setPassword] = useState('');
  const [inviteCode, setInviteCode] = useState('');
  const [error, setError] = useState<string | null>(null);
  const [isSubmitting, setIsSubmitting] = useState(false);
  const [isPasswordVisible, setIsPasswordVisible] = useState(false);
  const [mode, setMode] = useState<AuthFormMode>('login');
  const [inviteRequired, setInviteRequired] = useState(true);
  const [bootstrapRequired, setBootstrapRequired] = useState(false);
  const [isRecoveryComplete, setIsRecoveryComplete] = useState(false);
  const panelTransition = useMotionTransition(MOTION_DURATION_SECONDS.base);
  const fastTransition = useMotionTransition(MOTION_DURATION_SECONDS.fast);
  const authModeKey = isLogoutRecovery ? 'recovery' : mode;
  const title = isLogoutRecovery ? '撤销全部会话' : mode === 'login' ? '登录' : '注册';
  const description = isLogoutRecovery
    ? '重新验证账号后，撤销该账号的所有登录令牌和个人访问令牌'
    : mode === 'login'
      ? '输入账号和密码登录'
      : '创建一个新账号';
  const submitLabel = isSubmitting
    ? '请稍候…'
    : isLogoutRecovery
      ? '重新认证并撤销全部会话'
      : mode === 'login'
        ? '登录'
        : '注册';

  useEffect(() => {
    if (!loading && user && !isLogoutRecovery) {
      router.replace(nextPath);
    }
  }, [isLogoutRecovery, loading, nextPath, router, user]);

  useEffect(() => {
    if (loading || user || isLogoutRecovery) {
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
  }, [isLogoutRecovery, loading, user]);

  const handleSubmit = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    setError(null);
    setIsSubmitting(true);
    setIsRecoveryComplete(false);

    try {
      if (isLogoutRecovery) {
        await recoverLogout(username.trim(), password);
        setIsRecoveryComplete(true);
      } else if (mode === 'register') {
        await register(username.trim(), password, inviteCode.trim());
      } else {
        await login(username.trim(), password);
      }
      if (!isLogoutRecovery) {
        router.replace(nextPath);
      }
    } catch (err) {
      setError(getAuthErrorMessage(err, mode));
    } finally {
      setIsSubmitting(false);
    }
  };

  if (loading || (user && !isLogoutRecovery)) {
    return (
      <main
        id="main-content"
        className="flex min-h-dvh items-center justify-center bg-background px-6"
      >
        <div className="w-full max-w-md">
          <AuthBrand />
          <div
            role="status"
            className="rounded-lg border bg-card px-5 py-4 text-sm text-muted-foreground shadow-vercel-card"
          >
            正在检查登录状态…
          </div>
        </div>
      </main>
    );
  }

  return (
    <main
      id="main-content"
      className="flex min-h-dvh items-center justify-center bg-background px-6"
    >
      <div className="w-full max-w-md">
        <AuthBrand />
        <Card className="gap-0 overflow-hidden border border-border/80 py-0 shadow-lg shadow-black/5 dark:shadow-black/20">
          <CardHeader className="block border-b bg-muted/20 px-6 py-6">
            <CardTitle className="sr-only">{title}</CardTitle>
            <CardDescription className="sr-only">{description}</CardDescription>
            <MotionPresence mode="wait">
              <MotionDiv
                key={authModeKey}
                aria-hidden="true"
                data-auth-header-mode={authModeKey}
                variants={FADE_UP_VARIANTS}
                initial="hidden"
                animate="visible"
                exit={{ opacity: 0, pointerEvents: 'none', y: -3 }}
                transition={fastTransition}
              >
                <p className="text-xs font-medium uppercase tracking-[0.18em] text-muted-foreground">
                  {isLogoutRecovery ? '账户安全' : '欢迎使用'}
                </p>
                <div className="mt-2 text-2xl font-semibold tracking-tight">{title}</div>
                <p className="mt-2 text-sm leading-6 text-muted-foreground">{description}</p>
              </MotionDiv>
            </MotionPresence>
          </CardHeader>
          <CardContent className="py-6">
            {logoutWarning && !isRecoveryComplete && (
              <MotionDiv
                key={`${logoutWarning.occurredAt}-${logoutWarning.requestId ?? 'unknown'}`}
                role="alert"
                data-auth-feedback="logout-warning"
                className="mb-4 rounded-md border border-destructive/30 bg-destructive/10 px-3 py-2 text-sm text-foreground"
                variants={FADE_UP_VARIANTS}
                initial="hidden"
                animate="visible"
                transition={panelTransition}
              >
                <p className="font-medium">服务端会话撤销未确认</p>
                <p className="mt-1 text-xs text-muted-foreground">
                  本地会话信息已清除，但旧令牌可能仍有效。
                  {isLogoutRecovery
                    ? ' 请重新输入账号密码以撤销全部会话。'
                    : ' 请重新认证后撤销全部会话。'}
                </p>
                {logoutWarning.requestId && (
                  <p className="mt-1 break-all text-xs text-muted-foreground">
                    请求 ID：{logoutWarning.requestId}
                  </p>
                )}
                {!isLogoutRecovery && (
                  <Link
                    href="/login?logout_recovery=1"
                    className="motion-control mt-2 inline-flex h-8 items-center rounded-md border border-input bg-background px-3 text-xs font-medium transition-[background-color,color] hover:bg-accent hover:text-accent-foreground"
                  >
                    重新认证并撤销全部会话
                  </Link>
                )}
              </MotionDiv>
            )}
            <MotionPresence mode="wait">
              {isRecoveryComplete ? (
                <MotionDiv
                  key="recovery-complete"
                  data-auth-state="recovery-complete"
                  className="space-y-4"
                  variants={FADE_UP_VARIANTS}
                  initial="hidden"
                  animate="visible"
                  exit={{ opacity: 0, pointerEvents: 'none', y: -4 }}
                  transition={panelTransition}
                >
                  <div
                    role="status"
                    className="rounded-md border border-emerald-500/30 bg-emerald-500/10 px-3 py-2 text-sm text-foreground"
                  >
                    全部会话和个人访问令牌已撤销。现在可以重新登录。
                  </div>
                  <Button asChild className="w-full">
                    <Link href="/login">返回登录</Link>
                  </Button>
                </MotionDiv>
              ) : (
                <MotionForm
                  key="auth-form"
                  data-auth-state="form"
                  aria-label={isLogoutRecovery ? '会话撤销表单' : '身份验证表单'}
                  onSubmit={handleSubmit}
                  className="space-y-4"
                  aria-describedby={error ? 'login-error' : undefined}
                  variants={FADE_UP_VARIANTS}
                  initial="hidden"
                  animate="visible"
                  exit={{ opacity: 0, pointerEvents: 'none', y: -4 }}
                  transition={panelTransition}
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
                  {!isLogoutRecovery && mode === 'register' && inviteRequired && (
                    <MotionDiv
                      key="invite-code"
                      data-auth-conditional="invite-code"
                      className="overflow-hidden"
                      variants={COLLAPSE_VARIANTS}
                      initial="hidden"
                      animate="visible"
                      transition={fastTransition}
                    >
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
                    </MotionDiv>
                  )}
                  {!isLogoutRecovery && mode === 'register' && bootstrapRequired && (
                    <MotionDiv
                      key="bootstrap-required"
                      role="status"
                      data-auth-feedback="bootstrap-required"
                      className="rounded-md border border-amber-500/30 bg-amber-500/10 px-3 py-2 text-sm text-foreground"
                      variants={FADE_UP_VARIANTS}
                      initial="hidden"
                      animate="visible"
                      transition={panelTransition}
                    >
                      系统管理员尚未完成本机初始化。请先在服务器上运行{' '}
                      <code>admin bootstrap --username NAME --password-stdin</code>
                      ，再使用管理员生成的邀请码注册。
                    </MotionDiv>
                  )}
                  {error && (
                    <MotionDiv
                      key={error}
                      id="login-error"
                      role="alert"
                      data-auth-feedback="error"
                      className="rounded-md border border-destructive/30 bg-destructive/10 px-3 py-2 text-sm text-destructive"
                      variants={FADE_UP_VARIANTS}
                      initial="hidden"
                      animate="visible"
                      transition={panelTransition}
                    >
                      {error}
                    </MotionDiv>
                  )}
                  <Button
                    type="submit"
                    aria-label={submitLabel}
                    className="w-full"
                    disabled={
                      isSubmitting ||
                      (!isLogoutRecovery && mode === 'register' && bootstrapRequired)
                    }
                  >
                    <span className="grid" aria-hidden="true">
                      <MotionPresence>
                        <MotionSpan
                          key={submitLabel}
                          className="col-start-1 row-start-1"
                          variants={FADE_UP_VARIANTS}
                          initial="hidden"
                          animate="visible"
                          exit={{ opacity: 0, y: -2 }}
                          transition={fastTransition}
                        >
                          {submitLabel}
                        </MotionSpan>
                      </MotionPresence>
                    </span>
                  </Button>
                </MotionForm>
              )}
            </MotionPresence>
            {!isLogoutRecovery && !isRecoveryComplete && (
              <div
                className="mt-4 text-center text-sm text-muted-foreground"
                data-auth-mode-switch={mode}
              >
                {mode === 'login' ? '没有账号？' : '已有账号？'}{' '}
                <button
                  type="button"
                  className="motion-control font-medium text-foreground underline decoration-muted-foreground underline-offset-4 transition-[color,text-decoration-color] hover:text-primary hover:decoration-primary"
                  onClick={() => {
                    setMode((current) => (current === 'login' ? 'register' : 'login'));
                    setError(null);
                  }}
                >
                  {mode === 'login' ? '注册' : '登录'}
                </button>
              </div>
            )}
          </CardContent>
        </Card>
      </div>
    </main>
  );
}
