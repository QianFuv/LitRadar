/**
 * Public authentication surface for login, registration, and logout recovery.
 */

import { Eye, EyeOff, Megaphone, Search, Radar } from '@/components/ui/animated-icons';
import { Link, useNavigate, useSearch } from '@tanstack/react-router';
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
import { LOGIN_ROUTE_ID } from '@/lib/search-params';
import { preloadWorkspaceRoutes } from '@/src/routes/lazy-routes';
import * as stylex from '@stylexjs/stylex';
import { colors, lineHeights } from '@/lib/theme.stylex';

const LOGIN_RETURN_ORIGIN = 'https://litradar.invalid';

/**
 * Render the compact product identity shown above the form on narrow screens.
 *
 * @returns Compact LitRadar brand mark.
 */
function AuthBrand() {
  return (
    <div {...stylex.props(styles.authBrandDiv)} aria-label="LitRadar">
      <img
        src="/litradar-logo.png"
        alt=""
        width={40}
        height={40}
        {...stylex.props(styles.authBrandImg)}
      />
      <div>
        <p {...stylex.props(styles.authBrandP)}>LitRadar</p>
        <p {...stylex.props(styles.authBrandP2)}>文献雷达与持续追踪</p>
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

type ShowcaseIcon = ComponentType<{
  xstyle?: stylex.StyleXStyles;
  animateOnView?: boolean;
  delay?: number;
}>;

/**
 * Draw lucide's radar, which has no Animate UI counterpart, still among the animated icons.
 *
 * @param props - Icon styles; animation props are ignored.
 * @returns Still radar icon.
 */
function StillRadarIcon({ xstyle }: { xstyle?: stylex.StyleXStyles }) {
  return <Radar xstyle={xstyle} />;
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
    <aside aria-hidden="true" {...stylex.props(styles.authShowcaseAside)}>
      <div {...stylex.props(styles.authShowcaseDiv)}>
        <img
          src="/litradar-logo.png"
          alt=""
          width={40}
          height={40}
          {...stylex.props(styles.authBrandImg)}
        />
        <div>
          <p {...stylex.props(styles.authBrandP)}>LitRadar</p>
          <p {...stylex.props(styles.authShowcaseP)}>文献雷达与持续追踪</p>
        </div>
      </div>
      <RadarMark xstyle={styles.authShowcaseRadarMark} />
      <div {...stylex.props(styles.authShowcaseDiv2)}>
        <p {...stylex.props(styles.authShowcaseP2)}>
          {HEADLINE_LINES.map((line, index) => (
            <SplittingText
              key={line}
              text={line}
              {...stylex.props(styles.authShowcaseSplittingText)}
              stagger={HEADLINE_STAGGER_S}
              delay={HEADLINE_LINES.slice(0, index).join('').length * HEADLINE_STAGGER_S * 1000}
              initial={HEADLINE_CHARACTER_FROM}
              animate={HEADLINE_CHARACTER_TO}
            />
          ))}
        </p>
        <ul {...stylex.props(styles.authShowcaseUl)}>
          {AUTH_FEATURES.map((feature, index) => {
            const Icon = feature.icon;
            return (
              <li
                key={feature.title}
                {...stylex.props(styles.authShowcaseUlItem, styles.authShowcaseLi)}
              >
                <span {...stylex.props(styles.authShowcaseSpan)}>
                  <Icon
                    xstyle={styles.authShowcaseIcon}
                    animateOnView
                    delay={FEATURE_ICON_DELAY_MS + index * FEATURE_ICON_STAGGER_MS}
                  />
                </span>
                <span>
                  <span {...stylex.props(styles.authShowcaseSpan2)}>{feature.title}</span>
                  <span {...stylex.props(styles.authShowcaseSpan3)}>{feature.description}</span>
                </span>
              </li>
            );
          })}
        </ul>
      </div>
      <p {...stylex.props(styles.authShowcaseP3)}>LitRadar · QianFuv</p>
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
  const search = useSearch({ from: LOGIN_ROUTE_ID });
  const { loading, login, logoutWarning, recoverLogout, register, signInWithNativeOidc, user } =
    useAuth();
  const serverConnection = useServerConnection();
  const nextParam = search.next || '';
  const nextPath = normalizeLoginReturnPath(nextParam);
  const isLogoutRecovery = search.logout_recovery === '1';
  const oidcErrorCode = search.oidc_error ?? null;
  const [nativeOidcError, setNativeOidcError] = useState<string | null>(null);
  const [pendingNativeOidc, setPendingNativeOidc] = useState<AbortController | null>(null);
  const pendingNativeOidcRef = useRef<AbortController | null>(null);
  const oidcError = oidcErrorCode === null ? nativeOidcError : getOidcErrorMessage(oidcErrorCode);

  const [username, setUsername] = useState('');
  const [password, setPassword] = useState('');
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
      void navigate({ href: nextPath, replace: true });
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
      void navigate({ href: nextPath, replace: true });
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
      } else {
        await login(username.trim(), password);
      }
      if (!isLogoutRecovery) {
        void navigate({ href: nextPath, replace: true });
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
      <main id="main-content" {...stylex.props(styles.renderLoginPageMain)}>
        <AuthShowcase />
        <div {...stylex.props(styles.renderLoginPageDiv)}>
          <div {...stylex.props(styles.renderLoginPageDiv2)}>
            <AuthBrand />
            <output {...stylex.props(styles.statusOutput, styles.renderLoginPageDiv3)}>
              <LoadingText text="正在检查登录状态…" />
            </output>
          </div>
        </div>
      </main>
    );
  }

  return (
    <main id="main-content" {...stylex.props(styles.renderLoginPageMain)}>
      <AuthShowcase />
      <div {...stylex.props(styles.renderLoginPageDiv)}>
        <div {...stylex.props(styles.renderLoginPageDiv2)}>
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
    <div>
      <Label xstyle={styles.renderLoginPasswordDivItem} htmlFor="password">
        密码
      </Label>
      <div {...stylex.props(styles.renderLoginPasswordDivItem, styles.renderLoginPasswordDiv)}>
        <Input
          id="password"
          name="password"
          type={isPasswordVisible ? 'text' : 'password'}
          value={password}
          autoComplete={mode === 'register' ? 'new-password' : 'current-password'}
          onChange={(event) => setPassword(event.target.value)}
          placeholder={mode === 'register' ? '至少12位' : '输入当前密码'}
          minLength={mode === 'register' ? 12 : undefined}
          xstyle={styles.renderLoginPasswordInput}
          aria-invalid={Boolean(error)}
          aria-describedby={error ? 'login-error' : undefined}
          required
        />
        <Button
          type="button"
          variant="ghost"
          size="icon"
          xstyle={styles.renderLoginPasswordButton}
          aria-label={isPasswordVisible ? '隐藏密码' : '显示密码'}
          aria-pressed={isPasswordVisible}
          onClick={() => setIsPasswordVisible((current) => !current)}
        >
          {isPasswordVisible ? (
            <EyeOff xstyle={styles.renderLoginPasswordEyeOff} aria-hidden="true" />
          ) : (
            <Eye xstyle={styles.renderLoginPasswordEyeOff} aria-hidden="true" />
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
        {...stylex.props(styles.renderLoginInviteMotionDiv)}
        variants={DISCLOSURE_VARIANTS}
        initial="hidden"
        animate="visible"
        exit="exit"
      >
        <div>
          <Label xstyle={styles.renderLoginPasswordDivItem} htmlFor="invite-code">
            邀请码
          </Label>
          <Input
            xstyle={styles.renderLoginPasswordDivItem}
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
        {...stylex.props(styles.renderLoginInviteMotionDiv)}
        variants={DISCLOSURE_VARIANTS}
        initial="hidden"
        animate="visible"
        exit="exit"
      >
        <div>
          <Label xstyle={styles.renderLoginPasswordDivItem} htmlFor="email">
            邮箱（可选）
          </Label>
          <Input
            xstyle={styles.renderLoginPasswordDivItem}
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
        {...stylex.props(styles.renderLoginOidcErrorMotionDiv)}
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
    <div data-auth-oidc="available" {...stylex.props(styles.renderLoginOidcDiv)}>
      <div {...stylex.props(styles.renderLoginOidcDivItem, styles.renderLoginOidcDiv2)}>
        <span {...stylex.props(styles.renderLoginOidcSpan)} aria-hidden="true" />
        或
        <span {...stylex.props(styles.renderLoginOidcSpan)} aria-hidden="true" />
      </div>
      <Button
        asChild
        variant="outline"
        xstyle={[styles.renderLoginOidcDivItem, styles.renderLoginOidcButton]}
      >
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
    <div
      data-auth-oidc={isNativeOidcPending ? 'pending' : 'available'}
      {...stylex.props(styles.renderLoginOidcDiv)}
    >
      <div {...stylex.props(styles.renderLoginOidcDivItem, styles.renderLoginOidcDiv2)}>
        <span {...stylex.props(styles.renderLoginOidcSpan)} aria-hidden="true" />
        或
        <span {...stylex.props(styles.renderLoginOidcSpan)} aria-hidden="true" />
      </div>
      {isNativeOidcPending ? (
        <div {...stylex.props(styles.renderLoginOidcDivItem)}>
          <output
            {...stylex.props(
              styles.statusOutput,
              styles.renderLoginNativeOidcDivItem,
              styles.renderLoginNativeOidcDiv,
            )}
          >
            <LoadingText text={`已在浏览器中打开 ${displayName}，完成登录后将自动返回…`} />
          </output>
          <Button
            type="button"
            variant="outline"
            xstyle={[styles.renderLoginNativeOidcDivItem, styles.renderLoginOidcButton]}
            onClick={cancelNativeOidc}
          >
            取消单点登录
          </Button>
        </div>
      ) : (
        <Button
          type="button"
          variant="outline"
          xstyle={[styles.renderLoginOidcDivItem, styles.renderLoginOidcButton]}
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
    <p data-auth-server={serverConnection.origin} {...stylex.props(styles.renderLoginServerP)}>
      <span {...stylex.props(styles.renderLoginServerSpan)}>服务器：{serverConnection.origin}</span>
      <button
        type="button"
        {...stylex.props(styles.renderLoginServerButton)}
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
        data-auth-feedback="bootstrap-required"
        {...stylex.props(styles.renderLoginBootstrapMotionDiv)}
        variants={REVEAL_VARIANTS}
        initial="hidden"
        animate="visible"
        exit="exit"
      >
        <output>
          系统管理员尚未完成本机初始化。请先在服务器上运行{' '}
          <code>admin bootstrap --username NAME --password-stdin</code>
          ，再使用管理员生成的邀请码注册。
        </output>
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
        {...stylex.props(styles.renderLoginErrorMotionDiv)}
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
        {...stylex.props(styles.renderLoginLogoutWarningMotionDiv)}
        variants={REVEAL_VARIANTS}
        initial="hidden"
        animate="visible"
      >
        <p {...stylex.props(styles.renderLoginLogoutWarningP)}>服务端会话撤销未确认</p>
        <p {...stylex.props(styles.renderLoginLogoutWarningP2)}>
          本地会话信息已清除，但旧令牌可能仍有效。
          {isLogoutRecovery ? ' 请重新输入账号密码以撤销全部会话。' : ' 请重新认证后撤销全部会话。'}
        </p>
        {logoutWarning.requestId && (
          <p {...stylex.props(styles.renderLoginLogoutWarningP3)}>
            请求 ID：{logoutWarning.requestId}
          </p>
        )}
        {!isLogoutRecovery && (
          <Link
            to="/login"
            search={{ logout_recovery: '1' }}
            {...stylex.props(styles.renderLoginLogoutWarningLink)}
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
    <Shine asChild enableOnHover color={colors.primaryForeground} opacity={0.28}>
      <Button
        type="submit"
        aria-label={submitLabel}
        xstyle={styles.renderLoginOidcButton}
        disabled={
          isSubmitting ||
          isNativeOidcPending ||
          (!isLogoutRecovery && mode === 'register' && bootstrapRequired)
        }
      >
        <span {...stylex.props(styles.renderLoginSubmitSpan)} aria-hidden="true">
          <MotionPresence>
            <MotionSpan
              key={submitLabel}
              {...stylex.props(styles.renderLoginSubmitMotionSpan)}
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
      {...stylex.props(styles.renderLoginFormMotionForm)}
      aria-describedby={error ? 'login-error' : undefined}
      variants={PANEL_VARIANTS}
      initial="hidden"
      animate="visible"
      exit="exit"
    >
      <div>
        <Label xstyle={styles.renderLoginPasswordDivItem} htmlFor="username">
          {isRegistering ? '用户名' : '用户名或邮箱'}
        </Label>
        <Input
          xstyle={styles.renderLoginPasswordDivItem}
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
    <header {...stylex.props(styles.renderLoginHeaderHeader)}>
      <h1 {...stylex.props(styles.renderLoginHeaderH1)}>{title}</h1>
      <p {...stylex.props(styles.renderLoginHeaderH1)}>{description}</p>
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
          <p {...stylex.props(styles.renderLoginHeaderP)}>
            {isLogoutRecovery ? '账户安全' : '欢迎使用 LitRadar'}
          </p>
          <div {...stylex.props(styles.renderLoginHeaderDiv)}>{title}</div>
          <p {...stylex.props(styles.renderLoginHeaderP2)}>{description}</p>
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

            variants={PANEL_VARIANTS}
            initial="hidden"
            animate="visible"
            exit="exit"
          >
            <output
              {...stylex.props(
                styles.statusOutput,
                styles.renderLoginOidcDivItem,
                styles.renderLoginCardContentDiv,
              )}
            >
              全部会话和个人访问令牌已撤销。现在可以重新登录。
            </output>
            <Button asChild xstyle={[styles.renderLoginOidcDivItem, styles.renderLoginOidcButton]}>
              <Link to="/login">返回登录</Link>
            </Button>
          </MotionDiv>
        ) : (
          renderLoginForm(state)
        )}
      </MotionPresence>
      {!isRecoveryComplete && renderLoginOidc(state)}
      {!isRecoveryComplete && renderLoginNativeOidc(state)}
      {!isLogoutRecovery && !isRecoveryComplete && (
        <div {...stylex.props(styles.renderLoginCardContentDiv2)} data-auth-mode-switch={mode}>
          {mode === 'login' ? '没有账号？' : '已有账号？'}{' '}
          <button
            type="button"
            {...stylex.props(styles.renderLoginServerButton)}
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
  return mode === 'login' ? '登录' : '注册';
}
/** Retain the authentication mode's original explanatory text. */
function getLoginDescription(isRecovery: boolean, mode: AuthFormMode): string {
  if (isRecovery) return '重新验证账号后，撤销该账号的所有登录令牌和个人访问令牌';
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

const styles = stylex.create({
  statusOutput: {
    display: 'block',
  },
  authBrandDiv: {
    marginBottom: '2rem',
    display: {
      default: 'flex',
      '@media (min-width: 64rem)': 'none',
    },
    alignItems: 'center',
    gap: '0.75rem',
  },
  authBrandImg: {
    width: '2.5rem',
    height: '2.5rem',
    borderRadius: '12px',
    objectFit: 'cover',
  },
  authBrandP: {
    fontSize: '1rem',
    lineHeight: 1.5,
    fontWeight: 700,
    letterSpacing: '-0.025em',
  },
  authBrandP2: {
    fontSize: '0.75rem',
    lineHeight: lineHeights.textXs,
    color: colors.mutedForeground,
  },
  authShowcaseAside: {
    position: 'relative',
    display: {
      default: 'none',
      '@media (min-width: 64rem)': 'flex',
    },
    minHeight: '100dvh',
    flexDirection: 'column',
    justifyContent: 'space-between',
    overflow: 'hidden',
    backgroundColor: colors.rail,
    padding: '3rem',
    color: colors.railActive,
  },
  authShowcaseDiv: {
    display: 'flex',
    alignItems: 'center',
    gap: '0.75rem',
  },
  authShowcaseP: {
    fontSize: '0.75rem',
    lineHeight: lineHeights.textXs,
    color: colors.railForeground,
  },
  authShowcaseRadarMark: {
    pointerEvents: 'none',
    position: 'absolute',
    top: 'calc(1 / 2 * 100%)',
    right: '-6rem',
    width: '34rem',
    height: '34rem',
    translate: '0 calc(calc(1 / 2 * 100%) * -1)',
    color: `color-mix(in oklab, ${colors.railForeground} 50%, transparent)`,
  },
  authShowcaseDiv2: {
    position: 'relative',
    maxWidth: '28rem',
  },
  authShowcaseP2: {
    fontSize: '32px',
    lineHeight: 1.3,
    fontWeight: 700,
    letterSpacing: '-0.025em',
  },
  authShowcaseSplittingText: {
    display: 'block',
  },
  authShowcaseUlItem: {
    marginBottom: {
      default: null,
      ':not(:last-child)': '1.25rem',
    },
  },
  authShowcaseUl: {
    marginTop: '2.5rem',
  },
  authShowcaseLi: {
    display: 'flex',
    alignItems: 'flex-start',
    gap: '1rem',
  },
  authShowcaseSpan: {
    display: 'flex',
    width: '2.25rem',
    height: '2.25rem',
    flexShrink: 0,
    alignItems: 'center',
    justifyContent: 'center',
    borderRadius: '12px',
    backgroundColor: colors.railHover,
  },
  authShowcaseIcon: {
    width: '1rem',
    height: '1rem',
  },
  authShowcaseSpan2: {
    display: 'block',
    fontSize: '0.875rem',
    lineHeight: lineHeights.textSm,
    fontWeight: 700,
  },
  authShowcaseSpan3: {
    marginTop: '0.125rem',
    display: 'block',
    fontSize: '13px',
    color: colors.railForeground,
  },
  authShowcaseP3: {
    position: 'relative',
    fontSize: '0.75rem',
    lineHeight: lineHeights.textXs,
    color: colors.railForeground,
  },
  renderLoginPageMain: {
    display: 'grid',
    minHeight: '100dvh',
    backgroundColor: colors.background,
    gridTemplateColumns: {
      default: null,
      '@media (min-width: 64rem)': 'minmax(0,1.05fr) minmax(0,1fr)',
    },
  },
  renderLoginPageDiv: {
    display: 'flex',
    alignItems: 'center',
    justifyContent: 'center',
    paddingInline: '1.5rem',
    paddingBlock: '3rem',
  },
  renderLoginPageDiv2: {
    width: '100%',
    maxWidth: '24rem',
  },
  renderLoginPageDiv3: {
    borderRadius: '12px',
    backgroundColor: colors.muted,
    paddingInline: '1.25rem',
    paddingBlock: '1rem',
    fontSize: '0.875rem',
    lineHeight: lineHeights.textSm,
    color: colors.mutedForeground,
  },
  renderLoginPasswordDivItem: {
    marginBottom: {
      default: null,
      ':not(:last-child)': '0.5rem',
    },
  },
  renderLoginPasswordDiv: {
    position: 'relative',
  },
  renderLoginPasswordInput: {
    paddingRight: '2.5rem',
  },
  renderLoginPasswordButton: {
    position: 'absolute',
    insetBlock: '0px',
    right: '0px',
    height: '100%',
    borderTopLeftRadius: 0,
    borderBottomLeftRadius: 0,
    color: {
      default: colors.mutedForeground,
      ':hover': {
        default: null,
        '@media (hover: hover)': colors.foreground,
      },
    },
  },
  renderLoginPasswordEyeOff: {
    height: '1rem',
    width: '1rem',
  },
  renderLoginInviteMotionDiv: {
    overflow: 'hidden',
  },
  renderLoginOidcErrorMotionDiv: {
    marginBottom: '1rem',
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
  renderLoginOidcDivItem: {
    marginBottom: {
      default: null,
      ':not(:last-child)': '1rem',
    },
  },
  renderLoginOidcDiv: {
    marginTop: '1rem',
  },
  renderLoginOidcDiv2: {
    display: 'flex',
    alignItems: 'center',
    gap: '0.75rem',
    fontSize: '0.75rem',
    lineHeight: lineHeights.textXs,
    color: colors.mutedForeground,
  },
  renderLoginOidcSpan: {
    height: '1px',
    flex: '1',
    backgroundColor: colors.border,
  },
  renderLoginOidcButton: {
    width: '100%',
  },
  renderLoginNativeOidcDivItem: {
    marginBottom: {
      default: null,
      ':not(:last-child)': '0.75rem',
    },
  },
  renderLoginNativeOidcDiv: {
    borderRadius: '12px',
    backgroundColor: colors.muted,
    paddingInline: '1rem',
    paddingBlock: '0.75rem',
    fontSize: '0.875rem',
    lineHeight: '1.5rem',
    color: colors.mutedForeground,
  },
  renderLoginServerP: {
    marginTop: '1.5rem',
    display: 'flex',
    flexWrap: 'wrap',
    alignItems: 'center',
    justifyContent: 'center',
    columnGap: '0.5rem',
    fontSize: '0.75rem',
    lineHeight: lineHeights.textXs,
    color: colors.mutedForeground,
  },
  renderLoginServerSpan: {
    wordBreak: 'break-all',
  },
  renderLoginServerButton: {
    fontWeight: 500,
    color: {
      default: colors.foreground,
      ':hover': {
        default: null,
        '@media (hover: hover)': colors.primary,
      },
    },
    textDecorationLine: 'underline',
    textDecorationColor: {
      default: colors.mutedForeground,
      ':hover': {
        default: null,
        '@media (hover: hover)': colors.primary,
      },
    },
    textUnderlineOffset: '4px',
    transitionProperty: 'color, text-decoration-color',
    transitionTimingFunction: 'cubic-bezier(0.25, 0.1, 0.25, 1)',
    transitionDuration: '200ms',
  },
  renderLoginBootstrapMotionDiv: {
    borderRadius: '6px',
    borderStyle: 'solid',
    borderWidth: '1px',
    borderColor: colors.warningBorder,
    backgroundColor: colors.warning,
    paddingInline: '0.75rem',
    paddingBlock: '0.5rem',
    fontSize: '0.875rem',
    lineHeight: lineHeights.textSm,
    color: colors.foreground,
  },
  renderLoginErrorMotionDiv: {
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
  renderLoginLogoutWarningMotionDiv: {
    marginBottom: '1rem',
    borderRadius: '6px',
    borderStyle: 'solid',
    borderWidth: '1px',
    borderColor: `color-mix(in oklab, ${colors.destructive} 30%, transparent)`,
    backgroundColor: `color-mix(in oklab, ${colors.destructive} 10%, transparent)`,
    paddingInline: '0.75rem',
    paddingBlock: '0.5rem',
    fontSize: '0.875rem',
    lineHeight: lineHeights.textSm,
    color: colors.foreground,
  },
  renderLoginLogoutWarningP: {
    fontWeight: 500,
  },
  renderLoginLogoutWarningP2: {
    marginTop: '0.25rem',
    fontSize: '0.75rem',
    lineHeight: lineHeights.textXs,
    color: colors.mutedForeground,
  },
  renderLoginLogoutWarningP3: {
    marginTop: '0.25rem',
    fontSize: '0.75rem',
    lineHeight: lineHeights.textXs,
    wordBreak: 'break-all',
    color: colors.mutedForeground,
  },
  renderLoginLogoutWarningLink: {
    marginTop: '0.5rem',
    display: 'inline-flex',
    height: '2rem',
    alignItems: 'center',
    borderRadius: '6px',
    borderStyle: 'solid',
    borderWidth: '1px',
    borderColor: colors.input,
    backgroundColor: {
      default: colors.background,
      ':hover': {
        default: null,
        '@media (hover: hover)': colors.accent,
      },
    },
    paddingInline: '0.75rem',
    fontSize: '0.75rem',
    lineHeight: lineHeights.textXs,
    fontWeight: 500,
    transitionProperty: 'background-color, color',
    transitionTimingFunction: 'cubic-bezier(0.25, 0.1, 0.25, 1)',
    transitionDuration: '200ms',
    color: {
      default: null,
      ':hover': {
        default: null,
        '@media (hover: hover)': colors.accentForeground,
      },
    },
  },
  renderLoginSubmitSpan: {
    display: 'grid',
  },
  renderLoginSubmitMotionSpan: {
    gridColumnStart: 1,
    gridRowStart: 1,
  },
  renderLoginFormMotionForm: {
    display: 'flex',
    flexDirection: 'column',
    rowGap: '1rem',
  },
  renderLoginHeaderHeader: {
    marginBottom: '1.75rem',
  },
  renderLoginHeaderH1: {
    position: 'absolute',
    width: '1px',
    height: '1px',
    padding: 0,
    margin: '-1px',
    overflow: 'hidden',
    clipPath: 'inset(50%)',
    whiteSpace: 'nowrap',
    borderWidth: 0,
  },
  renderLoginHeaderP: {
    fontSize: '0.75rem',
    lineHeight: lineHeights.textXs,
    fontWeight: 700,
    letterSpacing: '0.18em',
    color: colors.primaryText,
  },
  renderLoginHeaderDiv: {
    marginTop: '0.5rem',
    fontSize: '28px',
    lineHeight: 1.25,
    fontWeight: 700,
    letterSpacing: '-0.025em',
  },
  renderLoginHeaderP2: {
    marginTop: '0.5rem',
    fontSize: '0.875rem',
    lineHeight: '1.5rem',
    color: colors.mutedForeground,
  },
  renderLoginCardContentDiv: {
    borderRadius: '6px',
    borderStyle: 'solid',
    borderWidth: '1px',
    borderColor: colors.successBorder,
    backgroundColor: colors.success,
    paddingInline: '0.75rem',
    paddingBlock: '0.5rem',
    fontSize: '0.875rem',
    lineHeight: lineHeights.textSm,
    color: colors.foreground,
  },
  renderLoginCardContentDiv2: {
    marginTop: '1.5rem',
    borderTopStyle: 'solid',
    borderTopWidth: '1px',
    paddingTop: '1.25rem',
    textAlign: 'center',
    fontSize: '0.875rem',
    lineHeight: lineHeights.textSm,
    color: colors.mutedForeground,
  },
});
