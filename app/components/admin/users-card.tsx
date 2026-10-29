'use client';

import { useMemo, useState } from 'react';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { KeyRound, Shield, ShieldOff, Trash2, Users } from 'lucide-react';

import { adminDeleteUser, adminGetUsers, adminResetPassword, adminSetAdmin } from '@/lib/api';
import { AdminListHeader } from '@/components/admin/admin-list-header';
import { SettingsGroup, StatusPill } from '@/components/settings/settings-section';
import { UserAvatar } from '@/components/shell/user-avatar';
import { Button } from '@/components/ui/button';
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/components/ui/card';
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog';
import { Input } from '@/components/ui/input';

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
 * Render administrator user management and confirmation dialogs.
 *
 * @param props - Current administrator id and query enablement.
 * @returns User management card and dialogs.
 */
export function AdminUsersCard({
  currentUserId,
  isEnabled,
}: {
  currentUserId: number;
  isEnabled: boolean;
}) {
  const queryClient = useQueryClient();
  const [resetPwUserId, setResetPwUserId] = useState<number | null>(null);
  const [resetPwValue, setResetPwValue] = useState('');
  const [resetDialogOpen, setResetDialogOpen] = useState(false);
  const [deleteUserId, setDeleteUserId] = useState<number | null>(null);
  const [deleteDialogOpen, setDeleteDialogOpen] = useState(false);
  const {
    data: users,
    error: usersError,
    isLoading: isUsersLoading,
  } = useQuery({
    queryKey: ['admin-users'],
    queryFn: () => adminGetUsers(),
    enabled: isEnabled,
  });
  const userRows = useMemo(
    () => (users ?? []).map((user) => ({ user, registrationDate: formatDate(user.created_at) })),
    [users],
  );
  const toggleAdminMut = useMutation({
    mutationFn: ({ userId, isAdmin }: { userId: number; isAdmin: boolean }) =>
      adminSetAdmin(userId, isAdmin),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['admin-users'] });
      queryClient.invalidateQueries({ queryKey: ['admin-stats'] });
    },
  });
  const resetPwMut = useMutation({
    mutationFn: ({ userId, pw }: { userId: number; pw: string }) => adminResetPassword(userId, pw),
    onSuccess: () => {
      setResetDialogOpen(false);
      setResetPwValue('');
      setResetPwUserId(null);
    },
  });
  const deleteUserMut = useMutation({
    mutationFn: (userId: number) => adminDeleteUser(userId),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['admin-users'] });
      queryClient.invalidateQueries({ queryKey: ['admin-stats'] });
      setDeleteDialogOpen(false);
      setDeleteUserId(null);
    },
  });

  return (
    <>
      <Card>
        <CardHeader>
          <CardTitle className="flex items-center gap-2">
            <Users className="h-5 w-5" />
            账号管理
          </CardTitle>
          <CardDescription>查看和管理所有用户账号</CardDescription>
        </CardHeader>
        <CardContent className="space-y-4">
          {isUsersLoading && (
            <p role="status" className="text-sm text-muted-foreground">
              加载中…
            </p>
          )}
          {usersError instanceof Error && (
            <p role="alert" className="text-sm text-destructive">
              {usersError.message}
            </p>
          )}
          {toggleAdminMut.isError && (
            <p role="alert" className="text-sm text-destructive">
              {toggleAdminMut.error instanceof Error
                ? toggleAdminMut.error.message
                : '更新管理员状态失败'}
            </p>
          )}
          {users && (
            <AdminListHeader
              summary={`共 ${users.length} 位用户 · ${users.filter((entry) => entry.is_admin).length} 位管理员`}
            />
          )}
          <SettingsGroup>
            {userRows.map(({ user: u, registrationDate }) => (
              <div
                key={u.id}
                className="content-visibility-card flex flex-col gap-3 px-4 py-4 sm:px-5 md:flex-row md:items-center md:gap-6"
              >
                <div className="flex min-w-0 flex-1 items-center gap-3">
                  <UserAvatar
                    username={u.username}
                    className={u.is_admin ? undefined : 'bg-muted text-foreground'}
                  />
                  <div className="min-w-0">
                    <div className="flex flex-wrap items-center gap-2">
                      <span className="truncate text-sm font-bold">{u.username}</span>
                      {u.is_admin ? (
                        <StatusPill tone="info">管理员</StatusPill>
                      ) : (
                        <StatusPill>用户</StatusPill>
                      )}
                      {u.id === currentUserId && (
                        <span className="text-xs text-muted-foreground">（你）</span>
                      )}
                    </div>
                    <div className="mt-0.5 text-xs text-muted-foreground tabular-nums">
                      #{u.id} · <span>注册时间: {registrationDate}</span>
                    </div>
                  </div>
                </div>
                <dl className="grid grid-cols-3 gap-2 text-xs md:flex md:gap-6">
                  <div className="rounded-lg bg-muted/60 px-3 py-2 md:bg-transparent md:p-0 md:text-right">
                    <dt className="text-muted-foreground">收藏夹</dt>
                    <dd className="text-sm font-bold tabular-nums">{u.folder_count}</dd>
                  </div>
                  <div className="rounded-lg bg-muted/60 px-3 py-2 md:bg-transparent md:p-0 md:text-right">
                    <dt className="text-muted-foreground">收藏</dt>
                    <dd className="text-sm font-bold tabular-nums">{u.favorite_count}</dd>
                  </div>
                  <div className="rounded-lg bg-muted/60 px-3 py-2 md:w-14 md:bg-transparent md:p-0 md:text-right">
                    <dt className="text-muted-foreground">推送</dt>
                    <dd
                      className={
                        u.notify_enabled
                          ? 'text-sm font-bold text-primary'
                          : 'text-sm text-muted-foreground'
                      }
                    >
                      {u.notify_enabled ? '已订阅' : '未订阅'}
                    </dd>
                  </div>
                </dl>
                <div className="flex items-center justify-end gap-1 border-t pt-3 md:border-0 md:pt-0">
                  <Button
                    variant="ghost"
                    size="icon"
                    className="size-9"
                    title={u.is_admin ? '取消管理员' : '设为管理员'}
                    aria-label={
                      u.is_admin ? `取消 ${u.username} 的管理员` : `设为 ${u.username} 为管理员`
                    }
                    disabled={toggleAdminMut.isPending || u.id === currentUserId}
                    onClick={() =>
                      toggleAdminMut.mutate({
                        userId: u.id,
                        isAdmin: !u.is_admin,
                      })
                    }
                  >
                    {u.is_admin ? <ShieldOff className="size-4" /> : <Shield className="size-4" />}
                  </Button>
                  <Button
                    variant="ghost"
                    size="icon"
                    className="size-9"
                    title="重置密码"
                    aria-label={`重置 ${u.username} 的密码`}
                    onClick={() => {
                      resetPwMut.reset();
                      setResetPwUserId(u.id);
                      setResetPwValue('');
                      setResetDialogOpen(true);
                    }}
                  >
                    <KeyRound className="size-4" />
                  </Button>
                  <Button
                    variant="ghost"
                    size="icon"
                    title="删除用户"
                    aria-label={`删除用户 ${u.username}`}
                    disabled={u.id === currentUserId}
                    className="size-9 text-muted-foreground hover:bg-destructive/10 hover:text-destructive"
                    onClick={() => {
                      deleteUserMut.reset();
                      setDeleteUserId(u.id);
                      setDeleteDialogOpen(true);
                    }}
                  >
                    <Trash2 className="size-4" />
                  </Button>
                </div>
              </div>
            ))}
          </SettingsGroup>
        </CardContent>
      </Card>

      {/* Password reset dialog */}
      <Dialog open={resetDialogOpen} onOpenChange={setResetDialogOpen}>
        <DialogContent className="max-h-[90vh] overflow-y-auto sm:max-w-lg">
          <DialogHeader>
            <DialogTitle>重置密码</DialogTitle>
            <DialogDescription>为用户 #{resetPwUserId} 设置新密码</DialogDescription>
          </DialogHeader>
          <div className="space-y-3 py-2">
            <Input
              name="admin_new_password"
              type="password"
              aria-label="新密码"
              autoComplete="new-password"
              value={resetPwValue}
              onChange={(e) => setResetPwValue(e.target.value)}
              placeholder="新密码 (至少12位)"
              minLength={12}
            />
            <div className="flex flex-col-reverse gap-2 sm:flex-row sm:justify-end">
              <Button
                variant="outline"
                className="w-full sm:w-auto"
                onClick={() => setResetDialogOpen(false)}
              >
                取消
              </Button>
              <Button
                className="w-full sm:w-auto"
                disabled={resetPwValue.length < 12 || resetPwMut.isPending}
                onClick={() => {
                  if (resetPwUserId != null) {
                    resetPwMut.mutate({
                      userId: resetPwUserId,
                      pw: resetPwValue,
                    });
                  }
                }}
              >
                确认重置
              </Button>
            </div>
            {resetPwMut.isError && (
              <p role="alert" className="text-sm text-destructive">
                {resetPwMut.error instanceof Error ? resetPwMut.error.message : '重置失败'}
              </p>
            )}
          </div>
        </DialogContent>
      </Dialog>

      {/* Delete user dialog */}
      <Dialog open={deleteDialogOpen} onOpenChange={setDeleteDialogOpen}>
        <DialogContent className="max-h-[90vh] overflow-y-auto sm:max-w-lg">
          <DialogHeader>
            <DialogTitle>确认删除用户</DialogTitle>
            <DialogDescription>
              删除用户 #{deleteUserId} 及其所有数据（收藏夹、收藏、设置）。此操作不可恢复。
            </DialogDescription>
          </DialogHeader>
          <div className="flex flex-col-reverse gap-2 py-2 sm:flex-row sm:justify-end">
            <Button
              variant="outline"
              className="w-full sm:w-auto"
              onClick={() => setDeleteDialogOpen(false)}
            >
              取消
            </Button>
            <Button
              variant="destructive"
              className="w-full sm:w-auto"
              disabled={deleteUserMut.isPending}
              onClick={() => {
                if (deleteUserId != null) {
                  deleteUserMut.mutate(deleteUserId);
                }
              }}
            >
              确认删除
            </Button>
          </div>
          {deleteUserMut.isError && (
            <p role="alert" className="text-sm text-destructive">
              {deleteUserMut.error instanceof Error ? deleteUserMut.error.message : '删除失败'}
            </p>
          )}
        </DialogContent>
      </Dialog>
    </>
  );
}
