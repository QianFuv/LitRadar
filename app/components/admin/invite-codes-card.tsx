'use client';

import { useMemo, useState } from 'react';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { Ban, Copy, Plus, Ticket } from 'lucide-react';

import {
  adminCreateInviteCode,
  adminGetInviteCodes,
  adminRevokeInviteCode,
  type AdminInviteCode,
  type InviteCodeStatus,
} from '@/lib/api';
import { copyTextToClipboard } from '@/lib/clipboard';
import { AdminListHeader } from '@/components/admin/admin-list-header';
import { SettingsGroup, StatusPill } from '@/components/settings/settings-section';
import { Button } from '@/components/ui/button';
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/components/ui/card';
import { ConfirmDialog } from '@/components/ui/confirm-dialog';
import { Input } from '@/components/ui/input';
import {
  COLLAPSE_VARIANTS,
  FADE_UP_VARIANTS,
  MOTION_DURATION_SECONDS,
  MotionDiv,
  MotionParagraph,
  MotionPresence,
  useMotionTransition,
} from '@/components/ui/motion';
import { cn } from '@/lib/utils';

const INVITE_STATUS_LABELS: Record<InviteCodeStatus, string> = {
  active: '可用',
  expired: '已过期',
  revoked: '已撤销',
  exhausted: '已用尽',
};

/**
 * Format a Unix timestamp for administrator tables.
 *
 * @param timestamp - Unix timestamp in seconds.
 * @returns Localized date and time.
 */
function formatDate(timestamp: number): string {
  return new Date(timestamp * 1000).toLocaleString('zh-CN');
}

/**
 * Render administrator invite-code creation and lifecycle management.
 *
 * @param props - Whether administrator queries may run.
 * @returns Invite-code management card.
 */
export function AdminInviteCodesCard(props: { isEnabled: boolean }) {
  const state = useAdminInviteViewState(props);
  const {
    validDays,
    setValidDays,
    maxUses,
    setMaxUses,
    isPolicyValid,
    feedbackTransition,
    inviteCodesError,
    isLoading,
    createCodeMut,
  } = state;
  return (
    <Card>
      <CardHeader>
        <CardTitle className="flex items-center gap-2">
          <Ticket className="h-5 w-5" />
          邀请码管理
        </CardTitle>
        <CardDescription>创建有期限和使用次数上限的邀请码，并保留撤销历史</CardDescription>
      </CardHeader>
      <CardContent className="space-y-4">
        <SettingsGroup>
          <div className="flex flex-col gap-4 px-4 py-4 sm:px-5 lg:flex-row lg:items-end lg:justify-between">
            <div className="flex items-center gap-3">
              <span className="flex size-10 shrink-0 items-center justify-center rounded-xl bg-primary text-primary-foreground">
                <Ticket className="size-5" aria-hidden="true" />
              </span>
              <div>
                <p className="text-sm font-bold">生成新邀请码</p>
                <p className="text-xs text-muted-foreground">设置有效期与可注册次数</p>
              </div>
            </div>
            <div className="flex flex-wrap items-end gap-3">
              <label className="space-y-1" htmlFor="invite-valid-days">
                <span className="block text-xs text-muted-foreground">有效天数</span>
                <Input
                  id="invite-valid-days"
                  type="number"
                  min={1}
                  max={365}
                  className="w-24 tabular-nums"
                  value={validDays}
                  onChange={(event) => setValidDays(event.target.value)}
                />
              </label>
              <label className="space-y-1" htmlFor="invite-max-uses">
                <span className="block text-xs text-muted-foreground">最多使用次数</span>
                <Input
                  id="invite-max-uses"
                  type="number"
                  min={1}
                  max={1000}
                  className="w-24 tabular-nums"
                  value={maxUses}
                  onChange={(event) => setMaxUses(event.target.value)}
                />
              </label>
              <Button
                className="flex-1 sm:flex-none"
                onClick={() => createCodeMut.mutate()}
                disabled={createCodeMut.isPending || !isPolicyValid}
              >
                <Plus className="size-4" />
                生成邀请码
              </Button>
            </div>
          </div>
        </SettingsGroup>
        {isLoading && (
          <p role="status" className="text-sm text-muted-foreground">
            加载中…
          </p>
        )}
        {inviteCodesError instanceof Error && (
          <p role="alert" className="text-sm text-destructive">
            {inviteCodesError.message}
          </p>
        )}
        <MotionPresence>
          {createCodeMut.error instanceof Error && (
            <MotionParagraph
              key="invite-create-error"
              data-motion-feedback="invite-create"
              role="alert"
              className="text-sm text-destructive"
              initial="hidden"
              animate="visible"
              exit="exit"
              variants={FADE_UP_VARIANTS}
              transition={feedbackTransition}
            >
              {createCodeMut.error.message}
            </MotionParagraph>
          )}
        </MotionPresence>
        {renderAdminInviteCopyFeedback(state)}
        {state.inviteRows.length > 0 && (
          <AdminListHeader
            summary={`共 ${state.inviteRows.length} 个邀请码 · ${
              state.inviteRows.filter(({ inviteCode }) => inviteCode.status === 'active').length
            } 个可用`}
          />
        )}
        {renderAdminInviteList(state)}
        {renderAdminInviteRevocation(state)}
      </CardContent>
    </Card>
  );
}
/** Own the original hooks, draft stores and exact completion/cache order. */
function useAdminInviteViewState({ isEnabled }: { isEnabled: boolean }) {
  const queryClient = useQueryClient();
  const [validDays, setValidDays] = useState('7');
  const [maxUses, setMaxUses] = useState('1');
  const validDaysValue = Number(validDays);
  const maxUsesValue = Number(maxUses);
  const isPolicyValid =
    Number.isInteger(validDaysValue) &&
    validDaysValue >= 1 &&
    validDaysValue <= 365 &&
    Number.isInteger(maxUsesValue) &&
    maxUsesValue >= 1 &&
    maxUsesValue <= 1000;
  const [copyFeedback, setCopyFeedback] = useState<{
    message: string;
    tone: 'error' | 'success';
  } | null>(null);
  const [inviteCodeToRevoke, setInviteCodeToRevoke] = useState<AdminInviteCode | null>(null);
  const feedbackTransition = useMotionTransition(MOTION_DURATION_SECONDS.fast);
  const rowTransition = useMotionTransition(MOTION_DURATION_SECONDS.base);
  const {
    data: inviteCodes,
    error: inviteCodesError,
    isLoading,
  } = useQuery({
    queryKey: ['admin-invite-codes'],
    queryFn: () => adminGetInviteCodes(),
    enabled: isEnabled,
  });
  const inviteRows = useMemo(
    () =>
      (inviteCodes ?? []).map((inviteCode) => ({
        inviteCode,
        expiresAtLabel: formatDate(inviteCode.expires_at),
        createdAtLabel: formatDate(inviteCode.created_at),
      })),
    [inviteCodes],
  );
  const createCodeMut = useMutation({
    mutationFn: () =>
      adminCreateInviteCode({
        expires_at: Date.now() / 1000 + validDaysValue * 24 * 60 * 60,
        max_uses: maxUsesValue,
      }),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: ['admin-invite-codes'] });
      void queryClient.invalidateQueries({ queryKey: ['admin-stats'] });
    },
  });
  const revokeCodeMut = useMutation({
    mutationFn: (codeId: number) => adminRevokeInviteCode(codeId),
    onSuccess: (_data, codeId) => {
      void queryClient.invalidateQueries({ queryKey: ['admin-invite-codes'] });
      void queryClient.invalidateQueries({ queryKey: ['admin-stats'] });
      setInviteCodeToRevoke((current) => (current?.id === codeId ? null : current));
    },
  });

  /** Copy an invite code and surface deterministic feedback. */
  const handleCopyInviteCode = async (code: string) => {
    try {
      await copyTextToClipboard(code);
      setCopyFeedback({ message: '邀请码已复制。', tone: 'success' });
    } catch {
      setCopyFeedback({ message: '复制失败，请手动选择文本复制。', tone: 'error' });
    }
    setTimeout(() => setCopyFeedback(null), 3000);
  };

  return {
    validDays,
    setValidDays,
    maxUses,
    setMaxUses,
    isPolicyValid,
    copyFeedback,
    inviteCodeToRevoke,
    setInviteCodeToRevoke,
    feedbackTransition,
    rowTransition,
    inviteRows,
    inviteCodesError,
    isLoading,
    createCodeMut,
    revokeCodeMut,
    handleCopyInviteCode,
  };
}
type AdminInviteViewState = ReturnType<typeof useAdminInviteViewState>;

/** Retain accessible clipboard outcomes and unchanged feedback timer ownership. */
function renderAdminInviteCopyFeedback(
  state: Pick<AdminInviteViewState, 'copyFeedback' | 'feedbackTransition'>,
) {
  const { copyFeedback, feedbackTransition } = state;

  return (
    <MotionPresence>
      {copyFeedback && (
        <MotionParagraph
          key="invite-copy-feedback"
          data-motion-feedback="invite-copy"
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
  );
}

/** Retain full-code copying, lifecycle metadata and revoke admission in one list. */
function renderAdminInviteList(state: AdminInviteViewState) {
  const {
    setInviteCodeToRevoke,
    feedbackTransition,
    rowTransition,
    inviteRows,
    revokeCodeMut,
    handleCopyInviteCode,
  } = state;

  return (
    <SettingsGroup>
      <MotionPresence>
        {inviteRows.length === 0 ? (
          <MotionDiv
            key="empty-invite-codes"
            className="px-5 py-10 text-center text-sm text-muted-foreground"
            initial="hidden"
            animate="visible"
            exit="exit"
            variants={FADE_UP_VARIANTS}
            transition={feedbackTransition}
          >
            暂无邀请码
          </MotionDiv>
        ) : (
          inviteRows.map(({ inviteCode, expiresAtLabel, createdAtLabel }) => {
            const isActive = inviteCode.status === 'active';
            const usageRatio =
              inviteCode.max_uses > 0 ? Math.min(1, inviteCode.use_count / inviteCode.max_uses) : 0;
            return (
              <MotionDiv
                key={inviteCode.id}
                data-motion-invite-key={inviteCode.id}
                className="content-visibility-card flex flex-col gap-3 overflow-hidden px-4 py-4 sm:px-5 md:flex-row md:items-center md:gap-5"
                initial="hidden"
                animate="visible"
                exit={{ height: 0, opacity: 0, pointerEvents: 'none' }}
                variants={COLLAPSE_VARIANTS}
                transition={rowTransition}
              >
                <div className="flex min-w-0 flex-1 items-start gap-3">
                  <span
                    aria-hidden="true"
                    className={cn(
                      'mt-0.5 flex size-9 shrink-0 items-center justify-center rounded-lg',
                      isActive ? 'bg-info text-primary' : 'bg-muted text-muted-foreground',
                    )}
                  >
                    <Ticket className="size-4" />
                  </span>
                  <div className="min-w-0 flex-1">
                    <div className="flex items-center gap-1">
                      <code
                        className={cn(
                          'min-w-0 text-sm font-bold break-all',
                          !isActive && 'text-muted-foreground line-through',
                        )}
                      >
                        {inviteCode.code}
                      </code>
                      <Button
                        variant="ghost"
                        size="icon"
                        className="size-7 shrink-0 text-muted-foreground"
                        title="复制"
                        aria-label="复制邀请码"
                        disabled={!isActive}
                        onClick={() => void handleCopyInviteCode(inviteCode.code)}
                      >
                        <Copy className="size-3.5" />
                      </Button>
                    </div>
                    <p className="mt-0.5 text-xs leading-relaxed text-muted-foreground">
                      创建者 {inviteCode.created_by_name ?? '系统'} · 首位使用者{' '}
                      {inviteCode.used_by_name ?? '—'} · 创建于 <span>{createdAtLabel}</span> ·
                      有效期至 <span>{expiresAtLabel}</span>
                    </p>
                  </div>
                </div>
                <div className="flex items-center gap-4 pl-12 md:pl-0">
                  <div className="w-20">
                    <p className="text-xs text-muted-foreground tabular-nums">
                      已使用 {inviteCode.use_count}/{inviteCode.max_uses}
                    </p>
                    <span
                      aria-hidden="true"
                      className="mt-1 block h-1.5 overflow-hidden rounded-full bg-muted"
                    >
                      <span
                        className="block h-full rounded-full bg-primary"
                        style={{ width: `${usageRatio * 100}%` }}
                      />
                    </span>
                  </div>
                  <StatusPill
                    tone={isActive ? 'success' : 'neutral'}
                    className="w-20 justify-center"
                  >
                    {INVITE_STATUS_LABELS[inviteCode.status]}
                  </StatusPill>
                  <div className="ml-auto w-9 md:ml-0">
                    {inviteCode.revoked_at === null && (
                      <Button
                        variant="ghost"
                        size="icon"
                        className="size-9 text-muted-foreground hover:bg-destructive/10 hover:text-destructive"
                        title="撤销邀请码"
                        aria-label={`撤销邀请码 ${inviteCode.code}`}
                        disabled={revokeCodeMut.isPending}
                        onClick={() => {
                          revokeCodeMut.reset();
                          setInviteCodeToRevoke(inviteCode);
                        }}
                      >
                        <Ban className="size-4" />
                      </Button>
                    )}
                  </div>
                </div>
              </MotionDiv>
            );
          })
        )}
      </MotionPresence>
    </SettingsGroup>
  );
}

/** Retain target identity, pending guard and permanent-revocation feedback. */
function renderAdminInviteRevocation(
  state: Pick<
    AdminInviteViewState,
    'inviteCodeToRevoke' | 'setInviteCodeToRevoke' | 'revokeCodeMut'
  >,
) {
  const { inviteCodeToRevoke, setInviteCodeToRevoke, revokeCodeMut } = state;

  return (
    <ConfirmDialog
      open={inviteCodeToRevoke !== null}
      onOpenChange={(nextOpen) => {
        if (!nextOpen && !revokeCodeMut.isPending) {
          setInviteCodeToRevoke(null);
        }
      }}
      title="撤销邀请码？"
      description={`确认永久撤销邀请码 ${inviteCodeToRevoke?.code ?? ''}？历史记录会保留。`}
      actionLabel="确认撤销"
      pendingLabel="撤销中…"
      isPending={revokeCodeMut.isPending}
      error={revokeCodeMut.error instanceof Error ? revokeCodeMut.error.message : null}
      onConfirm={() => {
        if (inviteCodeToRevoke) {
          revokeCodeMut.mutate(inviteCodeToRevoke.id);
        }
      }}
    />
  );
}
