'use client';

import { useState } from 'react';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { RefreshCw, Shield, ShieldOff, Trash2, Users } from 'lucide-react';

import { adminDeleteUser, adminGetUsers, adminResetPassword, adminSetAdmin } from '@/lib/api';
import { Badge } from '@/components/ui/badge';
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
  const { data: users = [] } = useQuery({
    queryKey: ['admin-users'],
    queryFn: () => adminGetUsers(),
    enabled: isEnabled,
  });
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
          <div className="space-y-3 md:hidden">
            {users.map((u) => (
              <div key={u.id} className="content-visibility-card rounded-lg border p-4">
                <div className="space-y-3">
                  <div className="flex items-start justify-between gap-3">
                    <div className="min-w-0 space-y-1">
                      <div className="flex flex-wrap items-center gap-2">
                        <span className="font-medium">{u.username}</span>
                        {u.is_admin ? (
                          <Badge>管理员</Badge>
                        ) : (
                          <Badge variant="secondary">用户</Badge>
                        )}
                      </div>
                      <div className="text-xs text-muted-foreground">用户 #{u.id}</div>
                    </div>
                    {u.notify_enabled ? (
                      <Badge variant="secondary" className="text-xs">
                        已订阅
                      </Badge>
                    ) : (
                      <span className="text-xs text-muted-foreground">未订阅</span>
                    )}
                  </div>
                  <div className="grid grid-cols-2 gap-3 text-sm">
                    <div className="rounded-md bg-muted/40 px-3 py-2">
                      <div className="text-xs text-muted-foreground">收藏夹</div>
                      <div className="mt-1 font-medium">{u.folder_count}</div>
                    </div>
                    <div className="rounded-md bg-muted/40 px-3 py-2">
                      <div className="text-xs text-muted-foreground">收藏</div>
                      <div className="mt-1 font-medium">{u.favorite_count}</div>
                    </div>
                  </div>
                  <div className="text-xs text-muted-foreground">
                    注册时间: {formatDate(u.created_at)}
                  </div>
                  <div className="grid grid-cols-2 gap-2">
                    <Button
                      variant="outline"
                      size="sm"
                      disabled={toggleAdminMut.isPending || u.id === currentUserId}
                      onClick={() =>
                        toggleAdminMut.mutate({
                          userId: u.id,
                          isAdmin: !u.is_admin,
                        })
                      }
                    >
                      {u.is_admin ? (
                        <ShieldOff className="h-4 w-4" />
                      ) : (
                        <Shield className="h-4 w-4" />
                      )}
                      {u.is_admin ? '取消管理员' : '设为管理员'}
                    </Button>
                    <Button
                      variant="outline"
                      size="sm"
                      onClick={() => {
                        setResetPwUserId(u.id);
                        setResetPwValue('');
                        setResetDialogOpen(true);
                      }}
                    >
                      <RefreshCw className="h-4 w-4" />
                      重置密码
                    </Button>
                    <Button
                      variant="destructive"
                      size="sm"
                      className="col-span-2"
                      disabled={u.id === currentUserId}
                      onClick={() => {
                        setDeleteUserId(u.id);
                        setDeleteDialogOpen(true);
                      }}
                    >
                      <Trash2 className="h-4 w-4" />
                      删除用户
                    </Button>
                  </div>
                </div>
              </div>
            ))}
          </div>
          <div className="hidden overflow-x-auto rounded-md border md:block">
            <table className="min-w-[52rem] w-full text-sm">
              <thead>
                <tr className="border-b bg-muted/50">
                  <th scope="col" className="px-3 py-2 text-left font-medium">
                    编号
                  </th>
                  <th scope="col" className="px-3 py-2 text-left font-medium">
                    用户名
                  </th>
                  <th scope="col" className="px-3 py-2 text-left font-medium">
                    角色
                  </th>
                  <th scope="col" className="px-3 py-2 text-left font-medium">
                    收藏夹
                  </th>
                  <th scope="col" className="px-3 py-2 text-left font-medium">
                    收藏
                  </th>
                  <th scope="col" className="px-3 py-2 text-left font-medium">
                    推送
                  </th>
                  <th scope="col" className="px-3 py-2 text-left font-medium">
                    注册时间
                  </th>
                  <th scope="col" className="px-3 py-2 text-left font-medium">
                    操作
                  </th>
                </tr>
              </thead>
              <tbody>
                {users.map((u) => (
                  <tr key={u.id} className="content-visibility-table-row border-b last:border-0">
                    <td className="px-3 py-2">{u.id}</td>
                    <td className="px-3 py-2 font-medium">{u.username}</td>
                    <td className="px-3 py-2">
                      {u.is_admin ? <Badge>管理员</Badge> : <Badge variant="secondary">用户</Badge>}
                    </td>
                    <td className="px-3 py-2">{u.folder_count}</td>
                    <td className="px-3 py-2">{u.favorite_count}</td>
                    <td className="px-3 py-2">
                      {u.notify_enabled ? (
                        <Badge variant="secondary" className="text-xs">
                          已订阅
                        </Badge>
                      ) : (
                        <span className="text-muted-foreground">—</span>
                      )}
                    </td>
                    <td className="px-3 py-2 text-muted-foreground">{formatDate(u.created_at)}</td>
                    <td className="px-3 py-2">
                      <div className="flex items-center gap-1">
                        <Button
                          variant="ghost"
                          size="sm"
                          title={u.is_admin ? '取消管理员' : '设为管理员'}
                          aria-label={
                            u.is_admin
                              ? `取消 ${u.username} 的管理员`
                              : `设为 ${u.username} 为管理员`
                          }
                          disabled={toggleAdminMut.isPending || u.id === currentUserId}
                          onClick={() =>
                            toggleAdminMut.mutate({
                              userId: u.id,
                              isAdmin: !u.is_admin,
                            })
                          }
                        >
                          {u.is_admin ? (
                            <ShieldOff className="h-4 w-4" />
                          ) : (
                            <Shield className="h-4 w-4" />
                          )}
                        </Button>
                        <Button
                          variant="ghost"
                          size="sm"
                          title="重置密码"
                          aria-label={`重置 ${u.username} 的密码`}
                          onClick={() => {
                            setResetPwUserId(u.id);
                            setResetPwValue('');
                            setResetDialogOpen(true);
                          }}
                        >
                          <RefreshCw className="h-4 w-4" />
                        </Button>
                        <Button
                          variant="ghost"
                          size="sm"
                          title="删除用户"
                          aria-label={`删除用户 ${u.username}`}
                          disabled={u.id === currentUserId}
                          className="text-destructive hover:text-destructive"
                          onClick={() => {
                            setDeleteUserId(u.id);
                            setDeleteDialogOpen(true);
                          }}
                        >
                          <Trash2 className="h-4 w-4" />
                        </Button>
                      </div>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
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
        </DialogContent>
      </Dialog>
    </>
  );
}
