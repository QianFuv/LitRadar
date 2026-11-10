import { useState } from 'react';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { Loader2, QrCode } from 'lucide-react';
import { CheckCircle2, RefreshCw, ShieldCheck, Unlink2 } from '@/components/ui/animated-icons';

import {
  ApiError,
  clearCnkiSession,
  getCnkiSession,
  pollCnkiLogin,
  startCnkiLogin,
  type CnkiLoginStartResponse,
  type CnkiSessionStatus,
} from '@/lib/api';
import {
  SettingsGroup,
  SettingsSection,
  SettingsSectionDescription,
  SettingsSectionTitle,
  StatusPill,
} from '@/components/settings/settings-section';
import { Button } from '@/components/ui/button';
import { CopyButton } from '@/components/ui/copy-button';
import { ConfirmDialog } from '@/components/ui/confirm-dialog';
import {
  DISCLOSURE_VARIANTS,
  MotionDiv,
  MotionParagraph,
  MotionPresence,
  MotionSpan,
  REVEAL_VARIANTS,
  SWAP_VARIANTS,
} from '@/components/ui/motion';
import type {
  SettingsCopyFeedback,
  SettingsCopyScope,
} from '@/components/settings/use-settings-copy';
import * as stylex from '@stylexjs/stylex';
import { colors, lineHeights } from '@/lib/theme.stylex';

type CnkiMessageTone = 'error' | 'success' | 'warning';

type CnkiMessageState = {
  text: string;
  tone: CnkiMessageTone;
};

function formatExpiry(ts: number): string {
  return new Date(ts * 1000).toLocaleDateString('zh-CN', {
    year: 'numeric',
    month: '2-digit',
    day: '2-digit',
    hour: '2-digit',
    minute: '2-digit',
  });
}

/**
 * Format an optional Unix timestamp for settings metadata.
 *
 * @param ts - Unix timestamp in seconds.
 * @returns Localized timestamp or empty-state text.
 */
function formatOptionalTime(ts?: number | null): string {
  return ts ? formatExpiry(ts) : '暂无';
}

/**
 * Convert a CNKI session status to compact Chinese UI text.
 *
 * @param session - Safe CNKI session status.
 * @returns Status label.
 */
function getCnkiStatusLabel(session?: CnkiSessionStatus): string {
  if (!session) {
    return '检查中';
  }
  if (session.status === 'active') {
    return '已登录';
  }
  if (session.status === 'expired') {
    return '已过期';
  }
  if (session.status === 'waiting_scan') {
    return '等待扫码';
  }
  return '未配置';
}

/**
 * Select the status pill tone for a CNKI session status.
 *
 * @param session - Safe CNKI session status.
 * @returns Status pill tone.
 */
function getCnkiStatusTone(
  session?: CnkiSessionStatus,
): 'neutral' | 'success' | 'warning' | 'danger' {
  if (!session) {
    return 'neutral';
  }
  if (session.status === 'active') {
    return 'success';
  }
  if (session.status === 'expired') {
    return 'danger';
  }
  if (session.status === 'waiting_scan') {
    return 'warning';
  }
  return 'neutral';
}

/**
 * Check whether a QR payload can be rendered directly as an image source.
 *
 * @param value - QR payload.
 * @returns True when the payload is an image URL or data URI.
 */
function isQrImageSource(value: string): boolean {
  return (
    value.startsWith('data:image/') || value.startsWith('http://') || value.startsWith('https://')
  );
}

/**
 * Pick the styles of a CNKI status message from its tone.
 *
 * @param tone - Message tone.
 * @returns Message styles.
 */
function getCnkiMessageStyle(tone: CnkiMessageTone) {
  if (tone === 'error') {
    return styles.cnkiMessageError;
  }
  if (tone === 'warning') {
    return styles.cnkiMessageWarning;
  }
  return styles.cnkiMessageInfo;
}

/**
 * Convert an unknown CNKI API error into a user-facing status message.
 *
 * @param error - Unknown mutation error.
 * @param fallback - Fallback message.
 * @returns CNKI message state.
 */
function getCnkiApiErrorMessage(error: unknown, fallback: string): CnkiMessageState {
  if (error instanceof ApiError) {
    const code = error.findErrorDetail('code');
    const phase = error.findErrorDetail('phase');
    if (code === 'cnki_login_timeout') {
      return {
        text: '未检测到扫码确认。请确认已在支付宝完成扫码授权，然后再次点击“完成登录”。',
        tone: 'error',
      };
    }
    if (code === 'cnki_login_not_started') {
      return {
        text: '当前没有可确认的二维码。请重新生成二维码后再完成登录。',
        tone: 'error',
      };
    }
    if (code === 'cnki_login_failed' || phase === 'login') {
      return {
        text: `扫码登录未完成：${error.message}`,
        tone: 'error',
      };
    }
    if (code === 'cnki_warmup_failed' || phase === 'warmup') {
      return {
        text: `扫码登录已通过，但全文权限预热失败：${error.message}。请稍后再次点击“完成登录”；如果仍失败，请重新扫码。`,
        tone: 'error',
      };
    }
  }
  return {
    text: error instanceof Error ? error.message : fallback,
    tone: 'error',
  };
}

/**
 * Render and manage the current user's Zhejiang Library CNKI session.
 *
 * @param props - User id plus shared copy feedback/action.
 * @returns CNKI settings card.
 */
export function CnkiSettingsCard(props: {
  userId: number;
  copyFeedback: SettingsCopyFeedback | null;
  handleCopy: (value: string, successMessage: string, scope: SettingsCopyScope) => Promise<void>;
}) {
  const state = useCnkiViewState(props);
  const { cnkiMessage, isCnkiSessionError, cnkiSessionError } = state;
  return (
    <SettingsSection aria-label="浙江图书馆 CNKI">
      <SettingsGroup>
        <div {...stylex.props(styles.cnkiSettingsCardDiv)}>
          <span
            aria-hidden="true"
            {...stylex.props(
              styles.cnkiSettingsCardSpan,
              state.cnkiSession?.status === 'active'
                ? styles.cnkiSettingsCardSpanActive
                : styles.cnkiSettingsCardSpanNotActive,
            )}
          >
            <ShieldCheck {...stylex.props(styles.cnkiSettingsCardShieldCheck)} />
          </span>
          <div {...stylex.props(styles.cnkiSettingsCardDiv2)}>
            <SettingsSectionTitle xstyle={styles.cnkiSettingsCardSettingsSectionTitle}>
              浙江图书馆 CNKI
            </SettingsSectionTitle>
            <SettingsSectionDescription>用于中文数据库文章全文获取</SettingsSectionDescription>
          </div>
          {renderCnkiStatus(state)}
        </div>
        {renderCnkiSessionMetadata(state)}
        {renderCnkiLoginChallenge(state)}
        {(isCnkiSessionError || cnkiMessage) && (
          <div {...stylex.props(styles.cnkiSettingsCardDiv3)}>
            {isCnkiSessionError && (
              <p role="alert" {...stylex.props(styles.cnkiSettingsCardP)}>
                {cnkiSessionError instanceof Error ? cnkiSessionError.message : '获取知网状态失败'}
              </p>
            )}
            <MotionPresence>
              {cnkiMessage && (
                <MotionParagraph
                  key="cnki-message"
                  data-motion-feedback="cnki"
                  role={cnkiMessage.tone === 'error' ? 'alert' : 'status'}
                  {...stylex.props(getCnkiMessageStyle(cnkiMessage.tone))}
                  initial="hidden"
                  animate="visible"
                  exit="exit"
                  variants={REVEAL_VARIANTS}
                >
                  {cnkiMessage.text}
                </MotionParagraph>
              )}
            </MotionPresence>
          </div>
        )}
        {renderCnkiSessionActions(state)}
      </SettingsGroup>
      {renderCnkiClearConfirmation(state)}
    </SettingsSection>
  );
}
/** Own the original settings hooks and exact mutation/cache completion order. */
function useCnkiViewState({
  userId,
  copyFeedback,
  handleCopy,
}: {
  userId: number;
  copyFeedback: SettingsCopyFeedback | null;
  handleCopy: (value: string, successMessage: string, scope: SettingsCopyScope) => Promise<void>;
}) {
  const queryClient = useQueryClient();
  const [cnkiLogin, setCnkiLogin] = useState<CnkiLoginStartResponse | null>(null);
  const [cnkiMessage, setCnkiMessage] = useState<CnkiMessageState | null>(null);
  const [isClearConfirmOpen, setIsClearConfirmOpen] = useState(false);
  const cnkiSessionQueryKey = ['cnki-session', userId] as const;
  const currentCnkiSessionQueryKey = ['cnki-session', 'current'] as const;
  const {
    data: cnkiSession,
    isLoading: isCnkiSessionLoading,
    isError: isCnkiSessionError,
    error: cnkiSessionError,
    refetch: refetchCnkiSession,
  } = useQuery({
    queryKey: cnkiSessionQueryKey,
    queryFn: () => getCnkiSession(),
    enabled: true,
  });
  const startCnkiLoginMut = useMutation({
    mutationFn: () => startCnkiLogin(),
    onSuccess: (data) => {
      setCnkiLogin(data);
      setCnkiMessage(null);
      queryClient.setQueryData(cnkiSessionQueryKey, data.session);
      queryClient.setQueryData(currentCnkiSessionQueryKey, data.session);
      queryClient.removeQueries({ queryKey: ['article-access'] });
      queryClient.invalidateQueries({ queryKey: ['article-access'] });
    },
    onError: (err) => setCnkiMessage(getCnkiApiErrorMessage(err, '启动知网登录失败')),
  });
  const pollCnkiLoginMut = useMutation({
    mutationFn: () => pollCnkiLogin(15, 1.5),
    onMutate: () => setCnkiMessage({ text: '正在确认扫码并预热全文权限…', tone: 'warning' }),
    onSuccess: (data) => {
      setCnkiLogin(null);
      setCnkiMessage({
        text: data.session.status === 'active' ? '登录已完成，全文权限已预热。' : data.status,
        tone: data.session.status === 'active' ? 'success' : 'warning',
      });
      queryClient.setQueryData(cnkiSessionQueryKey, data.session);
      queryClient.setQueryData(currentCnkiSessionQueryKey, data.session);
      queryClient.invalidateQueries({ queryKey: ['cnki-session'] });
      queryClient.removeQueries({ queryKey: ['article-access'] });
      queryClient.invalidateQueries({ queryKey: ['article-access'] });
    },
    onError: (err) => setCnkiMessage(getCnkiApiErrorMessage(err, '确认知网登录失败')),
  });
  const clearCnkiSessionMut = useMutation({
    mutationFn: () => clearCnkiSession(),
    onSuccess: (data) => {
      setIsClearConfirmOpen(false);
      setCnkiLogin(null);
      setCnkiMessage({ text: '登录状态已清除', tone: 'success' });
      queryClient.setQueryData(cnkiSessionQueryKey, data);
      queryClient.setQueryData(currentCnkiSessionQueryKey, data);
      queryClient.invalidateQueries({ queryKey: ['cnki-session'] });
      queryClient.removeQueries({ queryKey: ['article-access'] });
      queryClient.invalidateQueries({ queryKey: ['article-access'] });
    },
    onError: (err) => setCnkiMessage(getCnkiApiErrorMessage(err, '清除知网登录失败')),
  });

  return {
    copyFeedback,
    handleCopy,
    cnkiLogin,
    cnkiMessage,
    setCnkiMessage,
    isClearConfirmOpen,
    setIsClearConfirmOpen,
    cnkiSession,
    isCnkiSessionLoading,
    isCnkiSessionError,
    cnkiSessionError,
    refetchCnkiSession,
    startCnkiLoginMut,
    pollCnkiLoginMut,
    clearCnkiSessionMut,
  };
}
type CnkiViewState = ReturnType<typeof useCnkiViewState>;

/** Retain the loading label and direct keyed status transition. */
function renderCnkiStatus(state: Pick<CnkiViewState, 'cnkiSession' | 'isCnkiSessionLoading'>) {
  const { cnkiSession, isCnkiSessionLoading } = state;

  return (
    <StatusPill
      tone={isCnkiSessionLoading ? 'neutral' : getCnkiStatusTone(cnkiSession)}
      aria-label={isCnkiSessionLoading ? '检查中' : getCnkiStatusLabel(cnkiSession)}
    >
      <MotionPresence mode="wait">
        <MotionSpan
          key={isCnkiSessionLoading ? 'loading' : (cnkiSession?.status ?? 'empty')}
          aria-hidden="true"
          initial="hidden"
          animate="visible"
          exit="exit"
          variants={SWAP_VARIANTS}
        >
          {isCnkiSessionLoading ? '检查中' : getCnkiStatusLabel(cnkiSession)}
        </MotionSpan>
      </MotionPresence>
    </StatusPill>
  );
}

/** Retain safe cookie-name metadata and optional timestamps. */
function renderCnkiSessionMetadata(state: Pick<CnkiViewState, 'cnkiSession'>) {
  const { cnkiSession } = state;

  return (
    <dl {...stylex.props(styles.renderCnkiSessionMetadataDl)}>
      <div
        {...stylex.props(
          styles.renderCnkiSessionMetadataDlItem,
          styles.renderCnkiSessionMetadataDiv,
        )}
      >
        <dt {...stylex.props(styles.renderCnkiSessionMetadataDt)}>有效期</dt>
        <dd {...stylex.props(styles.renderCnkiSessionMetadataDd)}>
          {formatOptionalTime(cnkiSession?.expires_at)}
        </dd>
      </div>
      <div
        {...stylex.props(
          styles.renderCnkiSessionMetadataDlItem,
          styles.renderCnkiSessionMetadataDiv,
        )}
      >
        <dt {...stylex.props(styles.renderCnkiSessionMetadataDt)}>最近使用</dt>
        <dd {...stylex.props(styles.renderCnkiSessionMetadataDd)}>
          {formatOptionalTime(cnkiSession?.last_used_at)}
        </dd>
      </div>
      <div
        {...stylex.props(
          styles.renderCnkiSessionMetadataDlItem,
          styles.renderCnkiSessionMetadataDiv2,
        )}
      >
        <dt {...stylex.props(styles.renderCnkiSessionMetadataDt2)}>Cookie</dt>
        <dd {...stylex.props(styles.renderCnkiSessionMetadataDd2)}>
          {cnkiSession?.cookie_names.length ? cnkiSession.cookie_names.join(', ') : '暂无'}
        </dd>
      </div>
    </dl>
  );
}

/** Retain the complete guarded QR panel and manual poll/copy actions. */
function renderCnkiLoginChallenge(state: CnkiViewState) {
  const { copyFeedback, handleCopy, cnkiLogin, pollCnkiLoginMut } = state;

  return (
    <MotionPresence>
      {cnkiLogin && (
        <MotionDiv
          key="cnki-login"
          data-motion-cnki-login="qr"
          {...stylex.props(styles.renderCnkiLoginChallengeMotionDiv)}
          initial="hidden"
          animate="visible"
          exit="exit"
          variants={DISCLOSURE_VARIANTS}
        >
          <div {...stylex.props(styles.renderCnkiLoginChallengeDiv)}>
            {isQrImageSource(cnkiLogin.qr_code) ? (
              <div
                role="img"
                aria-label="浙江图书馆 CNKI 二维码"
                {...stylex.props(
                  styles.renderCnkiLoginChallengeDiv2,
                  styles.qrCode(`url(${JSON.stringify(cnkiLogin.qr_code)})`),
                )}
              />
            ) : (
              <code {...stylex.props(styles.renderCnkiLoginChallengeCode)}>
                {cnkiLogin.qr_code}
              </code>
            )}
            <div {...stylex.props(styles.renderCnkiLoginChallengeDiv3)}>
              <div {...stylex.props(styles.renderCnkiLoginChallengeDiv4)}>
                <div
                  {...stylex.props(
                    styles.renderCnkiLoginChallengeDivItem,
                    styles.renderCnkiLoginChallengeDiv5,
                  )}
                >
                  扫码登录
                  <StatusPill tone="warning">{cnkiLogin.status || '等待扫码'}</StatusPill>
                </div>
                <ol
                  {...stylex.props(
                    styles.renderCnkiLoginChallengeDivItem,
                    styles.renderCnkiSessionMetadataDt,
                  )}
                >
                  <li
                    {...stylex.props(
                      styles.renderCnkiLoginChallengeOlItem,
                      styles.renderCnkiLoginChallengeLi,
                    )}
                  >
                    <span {...stylex.props(styles.renderCnkiLoginChallengeSpan)}>1</span>
                    使用支付宝扫描左侧二维码并完成授权
                  </li>
                  <li
                    {...stylex.props(
                      styles.renderCnkiLoginChallengeOlItem,
                      styles.renderCnkiLoginChallengeLi,
                    )}
                  >
                    <span {...stylex.props(styles.renderCnkiLoginChallengeSpan)}>2</span>
                    回到这里点击“完成登录”，系统会预热全文权限
                  </li>
                </ol>
              </div>
              <div {...stylex.props(styles.renderCnkiLoginChallengeDiv6)}>
                <Button
                  size="sm"
                  onClick={() => pollCnkiLoginMut.mutate()}
                  disabled={pollCnkiLoginMut.isPending}
                >
                  {pollCnkiLoginMut.isPending ? (
                    <Loader2 {...stylex.props(styles.renderCnkiLoginChallengeLoader2)} />
                  ) : (
                    <CheckCircle2 {...stylex.props(styles.renderCnkiLoginChallengeCheckCircle2)} />
                  )}
                  {pollCnkiLoginMut.isPending ? '确认并预热…' : '完成登录'}
                </Button>
                <CopyButton
                  variant="ghost"
                  size="sm"
                  aria-label="复制 CNKI 登录二维码内容"
                  isCopied={copyFeedback?.scope === 'cnkiQr' && copyFeedback.tone === 'success'}
                  onCopy={() =>
                    void handleCopy(cnkiLogin.qr_code, 'CNKI 登录二维码内容已复制。', 'cnkiQr')
                  }
                >
                  复制
                </CopyButton>
              </div>
              <MotionPresence>
                {copyFeedback?.scope === 'cnkiQr' && (
                  <MotionParagraph
                    key="cnki-copy-feedback"
                    data-motion-feedback="cnki-copy"
                    role={copyFeedback.tone === 'error' ? 'alert' : 'status'}
                    {...stylex.props(
                      copyFeedback.tone === 'error'
                        ? styles.cnkiSettingsCardP
                        : styles.renderCnkiLoginChallengeMotionParagraphNotError,
                    )}
                    initial="hidden"
                    animate="visible"
                    exit="exit"
                    variants={REVEAL_VARIANTS}
                  >
                    {copyFeedback.message}
                  </MotionParagraph>
                )}
              </MotionPresence>
            </div>
          </div>
        </MotionDiv>
      )}
    </MotionPresence>
  );
}

/** Retain independent start, refresh and configured-only clear actions. */
function renderCnkiSessionActions(state: CnkiViewState) {
  const {
    cnkiLogin,
    setCnkiMessage,
    setIsClearConfirmOpen,
    cnkiSession,
    refetchCnkiSession,
    startCnkiLoginMut,
    clearCnkiSessionMut,
  } = state;

  return (
    <div {...stylex.props(styles.renderCnkiSessionActionsDiv)}>
      {cnkiSession?.configured && (
        <Button
          variant="ghost"
          size="sm"
          xstyle={styles.renderCnkiSessionActionsButton}
          onClick={() => {
            clearCnkiSessionMut.reset();
            setCnkiMessage(null);
            setIsClearConfirmOpen(true);
          }}
          disabled={clearCnkiSessionMut.isPending}
        >
          <Unlink2 {...stylex.props(styles.renderCnkiSessionActionsUnlink2)} />
          清除
        </Button>
      )}
      <div {...stylex.props(styles.renderCnkiSessionActionsDiv2)}>
        <Button
          variant="outline"
          size="sm"
          aria-label="刷新 CNKI 登录状态"
          onClick={() => void refetchCnkiSession()}
        >
          <RefreshCw {...stylex.props(styles.renderCnkiSessionActionsUnlink2)} />
          刷新
        </Button>
        <Button
          variant={cnkiSession?.status === 'active' || cnkiLogin ? 'outline' : 'default'}
          size="sm"
          onClick={() => startCnkiLoginMut.mutate()}
          disabled={startCnkiLoginMut.isPending}
        >
          {startCnkiLoginMut.isPending ? (
            <Loader2 {...stylex.props(styles.renderCnkiSessionActionsLoader2)} />
          ) : (
            <QrCode {...stylex.props(styles.renderCnkiSessionActionsUnlink2)} />
          )}
          {cnkiLogin ? '重新生成' : '扫码登录'}
        </Button>
      </div>
    </div>
  );
}

/** Retain pending clear ownership and operation-specific feedback. */
function renderCnkiClearConfirmation(
  state: Pick<
    CnkiViewState,
    'cnkiMessage' | 'isClearConfirmOpen' | 'setIsClearConfirmOpen' | 'clearCnkiSessionMut'
  >,
) {
  const { cnkiMessage, isClearConfirmOpen, setIsClearConfirmOpen, clearCnkiSessionMut } = state;

  return (
    <ConfirmDialog
      open={isClearConfirmOpen}
      onOpenChange={(nextOpen) => {
        if (!clearCnkiSessionMut.isPending) {
          setIsClearConfirmOpen(nextOpen);
        }
      }}
      title="清除 CNKI 登录状态？"
      description="确认清除当前 CNKI 登录状态？之后需要重新扫码才能访问受保护全文。"
      actionLabel="确认清除"
      pendingLabel="清除中…"
      isPending={clearCnkiSessionMut.isPending}
      error={clearCnkiSessionMut.isError ? (cnkiMessage?.text ?? '清除知网登录状态失败') : null}
      onConfirm={() => clearCnkiSessionMut.mutate()}
    />
  );
}

const spinKeyframes = stylex.keyframes({ to: { transform: 'rotate(360deg)' } });

const styles = stylex.create({
  qrCode: (backgroundImage: string) => ({
    backgroundImage,
  }),
  cnkiMessageError: {
    fontSize: '0.875rem',
    lineHeight: lineHeights.textSm,
    color: colors.destructive,
  },
  cnkiMessageWarning: {
    fontSize: '0.875rem',
    lineHeight: lineHeights.textSm,
    color: colors.warningForeground,
  },
  cnkiMessageInfo: {
    fontSize: '0.875rem',
    lineHeight: lineHeights.textSm,
    color: colors.mutedForeground,
  },
  cnkiSettingsCardDiv: {
    display: 'flex',
    alignItems: {
      default: 'flex-start',
      '@media (min-width: 40rem)': 'center',
    },
    gap: '1rem',
    paddingInline: {
      default: '1rem',
      '@media (min-width: 40rem)': '1.25rem',
    },
    paddingBlock: '1.25rem',
  },
  cnkiSettingsCardSpan: {
    display: 'flex',
    width: '3rem',
    height: '3rem',
    flexShrink: 0,
    alignItems: 'center',
    justifyContent: 'center',
    borderRadius: '12px',
  },
  cnkiSettingsCardSpanActive: {
    backgroundColor: colors.success,
    color: colors.successForeground,
  },
  cnkiSettingsCardSpanNotActive: {
    backgroundColor: colors.muted,
    color: colors.mutedForeground,
  },
  cnkiSettingsCardShieldCheck: {
    width: '1.5rem',
    height: '1.5rem',
  },
  cnkiSettingsCardDiv2: {
    minWidth: '0px',
    flex: '1',
  },
  cnkiSettingsCardSettingsSectionTitle: {
    fontSize: '1rem',
    lineHeight: 1.5,
  },
  cnkiSettingsCardDiv3: {
    display: 'flex',
    flexDirection: 'column',
    rowGap: '0.25rem',
    paddingInline: {
      default: '1rem',
      '@media (min-width: 40rem)': '1.25rem',
    },
    paddingBlock: '0.75rem',
  },
  cnkiSettingsCardP: {
    fontSize: '0.875rem',
    lineHeight: lineHeights.textSm,
    color: colors.destructive,
  },
  renderCnkiSessionMetadataDlItem: {
    borderTopStyle: {
      default: null,
      ':not(:last-child)': 'solid',
      '@media (min-width: 40rem)': {
        default: null,
        ':not(:last-child)': 'solid',
      },
    },
    borderTopWidth: {
      default: null,
      ':not(:last-child)': '0px',
      '@media (min-width: 40rem)': {
        default: null,
        ':not(:last-child)': '0px',
      },
    },
    borderBottomStyle: {
      default: null,
      ':not(:last-child)': 'solid',
      '@media (min-width: 40rem)': {
        default: null,
        ':not(:last-child)': 'solid',
      },
    },
    borderBottomWidth: {
      default: null,
      ':not(:last-child)': '1px',
      '@media (min-width: 40rem)': {
        default: null,
        ':not(:last-child)': '0px',
      },
    },
    borderInlineStartStyle: {
      default: null,
      '@media (min-width: 40rem)': {
        default: null,
        ':not(:last-child)': 'solid',
      },
    },
    borderInlineStartWidth: {
      default: null,
      '@media (min-width: 40rem)': {
        default: null,
        ':not(:last-child)': '0px',
      },
    },
    borderInlineEndStyle: {
      default: null,
      '@media (min-width: 40rem)': {
        default: null,
        ':not(:last-child)': 'solid',
      },
    },
    borderInlineEndWidth: {
      default: null,
      '@media (min-width: 40rem)': {
        default: null,
        ':not(:last-child)': '1px',
      },
    },
  },
  renderCnkiSessionMetadataDl: {
    display: 'grid',
    fontSize: '0.875rem',
    lineHeight: lineHeights.textSm,
    gridTemplateColumns: {
      default: null,
      '@media (min-width: 40rem)': '1fr 1fr 1.4fr',
    },
  },
  renderCnkiSessionMetadataDiv: {
    display: {
      default: 'flex',
      '@media (min-width: 40rem)': 'block',
    },
    alignItems: 'center',
    justifyContent: 'space-between',
    gap: '0.75rem',
    paddingInline: {
      default: '1rem',
      '@media (min-width: 40rem)': '1.25rem',
    },
    paddingBlock: '0.75rem',
  },
  renderCnkiSessionMetadataDt: {
    fontSize: '0.75rem',
    lineHeight: lineHeights.textXs,
    color: colors.mutedForeground,
  },
  renderCnkiSessionMetadataDd: {
    fontVariantNumeric: 'tabular-nums',
    marginTop: {
      default: null,
      '@media (min-width: 40rem)': '0.25rem',
    },
  },
  renderCnkiSessionMetadataDiv2: {
    display: {
      default: 'flex',
      '@media (min-width: 40rem)': 'block',
    },
    alignItems: 'flex-start',
    justifyContent: 'space-between',
    gap: '0.75rem',
    paddingInline: {
      default: '1rem',
      '@media (min-width: 40rem)': '1.25rem',
    },
    paddingBlock: '0.75rem',
  },
  renderCnkiSessionMetadataDt2: {
    flexShrink: 0,
    fontSize: '0.75rem',
    lineHeight: lineHeights.textXs,
    color: colors.mutedForeground,
  },
  renderCnkiSessionMetadataDd2: {
    minWidth: '0px',
    textAlign: {
      default: 'right',
      '@media (min-width: 40rem)': 'left',
    },
    wordBreak: 'break-all',
    marginTop: {
      default: null,
      '@media (min-width: 40rem)': '0.25rem',
    },
  },
  renderCnkiLoginChallengeMotionDiv: {
    overflow: 'hidden',
    backgroundColor: `color-mix(in oklab, ${colors.muted} 40%, transparent)`,
  },
  renderCnkiLoginChallengeDiv: {
    display: 'flex',
    flexDirection: {
      default: 'column',
      '@media (min-width: 40rem)': 'row',
    },
    gap: '1.25rem',
    paddingInline: {
      default: '1rem',
      '@media (min-width: 40rem)': '1.25rem',
    },
    paddingBlock: '1.25rem',
    alignItems: {
      default: null,
      '@media (min-width: 40rem)': 'center',
    },
  },
  renderCnkiLoginChallengeDiv2: {
    width: '11rem',
    height: '11rem',
    flexShrink: 0,
    alignSelf: {
      default: 'center',
      '@media (min-width: 40rem)': 'auto',
    },
    borderRadius: '12px',
    borderStyle: 'solid',
    borderWidth: '1px',
    backgroundColor: '#fff',
    backgroundSize: 'contain',
    backgroundPosition: 'center',
    backgroundRepeat: 'no-repeat',
    padding: '0.5rem',
  },
  renderCnkiLoginChallengeCode: {
    maxHeight: '10rem',
    flex: '1',
    overflow: 'auto',
    borderRadius: '0.25rem',
    backgroundColor: colors.muted,
    padding: '0.75rem',
    fontSize: '0.75rem',
    lineHeight: lineHeights.textXs,
    wordBreak: 'break-all',
  },
  renderCnkiLoginChallengeDiv3: {
    display: 'flex',
    minWidth: '0px',
    flex: '1',
    flexDirection: 'column',
    rowGap: '0.75rem',
  },
  renderCnkiLoginChallengeDivItem: {
    marginBottom: {
      default: null,
      ':not(:last-child)': '0.5rem',
    },
  },
  renderCnkiLoginChallengeDiv4: {
    fontSize: '0.875rem',
    lineHeight: lineHeights.textSm,
  },
  renderCnkiLoginChallengeDiv5: {
    display: 'flex',
    alignItems: 'center',
    gap: '0.5rem',
    fontWeight: 700,
  },
  renderCnkiLoginChallengeOlItem: {
    marginBottom: {
      default: null,
      ':not(:last-child)': '0.375rem',
    },
  },
  renderCnkiLoginChallengeLi: {
    display: 'flex',
    gap: '0.5rem',
  },
  renderCnkiLoginChallengeSpan: {
    display: 'flex',
    width: '1rem',
    height: '1rem',
    flexShrink: 0,
    alignItems: 'center',
    justifyContent: 'center',
    borderRadius: 'calc(infinity * 1px)',
    backgroundColor: colors.foreground,
    fontSize: '10px',
    fontWeight: 700,
    color: colors.background,
  },
  renderCnkiLoginChallengeDiv6: {
    display: 'flex',
    flexWrap: 'wrap',
    gap: '0.5rem',
  },
  renderCnkiLoginChallengeLoader2: {
    height: '1rem',
    width: '1rem',
    animationName: spinKeyframes,
    animationDuration: '1s',
    animationTimingFunction: 'linear',
    animationIterationCount: 'infinite',
  },
  renderCnkiLoginChallengeCheckCircle2: {
    height: '1rem',
    width: '1rem',
  },
  renderCnkiLoginChallengeMotionParagraphNotError: {
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
  renderCnkiSessionActionsDiv: {
    display: 'flex',
    flexWrap: 'wrap',
    alignItems: 'center',
    gap: '0.5rem',
    backgroundColor: `color-mix(in oklab, ${colors.muted} 40%, transparent)`,
    paddingInline: {
      default: '1rem',
      '@media (min-width: 40rem)': '1.25rem',
    },
    paddingBlock: '0.75rem',
  },
  renderCnkiSessionActionsButton: {
    color: {
      default: colors.destructive,
      ':hover': {
        default: null,
        '@media (hover: hover)': colors.destructive,
      },
    },
    backgroundColor: {
      default: null,
      ':active': {
        default: colors.accentPressed,
        ':hover': {
          default: null,
          '@media (hover: hover)': colors.accentPressed,
        },
      },
      ':hover': {
        default: null,
        '@media (hover: hover)': `color-mix(in oklab, ${colors.destructive} 10%, transparent)`,
      },
    },
  },
  renderCnkiSessionActionsUnlink2: {
    width: '1rem',
    height: '1rem',
  },
  renderCnkiSessionActionsDiv2: {
    marginLeft: 'auto',
    display: 'flex',
    flexWrap: 'wrap',
    gap: '0.5rem',
  },
  renderCnkiSessionActionsLoader2: {
    width: '1rem',
    height: '1rem',
    animationName: spinKeyframes,
    animationDuration: '1s',
    animationTimingFunction: 'linear',
    animationIterationCount: 'infinite',
  },
});
