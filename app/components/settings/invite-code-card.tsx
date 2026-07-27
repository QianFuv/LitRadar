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
  SettingsSection,
  SettingsSectionContent,
  SettingsSectionDescription,
  SettingsSectionHeader,
  SettingsSectionTitle,
} from '@/components/settings/settings-section';
import type {
  SettingsCopyFeedback,
  SettingsCopyScope,
} from '@/components/settings/use-settings-copy';
import { Badge } from '@/components/ui/badge';
import { Button } from '@/components/ui/button';
import { ConfirmDialog } from '@/components/ui/confirm-dialog';

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
export function InviteCodeCard({
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

  return (
    <SettingsSection>
      <SettingsSectionHeader>
        <div className="flex items-center justify-between">
          <div>
            <SettingsSectionTitle className="flex items-center gap-2">
              <Ticket className="h-5 w-5" />
              邀请码
            </SettingsSectionTitle>
            <SettingsSectionDescription>
              邀请码默认有效 7 天且可注册 1 次，可随时轮换或永久撤销
            </SettingsSectionDescription>
          </div>
        </div>
      </SettingsSectionHeader>
      <SettingsSectionContent>
        {inviteCodeData ? (
          <div className="space-y-3">
            <div className="flex flex-col gap-2 sm:flex-row sm:items-center">
              <code className="flex-1 rounded bg-muted p-2 text-xs break-all sm:text-sm">
                {inviteCodeData.code}
              </code>
              <Button
                variant="outline"
                size="icon"
                className="self-start sm:self-auto"
                aria-label="复制邀请码"
                disabled={inviteCodeData.status !== 'active'}
                onClick={() => void handleCopy(inviteCodeData.code, '邀请码已复制。', 'invite')}
              >
                <Copy className="h-4 w-4" />
              </Button>
            </div>
            {copyFeedback?.scope === 'invite' && (
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
            <div className="flex flex-wrap items-center gap-2 text-xs text-muted-foreground">
              <Badge variant={inviteCodeData.status === 'active' ? 'default' : 'secondary'}>
                {INVITE_STATUS_LABELS[inviteCodeData.status]}
              </Badge>
              <span>
                已使用 {inviteCodeData.use_count}/{inviteCodeData.max_uses} 次
              </span>
              <span>有效期至 {formatInviteDate(inviteCodeData.expires_at)}</span>
            </div>
            <div className="flex flex-wrap gap-2">
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
                <RotateCcw className="h-4 w-4" />
                轮换邀请码
              </Button>
              {inviteCodeData.revoked_at === null && (
                <Button
                  variant="destructive"
                  size="sm"
                  disabled={isMutating}
                  onClick={() => {
                    generateInviteMut.reset();
                    rotateInviteMut.reset();
                    revokeInviteMut.reset();
                    setIsRevokeOpen(true);
                  }}
                >
                  <Ban className="h-4 w-4" />
                  撤销邀请码
                </Button>
              )}
            </div>
          </div>
        ) : (
          <Button
            onClick={() => {
              rotateInviteMut.reset();
              revokeInviteMut.reset();
              generateInviteMut.mutate();
            }}
            disabled={isMutating}
          >
            生成邀请码
          </Button>
        )}
        {mutationError && (
          <p role="alert" className="mt-2 text-sm text-destructive">
            {mutationError instanceof Error ? mutationError.message : '邀请码操作失败'}
          </p>
        )}
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
      </SettingsSectionContent>
    </SettingsSection>
  );
}
