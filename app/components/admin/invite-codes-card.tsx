'use client';

import { useState } from 'react';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { Copy, Plus, Ticket, Trash2 } from 'lucide-react';

import {
  adminCreateInviteCode,
  adminDeleteInviteCode,
  adminGetInviteCodes,
  type AdminInviteCode,
} from '@/lib/api';
import { copyTextToClipboard } from '@/lib/clipboard';
import { Badge } from '@/components/ui/badge';
import { Button } from '@/components/ui/button';
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/components/ui/card';
import { ConfirmDialog } from '@/components/ui/confirm-dialog';

/**
 * Format a Unix timestamp for administrator tables.
 *
 * @param ts - Unix timestamp in seconds.
 * @returns Localized date and time.
 */
function formatDate(ts: number): string {
  return new Date(ts * 1000).toLocaleDateString('zh-CN', {
    year: 'numeric',
    month: '2-digit',
    day: '2-digit',
    hour: '2-digit',
    minute: '2-digit',
  });
}

/**
 * Render administrator invite-code creation and management.
 *
 * @param props - Whether administrator queries may run.
 * @returns Invite-code management card.
 */
export function AdminInviteCodesCard({ isEnabled }: { isEnabled: boolean }) {
  const queryClient = useQueryClient();
  const [copyFeedback, setCopyFeedback] = useState<{
    message: string;
    tone: 'error' | 'success';
  } | null>(null);
  const [inviteCodeToDelete, setInviteCodeToDelete] = useState<AdminInviteCode | null>(null);
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
    mutationFn: () => adminCreateInviteCode(),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['admin-invite-codes'] });
      queryClient.invalidateQueries({ queryKey: ['admin-stats'] });
    },
  });
  const deleteCodeMut = useMutation({
    mutationFn: (codeId: number) => adminDeleteInviteCode(codeId),
    onSuccess: (_data, codeId) => {
      queryClient.invalidateQueries({ queryKey: ['admin-invite-codes'] });
      queryClient.invalidateQueries({ queryKey: ['admin-stats'] });
      setInviteCodeToDelete((current) => (current?.id === codeId ? null : current));
    },
  });
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
        <CardDescription>查看和管理所有邀请码</CardDescription>
      </CardHeader>
      <CardContent className="space-y-4">
        <Button
          variant="outline"
          size="sm"
          className="w-full sm:w-auto"
          onClick={() => createCodeMut.mutate()}
          disabled={createCodeMut.isPending}
        >
          <Plus className="h-4 w-4 mr-1" />
          生成邀请码
        </Button>
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
            inviteCodes.map((ic) => (
              <div key={ic.id} className="content-visibility-card rounded-lg border p-4">
                <div className="space-y-3">
                  <div className="flex items-start justify-between gap-3">
                    <div className="min-w-0 space-y-1">
                      <div className="text-xs text-muted-foreground">邀请码</div>
                      <code className="block break-all rounded bg-muted px-2 py-1 text-xs">
                        {ic.code}
                      </code>
                    </div>
                    <Button
                      variant="outline"
                      size="sm"
                      className="shrink-0"
                      onClick={() => void handleCopyInviteCode(ic.code)}
                    >
                      <Copy className="h-4 w-4" />
                      复制
                    </Button>
                  </div>
                  <div className="flex flex-wrap items-center gap-2">
                    {ic.used_by ? <Badge variant="secondary">已使用</Badge> : <Badge>可用</Badge>}
                    <span className="text-xs text-muted-foreground">
                      创建时间: {formatDate(ic.created_at)}
                    </span>
                  </div>
                  <div className="grid grid-cols-1 gap-2 text-sm">
                    <div className="rounded-md bg-muted/40 px-3 py-2">
                      <div className="text-xs text-muted-foreground">创建者</div>
                      <div className="mt-1 break-all">{ic.created_by_name ?? '系统'}</div>
                    </div>
                    <div className="rounded-md bg-muted/40 px-3 py-2">
                      <div className="text-xs text-muted-foreground">使用者</div>
                      <div className="mt-1 break-all">{ic.used_by_name ?? '—'}</div>
                    </div>
                  </div>
                  {!ic.used_by && (
                    <Button
                      variant="destructive"
                      size="sm"
                      className="w-full"
                      disabled={deleteCodeMut.isPending}
                      onClick={() => {
                        deleteCodeMut.reset();
                        setInviteCodeToDelete(ic);
                      }}
                    >
                      <Trash2 className="h-4 w-4" />
                      删除邀请码
                    </Button>
                  )}
                </div>
              </div>
            ))
          )}
        </div>
        <div className="hidden overflow-x-auto rounded-md border md:block">
          <table className="min-w-[48rem] w-full text-sm">
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
                  使用者
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
              {inviteCodes.map((ic) => (
                <tr key={ic.id} className="content-visibility-table-row border-b last:border-0">
                  <td className="px-3 py-2 font-mono text-xs">
                    <span className="flex items-center gap-1">
                      {ic.code.slice(0, 8)}…
                      <button
                        type="button"
                        onClick={() => void handleCopyInviteCode(ic.code)}
                        className="p-0.5 rounded hover:bg-muted"
                        title="复制"
                        aria-label="复制邀请码"
                      >
                        <Copy className="h-3 w-3" />
                      </button>
                    </span>
                  </td>
                  <td className="px-3 py-2">
                    {ic.created_by_name ?? <span className="text-muted-foreground">系统</span>}
                  </td>
                  <td className="px-3 py-2">
                    {ic.used_by ? <Badge variant="secondary">已使用</Badge> : <Badge>可用</Badge>}
                  </td>
                  <td className="px-3 py-2">
                    {ic.used_by_name ?? <span className="text-muted-foreground">—</span>}
                  </td>
                  <td className="px-3 py-2 text-muted-foreground">{formatDate(ic.created_at)}</td>
                  <td className="px-3 py-2">
                    {!ic.used_by && (
                      <Button
                        variant="ghost"
                        size="sm"
                        className="text-destructive hover:text-destructive"
                        aria-label={`删除邀请码 ${ic.code}`}
                        disabled={deleteCodeMut.isPending}
                        onClick={() => {
                          deleteCodeMut.reset();
                          setInviteCodeToDelete(ic);
                        }}
                      >
                        <Trash2 className="h-4 w-4" />
                      </Button>
                    )}
                  </td>
                </tr>
              ))}
              {inviteCodes.length === 0 && (
                <tr>
                  <td colSpan={6} className="px-3 py-4 text-center text-muted-foreground">
                    暂无邀请码
                  </td>
                </tr>
              )}
            </tbody>
          </table>
        </div>
        <ConfirmDialog
          open={inviteCodeToDelete !== null}
          onOpenChange={(nextOpen) => {
            if (!nextOpen && !deleteCodeMut.isPending) {
              setInviteCodeToDelete(null);
            }
          }}
          title="删除邀请码？"
          description={`确认删除邀请码 ${inviteCodeToDelete?.code ?? ''}？`}
          actionLabel="确认删除"
          pendingLabel="删除中…"
          isPending={deleteCodeMut.isPending}
          error={deleteCodeMut.error instanceof Error ? deleteCodeMut.error.message : null}
          onConfirm={() => {
            if (inviteCodeToDelete) {
              deleteCodeMut.mutate(inviteCodeToDelete.id);
            }
          }}
        />
      </CardContent>
    </Card>
  );
}
