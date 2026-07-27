'use client';

import { useState } from 'react';
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
import { Badge } from '@/components/ui/badge';
import { Button } from '@/components/ui/button';
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/components/ui/card';
import { ConfirmDialog } from '@/components/ui/confirm-dialog';
import { Input } from '@/components/ui/input';

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
export function AdminInviteCodesCard({ isEnabled }: { isEnabled: boolean }) {
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
  const {
    data: inviteCodes = [],
    error: inviteCodesError,
    isLoading,
  } = useQuery({
    queryKey: ['admin-invite-codes'],
    queryFn: () => adminGetInviteCodes(),
    enabled: isEnabled,
  });
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
        <div className="grid gap-3 rounded-lg border p-3 sm:grid-cols-[minmax(0,1fr)_minmax(0,1fr)_auto] sm:items-end">
          <label className="space-y-1 text-sm" htmlFor="invite-valid-days">
            <span className="font-medium">有效天数</span>
            <Input
              id="invite-valid-days"
              type="number"
              min={1}
              max={365}
              value={validDays}
              onChange={(event) => setValidDays(event.target.value)}
            />
          </label>
          <label className="space-y-1 text-sm" htmlFor="invite-max-uses">
            <span className="font-medium">最多使用次数</span>
            <Input
              id="invite-max-uses"
              type="number"
              min={1}
              max={1000}
              value={maxUses}
              onChange={(event) => setMaxUses(event.target.value)}
            />
          </label>
          <Button
            variant="outline"
            size="sm"
            className="w-full sm:w-auto"
            onClick={() => createCodeMut.mutate()}
            disabled={createCodeMut.isPending || !isPolicyValid}
          >
            <Plus className="h-4 w-4" />
            生成邀请码
          </Button>
        </div>
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
        {createCodeMut.error instanceof Error && (
          <p role="alert" className="text-sm text-destructive">
            {createCodeMut.error.message}
          </p>
        )}
        {copyFeedback && (
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
        <div className="space-y-3 md:hidden">
          {inviteCodes.length === 0 ? (
            <div className="rounded-lg border p-4 text-sm text-muted-foreground">暂无邀请码</div>
          ) : (
            inviteCodes.map((inviteCode) => (
              <div key={inviteCode.id} className="content-visibility-card rounded-lg border p-4">
                <div className="space-y-3">
                  <div className="flex items-start justify-between gap-3">
                    <div className="min-w-0 space-y-1">
                      <div className="text-xs text-muted-foreground">邀请码</div>
                      <code className="block break-all rounded bg-muted px-2 py-1 text-xs">
                        {inviteCode.code}
                      </code>
                    </div>
                    <Button
                      variant="outline"
                      size="sm"
                      className="shrink-0"
                      disabled={inviteCode.status !== 'active'}
                      onClick={() => void handleCopyInviteCode(inviteCode.code)}
                    >
                      <Copy className="h-4 w-4" />
                      复制
                    </Button>
                  </div>
                  <div className="flex flex-wrap items-center gap-2">
                    <Badge variant={inviteCode.status === 'active' ? 'default' : 'secondary'}>
                      {INVITE_STATUS_LABELS[inviteCode.status]}
                    </Badge>
                    <span className="text-xs text-muted-foreground">
                      已使用 {inviteCode.use_count}/{inviteCode.max_uses} 次
                    </span>
                  </div>
                  <div className="grid grid-cols-1 gap-2 text-sm">
                    <div className="rounded-md bg-muted/40 px-3 py-2">
                      <div className="text-xs text-muted-foreground">创建者</div>
                      <div className="mt-1 break-all">{inviteCode.created_by_name ?? '系统'}</div>
                    </div>
                    <div className="rounded-md bg-muted/40 px-3 py-2">
                      <div className="text-xs text-muted-foreground">首位使用者</div>
                      <div className="mt-1 break-all">{inviteCode.used_by_name ?? '—'}</div>
                    </div>
                    <div className="rounded-md bg-muted/40 px-3 py-2">
                      <div className="text-xs text-muted-foreground">有效期</div>
                      <div className="mt-1">{formatDate(inviteCode.expires_at)}</div>
                    </div>
                  </div>
                  {inviteCode.revoked_at === null && (
                    <Button
                      variant="destructive"
                      size="sm"
                      className="w-full"
                      disabled={revokeCodeMut.isPending}
                      onClick={() => {
                        revokeCodeMut.reset();
                        setInviteCodeToRevoke(inviteCode);
                      }}
                    >
                      <Ban className="h-4 w-4" />
                      撤销邀请码
                    </Button>
                  )}
                </div>
              </div>
            ))
          )}
        </div>
        <div className="hidden overflow-x-auto rounded-md border md:block">
          <table className="min-w-[64rem] w-full text-sm">
            <thead>
              <tr className="border-b bg-muted/50">
                <th scope="col" className="px-3 py-2 text-left font-medium">
                  邀请码
                </th>
                <th scope="col" className="px-3 py-2 text-left font-medium">
                  创建者
                </th>
                <th scope="col" className="px-3 py-2 text-left font-medium">
                  状态
                </th>
                <th scope="col" className="px-3 py-2 text-left font-medium">
                  用量
                </th>
                <th scope="col" className="px-3 py-2 text-left font-medium">
                  首位使用者
                </th>
                <th scope="col" className="px-3 py-2 text-left font-medium">
                  有效期
                </th>
                <th scope="col" className="px-3 py-2 text-left font-medium">
                  创建时间
                </th>
                <th scope="col" className="px-3 py-2 text-left font-medium">
                  操作
                </th>
              </tr>
            </thead>
            <tbody>
              {inviteCodes.map((inviteCode) => (
                <tr
                  key={inviteCode.id}
                  className="content-visibility-table-row border-b last:border-0"
                >
                  <td className="px-3 py-2 font-mono text-xs">
                    <span className="flex items-center gap-1">
                      {inviteCode.code.slice(0, 8)}…
                      <button
                        type="button"
                        onClick={() => void handleCopyInviteCode(inviteCode.code)}
                        className="p-0.5 rounded hover:bg-muted"
                        title="复制"
                        aria-label="复制邀请码"
                        disabled={inviteCode.status !== 'active'}
                      >
                        <Copy className="h-3 w-3" />
                      </button>
                    </span>
                  </td>
                  <td className="px-3 py-2">
                    {inviteCode.created_by_name ?? (
                      <span className="text-muted-foreground">系统</span>
                    )}
                  </td>
                  <td className="px-3 py-2">
                    <Badge variant={inviteCode.status === 'active' ? 'default' : 'secondary'}>
                      {INVITE_STATUS_LABELS[inviteCode.status]}
                    </Badge>
                  </td>
                  <td className="px-3 py-2">
                    {inviteCode.use_count}/{inviteCode.max_uses}
                  </td>
                  <td className="px-3 py-2">
                    {inviteCode.used_by_name ?? <span className="text-muted-foreground">—</span>}
                  </td>
                  <td className="px-3 py-2 text-muted-foreground">
                    {formatDate(inviteCode.expires_at)}
                  </td>
                  <td className="px-3 py-2 text-muted-foreground">
                    {formatDate(inviteCode.created_at)}
                  </td>
                  <td className="px-3 py-2">
                    {inviteCode.revoked_at === null && (
                      <Button
                        variant="ghost"
                        size="sm"
                        className="text-destructive hover:text-destructive"
                        aria-label={`撤销邀请码 ${inviteCode.code}`}
                        disabled={revokeCodeMut.isPending}
                        onClick={() => {
                          revokeCodeMut.reset();
                          setInviteCodeToRevoke(inviteCode);
                        }}
                      >
                        <Ban className="h-4 w-4" />
                      </Button>
                    )}
                  </td>
                </tr>
              ))}
              {inviteCodes.length === 0 && (
                <tr>
                  <td colSpan={8} className="px-3 py-4 text-center text-muted-foreground">
                    暂无邀请码
                  </td>
                </tr>
              )}
            </tbody>
          </table>
        </div>
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
      </CardContent>
    </Card>
  );
}
