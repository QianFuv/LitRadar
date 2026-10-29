'use client';

import { useState } from 'react';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { CheckCircle2, Copy, Loader2, QrCode, RefreshCw, ShieldCheck, Unlink } from 'lucide-react';

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
import { ConfirmDialog } from '@/components/ui/confirm-dialog';
import {
  COLLAPSE_VARIANTS,
  FADE_UP_VARIANTS,
  FADE_VARIANTS,
  MOTION_DURATION_SECONDS,
  MotionDiv,
  MotionParagraph,
  MotionPresence,
  MotionSpan,
  useMotionTransition,
} from '@/components/ui/motion';
import type {
  SettingsCopyFeedback,
  SettingsCopyScope,
} from '@/components/settings/use-settings-copy';
import { cn } from '@/lib/utils';

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
 * Render a CNKI status message style from its tone.
 *
 * @param tone - Message tone.
 * @returns CSS class name.
 */
function getCnkiMessageClassName(tone: CnkiMessageTone): string {
  if (tone === 'error') {
    return 'text-sm text-destructive';
  }
  if (tone === 'warning') {
    return 'text-sm text-warning-foreground';
  }
  return 'text-sm text-muted-foreground';
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
    if (error.code === 'cnki_login_timeout') {
      return {
        text: '未检测到扫码确认。请确认已在支付宝完成扫码授权，然后再次点击“完成登录”。',
        tone: 'error',
      };
    }
    if (error.code === 'cnki_login_not_started') {
      return {
        text: '当前没有可确认的二维码。请重新生成二维码后再完成登录。',
        tone: 'error',
      };
    }
    if (error.code === 'cnki_login_failed' || error.phase === 'login') {
      return {
        text: `扫码登录未完成：${error.message}`,
        tone: 'error',
      };
    }
    if (error.code === 'cnki_warmup_failed' || error.phase === 'warmup') {
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
  const { cnkiMessage, feedbackTransition, isCnkiSessionError, cnkiSessionError } = state;
  return (
    <SettingsSection aria-label="浙江图书馆 CNKI">
      <SettingsGroup>
        <div className="flex items-start gap-4 px-4 py-5 sm:items-center sm:px-5">
          <span
            aria-hidden="true"
            className={cn(
              'flex size-12 shrink-0 items-center justify-center rounded-xl',
              state.cnkiSession?.status === 'active'
                ? 'bg-success text-success-foreground'
                : 'bg-muted text-muted-foreground',
            )}
          >
            <ShieldCheck className="size-6" />
          </span>
          <div className="min-w-0 flex-1">
            <SettingsSectionTitle className="text-base">浙江图书馆 CNKI</SettingsSectionTitle>
            <SettingsSectionDescription>用于中文数据库文章全文获取</SettingsSectionDescription>
          </div>
          {renderCnkiStatus(state)}
        </div>
        {renderCnkiSessionMetadata(state)}
        {renderCnkiLoginChallenge(state)}
        {(isCnkiSessionError || cnkiMessage) && (
          <div className="space-y-1 px-4 py-3 sm:px-5">
            {isCnkiSessionError && (
              <p role="alert" className="text-sm text-destructive">
                {cnkiSessionError instanceof Error ? cnkiSessionError.message : '获取知网状态失败'}
              </p>
            )}
            <MotionPresence>
              {cnkiMessage && (
                <MotionParagraph
                  key="cnki-message"
                  data-motion-feedback="cnki"
                  role={cnkiMessage.tone === 'error' ? 'alert' : 'status'}
                  className={getCnkiMessageClassName(cnkiMessage.tone)}
                  initial="hidden"
                  animate="visible"
                  exit="exit"
                  variants={FADE_UP_VARIANTS}
                  transition={feedbackTransition}
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
  const feedbackTransition = useMotionTransition(MOTION_DURATION_SECONDS.fast);
  const panelTransition = useMotionTransition(MOTION_DURATION_SECONDS.base);
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
    feedbackTransition,
    panelTransition,
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
function renderCnkiStatus(
  state: Pick<CnkiViewState, 'feedbackTransition' | 'cnkiSession' | 'isCnkiSessionLoading'>,
) {
  const { feedbackTransition, cnkiSession, isCnkiSessionLoading } = state;

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
          variants={FADE_VARIANTS}
          transition={feedbackTransition}
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
    <dl className="grid divide-y text-sm sm:grid-cols-[1fr_1fr_1.4fr] sm:divide-x sm:divide-y-0">
      <div className="flex items-center justify-between gap-3 px-4 py-3 sm:block sm:px-5">
        <dt className="text-xs text-muted-foreground">有效期</dt>
        <dd className="tabular-nums sm:mt-1">{formatOptionalTime(cnkiSession?.expires_at)}</dd>
      </div>
      <div className="flex items-center justify-between gap-3 px-4 py-3 sm:block sm:px-5">
        <dt className="text-xs text-muted-foreground">最近使用</dt>
        <dd className="tabular-nums sm:mt-1">{formatOptionalTime(cnkiSession?.last_used_at)}</dd>
      </div>
      <div className="flex items-start justify-between gap-3 px-4 py-3 sm:block sm:px-5">
        <dt className="shrink-0 text-xs text-muted-foreground">Cookie</dt>
        <dd className="min-w-0 text-right break-all sm:mt-1 sm:text-left">
          {cnkiSession?.cookie_names.length ? cnkiSession.cookie_names.join(', ') : '暂无'}
        </dd>
      </div>
    </dl>
  );
}

/** Retain the complete guarded QR panel and manual poll/copy actions. */
function renderCnkiLoginChallenge(state: CnkiViewState) {
  const {
    copyFeedback,
    handleCopy,
    cnkiLogin,
    feedbackTransition,
    panelTransition,
    pollCnkiLoginMut,
  } = state;

  return (
    <MotionPresence>
      {cnkiLogin && (
        <MotionDiv
          key="cnki-login"
          data-motion-cnki-login="qr"
          className="overflow-hidden bg-muted/40"
          initial="hidden"
          animate="visible"
          exit={{ height: 0, opacity: 0, pointerEvents: 'none' }}
          variants={COLLAPSE_VARIANTS}
          transition={panelTransition}
        >
          <div className="flex flex-col gap-5 px-4 py-5 sm:flex-row sm:items-center sm:px-5">
            {isQrImageSource(cnkiLogin.qr_code) ? (
              <div
                role="img"
                aria-label="浙江图书馆 CNKI 二维码"
                className="size-44 shrink-0 self-center rounded-xl border bg-white bg-contain bg-center bg-no-repeat p-2 sm:self-auto"
                style={{ backgroundImage: `url(${JSON.stringify(cnkiLogin.qr_code)})` }}
              />
            ) : (
              <code className="max-h-40 flex-1 overflow-auto rounded bg-muted p-3 text-xs break-all">
                {cnkiLogin.qr_code}
              </code>
            )}
            <div className="min-w-0 flex-1 space-y-3">
              <div className="space-y-2 text-sm">
                <div className="flex items-center gap-2 font-bold">
                  扫码登录
                  <StatusPill tone="warning">{cnkiLogin.status || '等待扫码'}</StatusPill>
                </div>
                <ol className="space-y-1.5 text-xs text-muted-foreground">
                  <li className="flex gap-2">
                    <span className="flex size-4 shrink-0 items-center justify-center rounded-full bg-foreground text-[10px] font-bold text-background">
                      1
                    </span>
                    使用支付宝扫描左侧二维码并完成授权
                  </li>
                  <li className="flex gap-2">
                    <span className="flex size-4 shrink-0 items-center justify-center rounded-full bg-foreground text-[10px] font-bold text-background">
                      2
                    </span>
                    回到这里点击“完成登录”，系统会预热全文权限
                  </li>
                </ol>
              </div>
              <div className="flex flex-wrap gap-2">
                <Button
                  size="sm"
                  onClick={() => pollCnkiLoginMut.mutate()}
                  disabled={pollCnkiLoginMut.isPending}
                >
                  {pollCnkiLoginMut.isPending ? (
                    <Loader2 className="h-4 w-4 animate-spin" />
                  ) : (
                    <CheckCircle2 className="h-4 w-4" />
                  )}
                  {pollCnkiLoginMut.isPending ? '确认并预热…' : '完成登录'}
                </Button>
                <Button
                  variant="ghost"
                  size="sm"
                  aria-label="复制 CNKI 登录二维码内容"
                  onClick={() =>
                    void handleCopy(cnkiLogin.qr_code, 'CNKI 登录二维码内容已复制。', 'cnkiQr')
                  }
                >
                  <Copy className="h-4 w-4" />
                  复制
                </Button>
              </div>
              <MotionPresence>
                {copyFeedback?.scope === 'cnkiQr' && (
                  <MotionParagraph
                    key="cnki-copy-feedback"
                    data-motion-feedback="cnki-copy"
                    role={copyFeedback.tone === 'error' ? 'alert' : 'status'}
                    className={
                      copyFeedback.tone === 'error'
                        ? 'text-sm text-destructive'
                        : 'text-sm text-muted-foreground'
                    }
                    initial="hidden"
                    animate="visible"
                    exit="exit"
                    variants={FADE_UP_VARIANTS}
                    transition={feedbackTransition}
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
    <div className="flex flex-wrap items-center gap-2 bg-muted/40 px-4 py-3 sm:px-5">
      {cnkiSession?.configured && (
        <Button
          variant="ghost"
          size="sm"
          className="text-destructive hover:bg-destructive/10 hover:text-destructive"
          onClick={() => {
            clearCnkiSessionMut.reset();
            setCnkiMessage(null);
            setIsClearConfirmOpen(true);
          }}
          disabled={clearCnkiSessionMut.isPending}
        >
          <Unlink className="size-4" />
          清除
        </Button>
      )}
      <div className="ml-auto flex flex-wrap gap-2">
        <Button
          variant="outline"
          size="sm"
          aria-label="刷新 CNKI 登录状态"
          onClick={() => void refetchCnkiSession()}
        >
          <RefreshCw className="size-4" />
          刷新
        </Button>
        <Button
          variant={cnkiSession?.status === 'active' || cnkiLogin ? 'outline' : 'default'}
          size="sm"
          onClick={() => startCnkiLoginMut.mutate()}
          disabled={startCnkiLoginMut.isPending}
        >
          {startCnkiLoginMut.isPending ? (
            <Loader2 className="size-4 animate-spin" />
          ) : (
            <QrCode className="size-4" />
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
