/**
 * Public authentication surface for login, registration, and logout recovery.
 */

import { Radar } from 'lucide-react';
import { Eye, EyeOff, Megaphone, Search } from '@/components/ui/animated-icons';
import { Link, useNavigate, useSearchParams } from 'react-router';
import { useEffect, useRef, useState, type ComponentType, type FormEvent } from 'react';
import { Shine } from '@animate-ui/react/primitives/effects/shine';
import { SplittingText } from '@animate-ui/react/primitives/texts/splitting';
import { Button } from '@/components/ui/button';
import { RadarMark } from '@/components/feature/reader-placeholder';
import { Input } from '@/components/ui/input';
import { Label } from '@/components/ui/label';
import {
  DISCLOSURE_VARIANTS,
  MotionDiv,
  MotionForm,
  MotionPresence,
  MotionSpan,
  PANEL_VARIANTS,
  REVEAL_VARIANTS,
  SWAP_VARIANTS,
} from '@/components/ui/motion';
import {
  buildOidcLoginUrl,
  getAuthProviders,
  getInviteRequirement,
  type AuthProviders,
} from '@/lib/api';
import {
  getAuthErrorMessage,
  getNativeOidcErrorMessage,
  getOidcErrorMessage,
  type AuthFormMode,
} from '@/lib/auth-error';
import { useAuth } from '@/lib/auth-context';
import { CLIENT_PLATFORM } from '@/lib/platform';
import { LoadingText } from '@/components/ui/loading-text';
import { useServerConnection } from '@/lib/server-connection';
import { preloadWorkspaceRoutes } from '@/src/routes/lazy-routes';

const LOGIN_RETURN_ORIGIN = 'https://litradar.invalid';

/**
 * Render the compact product identity shown above the form on narrow screens.
 *
 * @returns Compact LitRadar brand mark.
 */
function AuthBrand() {
  return (
    <div className="mb-8 flex items-center gap-3 lg:hidden" aria-label="LitRadar">
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
  );
}

/** Showcase headline lines, each revealed character by character. */
const HEADLINE_LINES = ['让新文献', '主动出现在你的雷达上'] as const;

/** Delay between two headline characters, in seconds. */
const HEADLINE_STAGGER_S = 0.05;

/** Each headline character rises out of a soft blur. */
const HEADLINE_CHARACTER_FROM = { opacity: 0, y: 12, filter: 'blur(6px)' };
const HEADLINE_CHARACTER_TO = { opacity: 1, y: 0, filter: 'blur(0px)' };

/** When the feature icons start playing, after the headline has landed, in milliseconds. */
const FEATURE_ICON_DELAY_MS = 900;

/** Delay between two feature icons, in milliseconds. */
const FEATURE_ICON_STAGGER_MS = 180;

type ShowcaseIcon = ComponentType<{ className?: string; animateOnView?: boolean; delay?: number }>;

/**
 * Draw lucide's radar, which has no Animate UI counterpart, still among the animated icons.
 *
 * @param props - Icon classes; animation props are ignored.
 * @returns Still radar icon.
 */
function StillRadarIcon({ className }: { className?: string }) {
  return <Radar className={className} />;
}

const AUTH_FEATURES: readonly { icon: ShowcaseIcon; title: string; description: string }[] = [
  { icon: Search, title: '全文检索', description: '按期刊评级、领域与时间快速收窄文献范围' },
  { icon: StillRadarIcon, title: '持续追踪', description: '每周新文章按你的研究方向自动推送入库' },
  { icon: Megaphone, title: '征稿日程', description: '集中查看目标期刊的专题征稿与截止时间' },
];

/**
 * Render the dark brand panel shown beside the form on wide screens.
 *
 * @returns Decorative product showcase.
 */
function AuthShowcase() {
  return (
    <aside
      aria-hidden="true"
      className="relative hidden min-h-dvh flex-col justify-between overflow-hidden bg-rail p-12 text-rail-active lg:flex"
    >
      <div className="flex items-center gap-3">
        <img
          src="/litradar-logo.png"
          alt=""
          width={40}
          height={40}
          className="size-10 rounded-xl object-cover"
        />
        <div>
          <p className="text-base font-bold tracking-tight">LitRadar</p>
          <p className="text-xs text-rail-foreground">文献雷达与持续追踪</p>
        </div>
      </div>
      <RadarMark className="pointer-events-none absolute -right-24 top-1/2 size-[34rem] -translate-y-1/2 text-rail-foreground/50" />
      <div className="relative max-w-md">
        <p className="text-[32px] leading-[1.3] font-bold tracking-tight">
          {HEADLINE_LINES.map((line, index) => (
            <SplittingText
              key={line}
              text={line}
              className="block"
              stagger={HEADLINE_STAGGER_S}
              delay={HEADLINE_LINES.slice(0, index).join('').length * HEADLINE_STAGGER_S * 1000}
              initial={HEADLINE_CHARACTER_FROM}
              animate={HEADLINE_CHARACTER_TO}
            />
          ))}
        </p>
        <ul className="mt-10 space-y-5">
          {AUTH_FEATURES.map((feature, index) => {
            const Icon = feature.icon;
            return (
              <li key={feature.title} className="flex items-start gap-4">
                <span className="flex size-9 shrink-0 items-center justify-center rounded-xl bg-rail-hover">
                  <Icon
                    className="size-4"
                    animateOnView
                    delay={FEATURE_ICON_DELAY_MS + index * FEATURE_ICON_STAGGER_MS}
                  />
                </span>
                <span>
                  <span className="block text-sm font-bold">{feature.title}</span>
                  <span className="mt-0.5 block text-[13px] text-rail-foreground">
                    {feature.description}
                  </span>
                </span>
              </li>
            );
          })}
        </ul>
      </div>
      <p className="relative text-xs text-rail-foreground">LitRadar · QianFuv</p>
    </aside>
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
  const state = useLoginViewState();
  return renderLoginPage(state);
}
/** Own the original auth hooks, cancellation effects and captured submit closure in their original order. */
function useLoginViewState() {
  const navigate = useNavigate();
  const [searchParams] = useSearchParams();
  const {
    loading,
    login,
    logoutWarning,
    recoverLogout,
    register,
    signInWithAccessToken,
    signInWithNativeOidc,
    user,
  } = useAuth();
  const serverConnection = useServerConnection();
  const nextParam = searchParams.get('next') || '';
  const nextPath = normalizeLoginReturnPath(nextParam);
  const isLogoutRecovery = searchParams.get('logout_recovery') === '1';
  const oidcErrorCode = searchParams.get('oidc_error');
  const [nativeOidcError, setNativeOidcError] = useState<string | null>(null);
  const [pendingNativeOidc, setPendingNativeOidc] = useState<AbortController | null>(null);
  const pendingNativeOidcRef = useRef<AbortController | null>(null);
  const oidcError = oidcErrorCode === null ? nativeOidcError : getOidcErrorMessage(oidcErrorCode);

  const [username, setUsername] = useState('');
  const [password, setPassword] = useState('');
  const [accessToken, setAccessToken] = useState('');
  const [inviteCode, setInviteCode] = useState('');
  const [email, setEmail] = useState('');
  const [authProviders, setAuthProviders] = useState<AuthProviders | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [isSubmitting, setIsSubmitting] = useState(false);
  const [isPasswordVisible, setIsPasswordVisible] = useState(false);
  const [mode, setMode] = useState<AuthFormMode>('login');
  const [inviteRequired, setInviteRequired] = useState(true);
  const [bootstrapRequired, setBootstrapRequired] = useState(false);
  const [isRecoveryComplete, setIsRecoveryComplete] = useState(false);
  const authModeKey = isLogoutRecovery ? 'recovery' : mode;
  const title = getLoginTitle(isLogoutRecovery, mode);
  const description = getLoginDescription(isLogoutRecovery, mode);
  const submitLabel = getLoginSubmitLabel(isSubmitting, isLogoutRecovery, mode);

  useEffect(() => {
    if (!loading && user && !isLogoutRecovery) {
      navigate(nextPath, { replace: true });
    }
  }, [isLogoutRecovery, loading, navigate, nextPath, user]);

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

  useEffect(() => {
    if (loading || user || isLogoutRecovery) {
      return;
    }
    let didCancel = false;
    getAuthProviders()
      .then((data) => {
        if (!didCancel) {
          setAuthProviders(data);
        }
      })
      .catch(() => {});
    return () => {
      didCancel = true;
    };
  }, [isLogoutRecovery, loading, user]);

  useEffect(() => {
    if (loading || user || isLogoutRecovery || !CLIENT_PLATFORM.nativeSession) {
      return;
    }
    CLIENT_PLATFORM.nativeSession
      .consumeOidcInterruption()
      .then((isInterrupted) => {
        if (isInterrupted) {
          setNativeOidcError(getOidcErrorMessage('interrupted'));
        }
      })
      .catch(() => undefined);
  }, [isLogoutRecovery, loading, user]);

  useEffect(
    () => () => {
      pendingNativeOidcRef.current?.abort();
    },
    [],
  );

  /** Run single sign-on through the native host and the system browser. */
  const startNativeOidc = async () => {
    const controller = new AbortController();
    pendingNativeOidcRef.current = controller;
    setPendingNativeOidc(controller);
    setError(null);
    setNativeOidcError(null);
    preloadWorkspaceRoutes();
    try {
      await signInWithNativeOidc(controller.signal);
      navigate(nextPath, { replace: true });
    } catch (err) {
      if (!controller.signal.aborted) {
        setNativeOidcError(getNativeOidcErrorMessage(err));
      }
    } finally {
      if (pendingNativeOidcRef.current === controller) {
        pendingNativeOidcRef.current = null;
        setPendingNativeOidc(null);
      }
    }
  };

  /** Stop waiting for the browser and return the page to its idle state. */
  const cancelNativeOidc = () => {
    pendingNativeOidcRef.current?.abort();
  };

  const handleSubmit = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    setError(null);
    setIsSubmitting(true);
    setIsRecoveryComplete(false);
    if (!isLogoutRecovery) {
      preloadWorkspaceRoutes();
    }

    try {
      if (isLogoutRecovery) {
        await recoverLogout(username.trim(), password);
        setIsRecoveryComplete(true);
      } else if (mode === 'register') {
        await register(username.trim(), password, inviteCode.trim(), email.trim());
      } else if (mode === 'token') {
        await signInWithAccessToken(accessToken.trim());
      } else {
        await login(username.trim(), password);
      }
      if (!isLogoutRecovery) {
        navigate(nextPath, { replace: true });
      }
    } catch (err) {
      setError(getAuthErrorMessage(err, mode));
    } finally {
      setIsSubmitting(false);
    }
  };

  return {
    loading,
    logoutWarning,
    user,
    isLogoutRecovery,
    username,
    setUsername,
    password,
    setPassword,
    accessToken,
    setAccessToken,
    serverConnection,
    inviteCode,
    setInviteCode,
    email,
    setEmail,
    authProviders,
    nextPath,
    oidcError,
    isNativeOidcPending: pendingNativeOidc !== null,
    startNativeOidc,
    cancelNativeOidc,
    error,
    setError,
    isSubmitting,
    isPasswordVisible,
    setIsPasswordVisible,
    mode,
    setMode,
    inviteRequired,
    bootstrapRequired,
    isRecoveryComplete,
    authModeKey,
    title,
    description,
    submitLabel,
    handleSubmit,
  };
}
type LoginViewState = ReturnType<typeof useLoginViewState>;
/** Retain loading suppression and the editable authentication page as plain JSX. */
function renderLoginPage(state: LoginViewState) {
  const { loading, user, isLogoutRecovery } = state;
  if (loading || (user && !isLogoutRecovery)) {
    return (
      <main
        id="main-content"
        className="grid min-h-dvh bg-background lg:grid-cols-[minmax(0,1.05fr)_minmax(0,1fr)]"
      >
        <AuthShowcase />
        <div className="flex items-center justify-center px-6 py-12">
          <div className="w-full max-w-sm">
            <AuthBrand />
            <div
              role="status"
              className="rounded-xl bg-muted px-5 py-4 text-sm text-muted-foreground"
            >
              <LoadingText text="正在检查登录状态…" />
            </div>
          </div>
        </div>
      </main>
    );
  }

  return (
    <main
      id="main-content"
      className="grid min-h-dvh bg-background lg:grid-cols-[minmax(0,1.05fr)_minmax(0,1fr)]"
    >
      <AuthShowcase />
      <div className="flex items-center justify-center px-6 py-12">
        <div className="w-full max-w-sm">
          <AuthBrand />
          {renderLoginHeader(state)}
          {renderLoginCardContent(state)}
        </div>
      </div>
    </main>
  );
}

/** Retain password bytes, autocomplete, registration minimum and visibility toggle. */
function renderLoginPassword(state: LoginViewState) {
  const { password, setPassword, error, isPasswordVisible, setIsPasswordVisible, mode } = state;

  return (
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
  );
}

/** Retain conditional invite field and captured value. */
function renderLoginInvite(state: LoginViewState) {
  const { isLogoutRecovery, inviteCode, setInviteCode, error, mode, inviteRequired } = state;

  return (
    !isLogoutRecovery &&
    mode === 'register' &&
    inviteRequired && (
      <MotionDiv
        key="invite-code"
        data-auth-conditional="invite-code"
        className="overflow-hidden"
        variants={DISCLOSURE_VARIANTS}
        initial="hidden"
        animate="visible"
        exit="exit"
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
    )
  );
}

/**
 * Render the optional registration email field.
 *
 * @param state - Login view state.
 * @returns Animated email field in registration mode, otherwise false.
 */
function renderLoginEmail(state: LoginViewState) {
  const { isLogoutRecovery, email, setEmail, error, mode } = state;

  return (
    !isLogoutRecovery &&
    mode === 'register' && (
      <MotionDiv
        key="email"
        data-auth-conditional="email"
        className="overflow-hidden"
        variants={DISCLOSURE_VARIANTS}
        initial="hidden"
        animate="visible"
        exit="exit"
      >
        <div className="space-y-2">
          <Label htmlFor="email">邮箱（可选）</Label>
          <Input
            id="email"
            name="email"
            type="email"
            value={email}
            autoComplete="email"
            spellCheck={false}
            onChange={(event) => setEmail(event.target.value)}
            placeholder="可用于登录"
            aria-invalid={Boolean(error)}
            aria-describedby={error ? 'login-error' : undefined}
          />
        </div>
      </MotionDiv>
    )
  );
}

/**
 * Render the single sign-on failure reported by the OIDC callback redirect.
 *
 * @param state - Login view state.
 * @returns Alert describing the `oidc_error` code, or null when absent.
 */
function renderLoginOidcError(state: Pick<LoginViewState, 'isLogoutRecovery' | 'oidcError'>) {
  const { isLogoutRecovery, oidcError } = state;

  return (
    !isLogoutRecovery &&
    oidcError && (
      <MotionDiv
        key="oidc-error"
        role="alert"
        data-auth-feedback="oidc-error"
        className="mb-4 rounded-md border border-destructive/30 bg-destructive/10 px-3 py-2 text-sm text-destructive"
        variants={REVEAL_VARIANTS}
        initial="hidden"
        animate="visible"
      >
        {oidcError}
      </MotionDiv>
    )
  );
}

/**
 * Render the full-page single sign-on entry when the server enables OIDC.
 *
 * @param state - Login view state.
 * @returns Divider and provider link, or null when OIDC is unavailable.
 */
function renderLoginOidc(
  state: Pick<LoginViewState, 'authProviders' | 'isLogoutRecovery' | 'nextPath'>,
) {
  const { authProviders, isLogoutRecovery, nextPath } = state;
  if (isLogoutRecovery || !CLIENT_PLATFORM.supportsSingleSignOn || !authProviders?.oidc_enabled) {
    return null;
  }
  const displayName = authProviders.oidc_display_name.trim() || '单点登录';

  return (
    <div data-auth-oidc="available" className="mt-4 space-y-4">
      <div className="flex items-center gap-3 text-xs text-muted-foreground">
        <span className="h-px flex-1 bg-border" aria-hidden="true" />
        或
        <span className="h-px flex-1 bg-border" aria-hidden="true" />
      </div>
      <Button asChild variant="outline" className="w-full">
        <a href={buildOidcLoginUrl(nextPath)}>使用 {displayName} 登录</a>
      </Button>
    </div>
  );
}

/**
 * Render the native single sign-on entry, its browser wait state, and its cancel action.
 *
 * @param state - Login view state.
 * @returns Divider and sign-on controls, or null outside native hosts or without OIDC.
 */
function renderLoginNativeOidc(
  state: Pick<
    LoginViewState,
    | 'authProviders'
    | 'cancelNativeOidc'
    | 'isLogoutRecovery'
    | 'isNativeOidcPending'
    | 'isSubmitting'
    | 'startNativeOidc'
  >,
) {
  const {
    authProviders,
    cancelNativeOidc,
    isLogoutRecovery,
    isNativeOidcPending,
    isSubmitting,
    startNativeOidc,
  } = state;
  if (
    isLogoutRecovery ||
    CLIENT_PLATFORM.supportsSingleSignOn ||
    !CLIENT_PLATFORM.nativeSession ||
    !authProviders?.oidc_enabled
  ) {
    return null;
  }
  const displayName = authProviders.oidc_display_name.trim() || '单点登录';

  return (
    <div data-auth-oidc={isNativeOidcPending ? 'pending' : 'available'} className="mt-4 space-y-4">
      <div className="flex items-center gap-3 text-xs text-muted-foreground">
        <span className="h-px flex-1 bg-border" aria-hidden="true" />
        或
        <span className="h-px flex-1 bg-border" aria-hidden="true" />
      </div>
      {isNativeOidcPending ? (
        <div className="space-y-3">
          <div
            role="status"
            className="rounded-xl bg-muted px-4 py-3 text-sm leading-6 text-muted-foreground"
          >
            <LoadingText text={`已在浏览器中打开 ${displayName}，完成登录后将自动返回…`} />
          </div>
          <Button type="button" variant="outline" className="w-full" onClick={cancelNativeOidc}>
            取消单点登录
          </Button>
        </div>
      ) : (
        <Button
          type="button"
          variant="outline"
          className="w-full"
          disabled={isSubmitting}
          onClick={() => void startNativeOidc()}
        >
          使用 {displayName} 登录
        </Button>
      )}
    </div>
  );
}

/**
 * Render the access-token sign-in switch shown by hosts without single sign-on.
 *
 * @param state - Login view state.
 * @returns Mode switch and hint, or null when single sign-on is available.
 */
function renderLoginAccessTokenOption(
  state: Pick<
    LoginViewState,
    'authProviders' | 'isLogoutRecovery' | 'mode' | 'setMode' | 'setError'
  >,
) {
  const { authProviders, isLogoutRecovery, mode, setMode, setError } = state;
  if (isLogoutRecovery || CLIENT_PLATFORM.supportsSingleSignOn || mode === 'register') {
    return null;
  }
  const isTokenMode = mode === 'token';

  return (
    <div data-auth-token-option={isTokenMode ? 'active' : 'available'} className="mt-4 space-y-3">
      <div className="flex items-center gap-3 text-xs text-muted-foreground">
        <span className="h-px flex-1 bg-border" aria-hidden="true" />
        或
        <span className="h-px flex-1 bg-border" aria-hidden="true" />
      </div>
      <Button
        type="button"
        variant="outline"
        className="w-full"
        onClick={() => {
          setMode(isTokenMode ? 'login' : 'token');
          setError(null);
        }}
      >
        {isTokenMode ? '使用账号密码登录' : '使用访问令牌登录'}
      </Button>
      <p className="text-xs leading-5 text-muted-foreground">
        {authProviders?.oidc_enabled
          ? '也可以在网页版“访问令牌”设置中创建令牌，再粘贴到此处登录。'
          : '单点登录账号请在网页版“访问令牌”设置中创建令牌，再粘贴到此处登录。'}
      </p>
    </div>
  );
}

/**
 * Render the access-token field used by token sign-in.
 *
 * @param state - Login view state.
 * @returns Token input.
 */
function renderLoginAccessToken(state: LoginViewState) {
  const { accessToken, setAccessToken, error } = state;

  return (
    <div className="space-y-2">
      <Label htmlFor="access-token">访问令牌</Label>
      <Input
        id="access-token"
        name="access_token"
        type="password"
        value={accessToken}
        autoComplete="off"
        autoFocus
        spellCheck={false}
        onChange={(event) => setAccessToken(event.target.value)}
        placeholder="粘贴个人访问令牌"
        aria-invalid={Boolean(error)}
        aria-describedby={error ? 'login-error' : undefined}
        required
      />
    </div>
  );
}

/**
 * Render the connected server and the switch action on native hosts.
 *
 * @param state - Login view state.
 * @returns Server line, or null in the browser.
 */
function renderLoginServer(state: Pick<LoginViewState, 'serverConnection'>) {
  const { serverConnection } = state;
  if (!serverConnection) {
    return null;
  }

  return (
    <p
      data-auth-server={serverConnection.origin}
      className="mt-6 flex flex-wrap items-center justify-center gap-x-2 text-xs text-muted-foreground"
    >
      <span className="break-all">服务器：{serverConnection.origin}</span>
      <button
        type="button"
        className="motion-control font-medium text-foreground underline decoration-muted-foreground underline-offset-4 transition-[color,text-decoration-color] hover:text-primary hover:decoration-primary"
        onClick={() => void serverConnection.switchServer().catch(() => undefined)}
      >
        更换服务器
      </button>
    </p>
  );
}

/** Retain the registration bootstrap guard and exact command spacing. */
function renderLoginBootstrap(
  state: Pick<LoginViewState, 'isLogoutRecovery' | 'mode' | 'bootstrapRequired'>,
) {
  const { isLogoutRecovery, mode, bootstrapRequired } = state;

  return (
    !isLogoutRecovery &&
    mode === 'register' &&
    bootstrapRequired && (
      <MotionDiv
        key="bootstrap-required"
        role="status"
        data-auth-feedback="bootstrap-required"
        className="rounded-md border border-warning-border bg-warning px-3 py-2 text-sm text-foreground"
        variants={REVEAL_VARIANTS}
        initial="hidden"
        animate="visible"
        exit="exit"
      >
        系统管理员尚未完成本机初始化。请先在服务器上运行{' '}
        <code>admin bootstrap --username NAME --password-stdin</code>
        ，再使用管理员生成的邀请码注册。
      </MotionDiv>
    )
  );
}

/** Retain the immediate keyed authentication error and its semantic ID. */
function renderLoginError(state: Pick<LoginViewState, 'error'>) {
  const { error } = state;

  return (
    error && (
      <MotionDiv
        key={error}
        id="login-error"
        role="alert"
        data-auth-feedback="error"
        className="rounded-md border border-destructive/30 bg-destructive/10 px-3 py-2 text-sm text-destructive"
        variants={REVEAL_VARIANTS}
        initial="hidden"
        animate="visible"
        exit="exit"
      >
        {error}
      </MotionDiv>
    )
  );
}

/** Retain warning identity, recovery link and request metadata. */
function renderLoginLogoutWarning(
  state: Pick<LoginViewState, 'logoutWarning' | 'isLogoutRecovery' | 'isRecoveryComplete'>,
) {
  const { logoutWarning, isLogoutRecovery, isRecoveryComplete } = state;

  return (
    logoutWarning &&
    !isRecoveryComplete && (
      <MotionDiv
        key={`${logoutWarning.occurredAt}-${logoutWarning.requestId ?? 'unknown'}`}
        role="alert"
        data-auth-feedback="logout-warning"
        className="mb-4 rounded-md border border-destructive/30 bg-destructive/10 px-3 py-2 text-sm text-foreground"
        variants={REVEAL_VARIANTS}
        initial="hidden"
        animate="visible"
      >
        <p className="font-medium">服务端会话撤销未确认</p>
        <p className="mt-1 text-xs text-muted-foreground">
          本地会话信息已清除，但旧令牌可能仍有效。
          {isLogoutRecovery ? ' 请重新输入账号密码以撤销全部会话。' : ' 请重新认证后撤销全部会话。'}
        </p>
        {logoutWarning.requestId && (
          <p className="mt-1 break-all text-xs text-muted-foreground">
            请求 ID：{logoutWarning.requestId}
          </p>
        )}
        {!isLogoutRecovery && (
          <Link
            to="/login?logout_recovery=1"
            className="motion-control mt-2 inline-flex h-8 items-center rounded-md border border-input bg-background px-3 text-xs font-medium transition-[background-color,color] hover:bg-accent hover:text-accent-foreground"
          >
            重新认证并撤销全部会话
          </Link>
        )}
      </MotionDiv>
    )
  );
}

/** Retain pending/bootstrap admission guards and keyed animated submit label; a sheen sweeps across on hover. */
function renderLoginSubmit(state: LoginViewState) {
  const {
    isLogoutRecovery,
    isNativeOidcPending,
    isSubmitting,
    mode,
    bootstrapRequired,
    submitLabel,
  } = state;

  return (
    <Shine asChild enableOnHover color="var(--primary-foreground)" opacity={0.28}>
      <Button
        type="submit"
        aria-label={submitLabel}
        className="w-full"
        disabled={
          isSubmitting ||
          isNativeOidcPending ||
          (!isLogoutRecovery && mode === 'register' && bootstrapRequired)
        }
      >
        <span className="grid" aria-hidden="true">
          <MotionPresence>
            <MotionSpan
              key={submitLabel}
              className="col-start-1 row-start-1"
              variants={SWAP_VARIANTS}
              initial="hidden"
              animate="visible"
              exit="exit"
            >
              {submitLabel}
            </MotionSpan>
          </MotionPresence>
        </span>
      </Button>
    </Shine>
  );
}

/** Retain the stable auth-form element and input identity across mode switches. */
function renderLoginForm(state: LoginViewState) {
  const { isLogoutRecovery, username, setUsername, error, handleSubmit, mode } = state;
  const isRegistering = !isLogoutRecovery && mode === 'register';

  return (
    <MotionForm
      key="auth-form"
      data-auth-state="form"
      aria-label={isLogoutRecovery ? '会话撤销表单' : '身份验证表单'}
      onSubmit={handleSubmit}
      className="space-y-4"
      aria-describedby={error ? 'login-error' : undefined}
      variants={PANEL_VARIANTS}
      initial="hidden"
      animate="visible"
      exit="exit"
    >
      {!isLogoutRecovery && mode === 'token' ? (
        renderLoginAccessToken(state)
      ) : (
        <>
          <div className="space-y-2">
            <Label htmlFor="username">{isRegistering ? '用户名' : '用户名或邮箱'}</Label>
            <Input
              id="username"
              name="username"
              type="text"
              value={username}
              autoComplete="username"
              autoFocus
              spellCheck={false}
              onChange={(event) => setUsername(event.target.value)}
              placeholder={isRegistering ? '3-32位字母数字下划线' : '输入用户名或邮箱'}
              aria-invalid={Boolean(error)}
              aria-describedby={error ? 'login-error' : undefined}
              required
            />
          </div>
          {renderLoginPassword(state)}
        </>
      )}
      <MotionPresence>{renderLoginEmail(state)}</MotionPresence>
      <MotionPresence>{renderLoginInvite(state)}</MotionPresence>
      <MotionPresence>{renderLoginBootstrap(state)}</MotionPresence>
      <MotionPresence mode="wait">{renderLoginError(state)}</MotionPresence>
      {renderLoginSubmit(state)}
    </MotionForm>
  );
}

/** Retain the mode-keyed visual header and immediate accessible title. */
function renderLoginHeader(
  state: Pick<LoginViewState, 'isLogoutRecovery' | 'authModeKey' | 'title' | 'description'>,
) {
  const { isLogoutRecovery, authModeKey, title, description } = state;

  return (
    <header className="mb-7">
      <h1 className="sr-only">{title}</h1>
      <p className="sr-only">{description}</p>
      <MotionPresence mode="wait">
        <MotionDiv
          key={authModeKey}
          aria-hidden="true"
          data-auth-header-mode={authModeKey}
          variants={PANEL_VARIANTS}
          initial="hidden"
          animate="visible"
          exit="exit"
        >
          <p className="text-xs font-bold tracking-[0.18em] text-primary-text">
            {isLogoutRecovery ? '账户安全' : '欢迎使用 LitRadar'}
          </p>
          <div className="mt-2 text-[28px] leading-tight font-bold tracking-tight">{title}</div>
          <p className="mt-2 text-sm leading-6 text-muted-foreground">{description}</p>
        </MotionDiv>
      </MotionPresence>
    </header>
  );
}

/** Retain recovery result, stable form and original mode-switch event order. */
function renderLoginCardContent(state: LoginViewState) {
  const { isLogoutRecovery, setError, mode, setMode, isRecoveryComplete } = state;

  return (
    <div>
      {renderLoginLogoutWarning(state)}
      {renderLoginOidcError(state)}
      <MotionPresence mode="wait">
        {isRecoveryComplete ? (
          <MotionDiv
            key="recovery-complete"
            data-auth-state="recovery-complete"
            className="space-y-4"
            variants={PANEL_VARIANTS}
            initial="hidden"
            animate="visible"
            exit="exit"
          >
            <div
              role="status"
              className="rounded-md border border-success-border bg-success px-3 py-2 text-sm text-foreground"
            >
              全部会话和个人访问令牌已撤销。现在可以重新登录。
            </div>
            <Button asChild className="w-full">
              <Link to="/login">返回登录</Link>
            </Button>
          </MotionDiv>
        ) : (
          renderLoginForm(state)
        )}
      </MotionPresence>
      {!isRecoveryComplete && renderLoginOidc(state)}
      {!isRecoveryComplete && renderLoginNativeOidc(state)}
      {!isRecoveryComplete && renderLoginAccessTokenOption(state)}
      {!isLogoutRecovery && !isRecoveryComplete && mode !== 'token' && (
        <div
          className="mt-6 border-t pt-5 text-center text-sm text-muted-foreground"
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
      {renderLoginServer(state)}
    </div>
  );
}

/** Resolve the recovery, login and registration title in its original priority. */
function getLoginTitle(isRecovery: boolean, mode: AuthFormMode): string {
  if (isRecovery) return '撤销全部会话';
  if (mode === 'token') return '访问令牌登录';
  return mode === 'login' ? '登录' : '注册';
}
/** Retain the authentication mode's original explanatory text. */
function getLoginDescription(isRecovery: boolean, mode: AuthFormMode): string {
  if (isRecovery) return '重新验证账号后，撤销该账号的所有登录令牌和个人访问令牌';
  if (mode === 'token') return '粘贴在网页版设置中创建的个人访问令牌';
  return mode === 'login' ? '输入用户名或邮箱和密码登录' : '创建一个新账号';
}
/** Resolve pending feedback before recovery and normal authentication labels. */
function getLoginSubmitLabel(
  isSubmitting: boolean,
  isRecovery: boolean,
  mode: AuthFormMode,
): string {
  if (isSubmitting) return '请稍候…';
  if (isRecovery) return '重新认证并撤销全部会话';
  return mode === 'register' ? '注册' : '登录';
}
