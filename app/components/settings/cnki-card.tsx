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
  SettingsSection,
  SettingsSectionContent,
  SettingsSectionDescription,
  SettingsSectionHeader,
  SettingsSectionTitle,
} from '@/components/settings/settings-section';
import { Badge } from '@/components/ui/badge';
import { Button } from '@/components/ui/button';
import { ConfirmDialog } from '@/components/ui/confirm-dialog';
import type {
  SettingsCopyFeedback,
  SettingsCopyScope,
} from '@/components/settings/use-settings-copy';

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
 * Select the badge variant for a CNKI session status.
 *
 * @param session - Safe CNKI session status.
 * @returns Badge variant.
 */
function getCnkiStatusVariant(
  session?: CnkiSessionStatus,
): 'default' | 'secondary' | 'destructive' | 'outline' {
  if (!session) {
    return 'secondary';
  }
  if (session.status === 'active') {
    return 'default';
  }
  if (session.status === 'expired') {
    return 'destructive';
  }
  return 'outline';
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
    return 'text-sm text-amber-700 dark:text-amber-400';
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
export function CnkiSettingsCard({
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

  return (
    <SettingsSection>
      <SettingsSectionHeader>
        <div className="flex flex-col gap-3 sm:flex-row sm:items-start sm:justify-between">
          <div>
            <SettingsSectionTitle className="flex items-center gap-2">
              <ShieldCheck className="h-5 w-5" />
              浙江图书馆 CNKI
            </SettingsSectionTitle>
            <SettingsSectionDescription>用于中文数据库文章全文获取</SettingsSectionDescription>
          </div>
          <Badge variant={getCnkiStatusVariant(cnkiSession)}>
            {isCnkiSessionLoading ? '检查中' : getCnkiStatusLabel(cnkiSession)}
          </Badge>
        </div>
      </SettingsSectionHeader>
      <SettingsSectionContent className="space-y-4">
        <div className="grid gap-3 text-sm sm:grid-cols-2">
          <div className="space-y-1">
            <div className="text-xs text-muted-foreground">有效期</div>
            <div>{formatOptionalTime(cnkiSession?.expires_at)}</div>
          </div>
          <div className="space-y-1">
            <div className="text-xs text-muted-foreground">最近使用</div>
            <div>{formatOptionalTime(cnkiSession?.last_used_at)}</div>
          </div>
          <div className="space-y-1 sm:col-span-2">
            <div className="text-xs text-muted-foreground">Cookie</div>
            <div className="break-all">
              {cnkiSession?.cookie_names.length ? cnkiSession.cookie_names.join(', ') : '暂无'}
            </div>
          </div>
        </div>

        {isCnkiSessionError && (
          <p role="alert" className="text-sm text-destructive">
            {cnkiSessionError instanceof Error ? cnkiSessionError.message : '获取知网状态失败'}
          </p>
        )}

        {cnkiMessage && (
          <p
            role={cnkiMessage.tone === 'error' ? 'alert' : 'status'}
            className={getCnkiMessageClassName(cnkiMessage.tone)}
          >
            {cnkiMessage.text}
          </p>
        )}

        {cnkiLogin && (
          <div className="rounded-md border p-3">
            <div className="flex flex-col gap-3 sm:flex-row sm:items-center">
              {isQrImageSource(cnkiLogin.qr_code) ? (
                <div
                  role="img"
                  aria-label="浙江图书馆 CNKI 二维码"
                  className="h-40 w-40 rounded-md border bg-white bg-contain bg-center bg-no-repeat p-2"
                  style={{ backgroundImage: `url(${JSON.stringify(cnkiLogin.qr_code)})` }}
                />
              ) : (
                <code className="max-h-40 flex-1 overflow-auto rounded bg-muted p-3 text-xs break-all">
                  {cnkiLogin.qr_code}
                </code>
              )}
              <div className="min-w-0 flex-1 space-y-3">
                <div className="space-y-1 text-sm">
                  <div className="font-medium">扫码登录</div>
                  <div className="text-muted-foreground">
                    状态：{cnkiLogin.status || '等待扫码'}
                  </div>
                </div>
                <div className="flex flex-wrap gap-2">
                  <Button
                    variant="outline"
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
                {copyFeedback?.scope === 'cnkiQr' && (
                  <p
                    role={copyFeedback.tone === 'error' ? 'alert' : 'status'}
                    className={
                      copyFeedback.tone === 'error'
                        ? 'text-sm text-destructive'
                        : 'text-sm text-muted-foreground'
                    }
                  >
                    {copyFeedback.message}
                  </p>
                )}
              </div>
            </div>
          </div>
        )}

        <div className="flex flex-wrap gap-2">
          <Button
            variant="outline"
            size="sm"
            onClick={() => startCnkiLoginMut.mutate()}
            disabled={startCnkiLoginMut.isPending}
          >
            {startCnkiLoginMut.isPending ? (
              <Loader2 className="h-4 w-4 animate-spin" />
            ) : (
              <QrCode className="h-4 w-4" />
            )}
            {cnkiLogin ? '重新生成' : '扫码登录'}
          </Button>
          <Button
            variant="outline"
            size="sm"
            aria-label="刷新 CNKI 登录状态"
            onClick={() => void refetchCnkiSession()}
          >
            <RefreshCw className="h-4 w-4" />
            刷新
          </Button>
          {cnkiSession?.configured && (
            <Button
              variant="ghost"
              size="sm"
              className="text-destructive"
              onClick={() => {
                clearCnkiSessionMut.reset();
                setCnkiMessage(null);
                setIsClearConfirmOpen(true);
              }}
              disabled={clearCnkiSessionMut.isPending}
            >
              <Unlink className="h-4 w-4" />
              清除
            </Button>
          )}
        </div>
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
      </SettingsSectionContent>
    </SettingsSection>
  );
}
