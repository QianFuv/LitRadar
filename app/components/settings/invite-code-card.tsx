'use client';

import { useMutation, useQuery } from '@tanstack/react-query';
import { Copy, Ticket } from 'lucide-react';

import { generateInviteCode, getInviteCode } from '@/lib/api';
import { Button } from '@/components/ui/button';
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/components/ui/card';
import type {
  SettingsCopyFeedback,
  SettingsCopyScope,
} from '@/components/settings/use-settings-copy';

/**
 * Render and manage the current user's single invite code.
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
  const { data: inviteCodeData, refetch: refetchInviteCode } = useQuery({
    queryKey: ['invite-code'],
    queryFn: () => getInviteCode(),
    enabled: true,
  });
  const generateInviteMut = useMutation({
    mutationFn: () => generateInviteCode(),
    onSuccess: () => refetchInviteCode(),
  });

  return (
    <Card>
      <CardHeader>
        <div className="flex items-center justify-between">
          <div>
            <CardTitle className="flex items-center gap-2">
              <Ticket className="h-5 w-5" />
              邀请码
            </CardTitle>
            <CardDescription>每个用户可以生成一个邀请码，供他人注册使用</CardDescription>
          </div>
        </div>
      </CardHeader>
      <CardContent>
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
            <p className="text-xs text-muted-foreground">
              {inviteCodeData.used ? '此邀请码已被使用' : '此邀请码尚未使用'}
            </p>
          </div>
        ) : (
          <Button onClick={() => generateInviteMut.mutate()} disabled={generateInviteMut.isPending}>
            生成邀请码
          </Button>
        )}
        {generateInviteMut.isError && (
          <p role="alert" className="text-sm text-destructive mt-2">
            {generateInviteMut.error instanceof Error
              ? generateInviteMut.error.message
              : '生成失败'}
          </p>
        )}
      </CardContent>
    </Card>
  );
}
