'use client';

import { useState } from 'react';
import { useMutation, useQuery } from '@tanstack/react-query';
import { Ban, Copy, RotateCcw, Ticket } from 'lucide-react';

import {
  generateInviteCode,
  getInviteCode,
  revokeInviteCode,
  rotateInviteCode,
  type InviteCodeStatus,
} from '@/lib/api';
import {
  SettingsGroup,
  SettingsRow,
  SettingsSection,
  SettingsSectionContent,
  SettingsSectionDescription,
  SettingsSectionHeader,
  SettingsSectionTitle,
  StatusPill,
} from '@/components/settings/settings-section';
import type {
  SettingsCopyFeedback,
  SettingsCopyScope,
} from '@/components/settings/use-settings-copy';
import { Button } from '@/components/ui/button';
import { ConfirmDialog } from '@/components/ui/confirm-dialog';
import { cn } from '@/lib/utils';

const INVITE_STATUS_LABELS: Record<InviteCodeStatus, string> = {
  active: '可用',
  expired: '已过期',
  revoked: '已撤销',
  exhausted: '已用尽',
};

/**
 * Format a Unix timestamp for invite lifecycle details.
 *
 * @param timestamp - Unix timestamp in seconds.
 * @returns Localized date and time.
 */
function formatInviteDate(timestamp: number): string {
  return new Date(timestamp * 1000).toLocaleString('zh-CN');
}

/**
 * Render and manage the current user's invite-code lifecycle.
 *
 * @param props - Shared copy feedback and action.
 * @returns Invite-code settings card.
 */
export function InviteCodeCard(props: {
  copyFeedback: SettingsCopyFeedback | null;
  handleCopy: (value: string, successMessage: string, scope: SettingsCopyScope) => Promise<void>;
}) {
  const state = useInviteViewState(props);
  const {
    inviteCodeData,
    generateInviteMut,
    rotateInviteMut,
    revokeInviteMut,
    mutationError,
    isMutating,
  } = state;
  return (
    <SettingsSection>
      <SettingsSectionHeader>
        <SettingsSectionTitle>邀请码</SettingsSectionTitle>
        <SettingsSectionDescription>
          邀请码默认有效 7 天且可注册 1 次，可随时轮换或永久撤销
        </SettingsSectionDescription>
      </SettingsSectionHeader>
      <SettingsSectionContent>
        {inviteCodeData ? (
          renderCurrentInviteCode(state, inviteCodeData)
        ) : (
          <SettingsGroup>
            <SettingsRow
              leading={
                <span className="flex size-10 shrink-0 items-center justify-center rounded-full bg-info text-primary">
                  <Ticket className="size-5" aria-hidden="true" />
                </span>
              }
              title="还没有邀请码"
              description="生成后即可分享给同事注册 LitRadar"
            >
              <Button
                size="sm"
                onClick={() => {
                  rotateInviteMut.reset();
                  revokeInviteMut.reset();
                  generateInviteMut.mutate();
                }}
                disabled={isMutating}
              >
                生成邀请码
              </Button>
            </SettingsRow>
          </SettingsGroup>
        )}
        {mutationError && (
          <p role="alert" className="mt-2 px-1 text-sm text-destructive">
            {mutationError instanceof Error ? mutationError.message : '邀请码操作失败'}
          </p>
        )}
        {renderInviteRevokeConfirmation(state)}
      </SettingsSectionContent>
    </SettingsSection>
  );
}
/** Own the original settings hooks and exact mutation/cache completion order. */
function useInviteViewState({
  copyFeedback,
  handleCopy,
}: {
  copyFeedback: SettingsCopyFeedback | null;
  handleCopy: (value: string, successMessage: string, scope: SettingsCopyScope) => Promise<void>;
}) {
  const [isRevokeOpen, setIsRevokeOpen] = useState(false);
  const { data: inviteCodeData, refetch: refetchInviteCode } = useQuery({
    queryKey: ['invite-code'],
    queryFn: () => getInviteCode(),
    enabled: true,
  });
  const generateInviteMut = useMutation({
    mutationFn: () => generateInviteCode(),
    onSuccess: () => void refetchInviteCode(),
  });
  const rotateInviteMut = useMutation({
    mutationFn: () => rotateInviteCode(),
    onSuccess: () => void refetchInviteCode(),
  });
  const revokeInviteMut = useMutation({
    mutationFn: () => revokeInviteCode(),
    onSuccess: () => {
      setIsRevokeOpen(false);
      void refetchInviteCode();
    },
  });
  const mutationError = generateInviteMut.error ?? rotateInviteMut.error ?? revokeInviteMut.error;
  const isMutating =
    generateInviteMut.isPending || rotateInviteMut.isPending || revokeInviteMut.isPending;

  return {
    copyFeedback,
    handleCopy,
    isRevokeOpen,
    setIsRevokeOpen,
    inviteCodeData,
    generateInviteMut,
    rotateInviteMut,
    revokeInviteMut,
    mutationError,
    isMutating,
  };
}
type InviteViewState = ReturnType<typeof useInviteViewState>;

/** Retain invite-only copy feedback and accessible tone. */
function renderInviteCopyFeedback(state: Pick<InviteViewState, 'copyFeedback'>) {
  const { copyFeedback } = state;

  return (
    copyFeedback?.scope === 'invite' && (
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
    )
  );
}

/** Retain status-dependent copying, lifecycle metadata and reset-before-action order. */
function renderCurrentInviteCode(
  state: InviteViewState,
  inviteCodeData: NonNullable<InviteViewState['inviteCodeData']>,
) {
  const {
    handleCopy,
    setIsRevokeOpen,
    generateInviteMut,
    rotateInviteMut,
    revokeInviteMut,
    isMutating,
  } = state;

  const usageRatio =
    inviteCodeData.max_uses > 0
      ? Math.min(1, inviteCodeData.use_count / inviteCodeData.max_uses)
      : 0;
  const isActive = inviteCodeData.status === 'active';

  return (
    <SettingsGroup>
      <div className="flex items-stretch gap-3 px-4 py-4 sm:px-5">
        <div
          className={cn(
            'relative min-w-0 flex-1 rounded-lg border border-dashed px-4 py-3',
            isActive ? 'border-primary/50 bg-info' : 'bg-muted/60',
          )}
        >
          <p className="text-xs text-muted-foreground">当前邀请码</p>
          <code
            className={cn(
              'mt-1 block text-base leading-6 font-bold tracking-wide break-all sm:text-lg',
              isActive ? 'text-info-foreground' : 'text-muted-foreground line-through',
            )}
          >
            {inviteCodeData.code}
          </code>
        </div>
        <Button
          variant="outline"
          className="h-auto w-12 shrink-0"
          aria-label="复制邀请码"
          disabled={!isActive}
          onClick={() => void handleCopy(inviteCodeData.code, '邀请码已复制。', 'invite')}
        >
          <Copy className="size-4" />
        </Button>
      </div>
      {state.copyFeedback?.scope === 'invite' && (
        <div className="px-4 py-2.5 sm:px-5">{renderInviteCopyFeedback(state)}</div>
      )}
      <dl className="grid grid-cols-1 divide-y text-sm sm:grid-cols-3 sm:divide-x sm:divide-y-0">
        <div className="flex items-center justify-between gap-3 px-4 py-3 sm:block sm:px-5">
          <dt className="text-xs text-muted-foreground">状态</dt>
          <dd className="sm:mt-1.5">
            <StatusPill tone={isActive ? 'success' : 'neutral'}>
              {INVITE_STATUS_LABELS[inviteCodeData.status]}
            </StatusPill>
          </dd>
        </div>
        <div className="flex items-center justify-between gap-3 px-4 py-3 sm:block sm:px-5">
          <dt className="text-xs text-muted-foreground">使用情况</dt>
          <dd className="flex items-center gap-2 sm:mt-1.5">
            <span className="tabular-nums">
              已使用 {inviteCodeData.use_count}/{inviteCodeData.max_uses} 次
            </span>
            <span
              aria-hidden="true"
              className="hidden h-1.5 w-16 overflow-hidden rounded-full bg-muted sm:block"
            >
              <span
                className="block h-full rounded-full bg-primary"
                style={{ width: `${usageRatio * 100}%` }}
              />
            </span>
          </dd>
        </div>
        <div className="flex items-center justify-between gap-3 px-4 py-3 sm:block sm:px-5">
          <dt className="text-xs text-muted-foreground">有效期至</dt>
          <dd className="tabular-nums sm:mt-1.5">{formatInviteDate(inviteCodeData.expires_at)}</dd>
        </div>
      </dl>
      <div className="flex flex-wrap justify-end gap-2 bg-muted/40 px-4 py-3 sm:px-5">
        {inviteCodeData.revoked_at === null && (
          <Button
            variant="ghost"
            size="sm"
            className="text-destructive hover:bg-destructive/10 hover:text-destructive"
            disabled={isMutating}
            onClick={() => {
              generateInviteMut.reset();
              rotateInviteMut.reset();
              revokeInviteMut.reset();
              setIsRevokeOpen(true);
            }}
          >
            <Ban className="size-4" />
            撤销邀请码
          </Button>
        )}
        <Button
          variant="outline"
          size="sm"
          disabled={isMutating}
          onClick={() => {
            generateInviteMut.reset();
            revokeInviteMut.reset();
            rotateInviteMut.mutate();
          }}
        >
          <RotateCcw className="size-4" />
          轮换邀请码
        </Button>
      </div>
    </SettingsGroup>
  );
}

/** Retain the explicit revoke confirmation and pending dismissal guard. */
function renderInviteRevokeConfirmation(
  state: Pick<InviteViewState, 'isRevokeOpen' | 'setIsRevokeOpen' | 'revokeInviteMut'>,
) {
  const { isRevokeOpen, setIsRevokeOpen, revokeInviteMut } = state;

  return (
    <ConfirmDialog
      open={isRevokeOpen}
      onOpenChange={(nextOpen) => {
        if (!revokeInviteMut.isPending) {
          setIsRevokeOpen(nextOpen);
        }
      }}
      title="撤销邀请码？"
      description="撤销后该邀请码将永久失效；如需新邀请码，可以随后轮换。"
      actionLabel="确认撤销"
      pendingLabel="撤销中…"
      isPending={revokeInviteMut.isPending}
      error={revokeInviteMut.error instanceof Error ? revokeInviteMut.error.message : null}
      onConfirm={() => revokeInviteMut.mutate()}
    />
  );
}
