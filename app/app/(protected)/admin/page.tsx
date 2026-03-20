'use client';

import { useState } from 'react';
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import Link from 'next/link';
import { useRouter } from 'next/navigation';
import {
  ArrowLeft,
  Copy,
  Database,
  Key,
  Plus,
  RefreshCw,
  Shield,
  ShieldOff,
  Ticket,
  Trash2,
  Users,
} from 'lucide-react';

import { useAuth } from '@/lib/auth-context';
import {
  adminGetStats,
  adminGetUsers,
  adminSetAdmin,
  adminResetPassword,
  adminDeleteUser,
  adminGetInviteCodes,
  adminCreateInviteCode,
  adminDeleteInviteCode,
} from '@/lib/api';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import { Badge } from '@/components/ui/badge';
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from '@/components/ui/card';
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog';
import { AnnouncementsCard } from '@/components/admin/announcements-card';
import { ScheduledTasksCard } from '@/components/admin/scheduled-tasks-card';

function formatDate(ts: number): string {
  return new Date(ts * 1000).toLocaleDateString('zh-CN', {
    year: 'numeric',
    month: '2-digit',
    day: '2-digit',
    hour: '2-digit',
    minute: '2-digit',
  });
}

function StatCard({
  label,
  value,
  icon,
}: {
  label: string;
  value: string | number;
  icon?: React.ReactNode;
}) {
  return (
    <div className="rounded-lg border bg-card p-4">
      <div className="flex items-center gap-2 text-sm text-muted-foreground">
        {icon}
        {label}
      </div>
      <div className="mt-1 text-2xl font-bold">{value}</div>
    </div>
  );
}

export default function AdminPage() {
  const { user, token } = useAuth();
  const router = useRouter();
  const queryClient = useQueryClient();

  const [resetPwUserId, setResetPwUserId] = useState<number | null>(null);
  const [resetPwValue, setResetPwValue] = useState('');
  const [resetDialogOpen, setResetDialogOpen] = useState(false);
  const [deleteUserId, setDeleteUserId] = useState<number | null>(null);
  const [deleteDialogOpen, setDeleteDialogOpen] = useState(false);

  const { data: stats, isLoading: statsLoading } = useQuery({
    queryKey: ['admin-stats'],
    queryFn: () => adminGetStats(token!),
    enabled: !!token && !!user?.is_admin,
  });

  const { data: users = [] } = useQuery({
    queryKey: ['admin-users'],
    queryFn: () => adminGetUsers(token!),
    enabled: !!token && !!user?.is_admin,
  });

  const { data: inviteCodes = [] } = useQuery({
    queryKey: ['admin-invite-codes'],
    queryFn: () => adminGetInviteCodes(token!),
    enabled: !!token && !!user?.is_admin,
  });

  const toggleAdminMut = useMutation({
    mutationFn: ({ userId, isAdmin }: { userId: number; isAdmin: boolean }) =>
      adminSetAdmin(token!, userId, isAdmin),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['admin-users'] });
      queryClient.invalidateQueries({ queryKey: ['admin-stats'] });
    },
  });

  const resetPwMut = useMutation({
    mutationFn: ({ userId, pw }: { userId: number; pw: string }) =>
      adminResetPassword(token!, userId, pw),
    onSuccess: () => {
      setResetDialogOpen(false);
      setResetPwValue('');
      setResetPwUserId(null);
    },
  });

  const deleteUserMut = useMutation({
    mutationFn: (userId: number) => adminDeleteUser(token!, userId),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['admin-users'] });
      queryClient.invalidateQueries({ queryKey: ['admin-stats'] });
      setDeleteDialogOpen(false);
      setDeleteUserId(null);
    },
  });

  const createCodeMut = useMutation({
    mutationFn: () => adminCreateInviteCode(token!),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['admin-invite-codes'] });
      queryClient.invalidateQueries({ queryKey: ['admin-stats'] });
    },
  });

  const deleteCodeMut = useMutation({
    mutationFn: (codeId: number) => adminDeleteInviteCode(token!, codeId),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['admin-invite-codes'] });
      queryClient.invalidateQueries({ queryKey: ['admin-stats'] });
    },
  });

  if (!user?.is_admin) {
    return (
      <div className="flex flex-col items-center justify-center min-h-[60vh] gap-4">
        <p className="text-muted-foreground">无管理员权限</p>
        <Button variant="outline" onClick={() => router.push('/')}>
          返回首页
        </Button>
      </div>
    );
  }

  const authStats = stats?.auth;
  const indexStats = stats?.index;
  const pushStats = stats?.push;

  return (
    <div className="max-w-5xl mx-auto p-6 space-y-6">
      <div className="flex items-center gap-3">
        <Button variant="ghost" size="icon" asChild>
          <Link href="/">
            <ArrowLeft className="h-5 w-5" />
          </Link>
        </Button>
        <h1 className="text-2xl font-bold flex items-center gap-2">
          <Shield className="h-6 w-6" />
          管理面板
        </h1>
      </div>

      {/* ── Stats Overview ─────────────────────────── */}
      <Card>
        <CardHeader>
          <CardTitle>系统概览</CardTitle>
          <CardDescription>全局统计信息</CardDescription>
        </CardHeader>
        <CardContent>
          {statsLoading ? (
            <div className="text-muted-foreground">加载中...</div>
          ) : (
            <div className="space-y-4">
              <div className="grid grid-cols-2 sm:grid-cols-3 lg:grid-cols-4 gap-3">
                <StatCard
                  label="用户总数"
                  value={authStats?.total_users ?? 0}
                  icon={<Users className="h-4 w-4" />}
                />
                <StatCard
                  label="管理员"
                  value={authStats?.admin_count ?? 0}
                  icon={<Shield className="h-4 w-4" />}
                />
                <StatCard
                  label="收藏夹"
                  value={authStats?.total_folders ?? 0}
                />
                <StatCard
                  label="收藏文章"
                  value={authStats?.total_favorites ?? 0}
                />
                <StatCard
                  label="活跃令牌"
                  value={authStats?.active_tokens ?? 0}
                  icon={<Key className="h-4 w-4" />}
                />
                <StatCard
                  label="推送订阅"
                  value={authStats?.notification_subscribers ?? 0}
                />
                <StatCard
                  label="邀请码 (未使用)"
                  value={authStats?.unused_invite_codes ?? 0}
                  icon={<Ticket className="h-4 w-4" />}
                />
                <StatCard
                  label="邀请码 (已使用)"
                  value={authStats?.used_invite_codes ?? 0}
                />
              </div>

              {/* Index stats */}
              {indexStats && (
                <div className="space-y-2">
                  <h3 className="text-sm font-medium">
                    索引数据库
                    <span className="ml-2 text-muted-foreground font-normal">
                      共 {indexStats.total_articles.toLocaleString()} 篇文章，
                      {indexStats.total_journals.toLocaleString()} 本期刊
                    </span>
                  </h3>
                  <div className="grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-3 gap-2">
                    {indexStats.databases.map((db) => (
                      <div
                        key={db.db_name}
                        className="rounded-md border px-3 py-2 text-sm"
                      >
                        <div className="flex items-center gap-1.5 font-medium">
                          <Database className="h-3.5 w-3.5" />
                          {db.db_name}
                        </div>
                        <div className="text-muted-foreground mt-0.5">
                          {db.articles.toLocaleString()} 文章 · {db.journals} 期刊 · {db.issues} 期
                        </div>
                      </div>
                    ))}
                  </div>
                </div>
              )}

              {/* Push stats */}
              {pushStats && pushStats.length > 0 && (
                <div className="space-y-2">
                  <h3 className="text-sm font-medium">推送状态</h3>
                  <div className="grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-3 gap-2">
                    {pushStats.map((ps) => (
                      <div
                        key={ps.db_name}
                        className="rounded-md border px-3 py-2 text-sm"
                      >
                        <div className="font-medium">{ps.db_name}</div>
                        <div className="text-muted-foreground">
                          状态: {ps.status}
                          {ps.delivered_count != null &&
                            ` · 已推送 ${ps.delivered_count} 篇`}
                          {ps.last_completed && (
                            <span className="block">
                              最近完成: {ps.last_completed}
                            </span>
                          )}
                        </div>
                      </div>
                    ))}
                  </div>
                </div>
              )}
            </div>
          )}
        </CardContent>
      </Card>

      {/* ── User management ────────────────────────── */}
      <Card>
        <CardHeader>
          <CardTitle className="flex items-center gap-2">
            <Users className="h-5 w-5" />
            账号管理
          </CardTitle>
          <CardDescription>查看和管理所有用户账号</CardDescription>
        </CardHeader>
        <CardContent>
          <div className="rounded-md border">
            <table className="w-full text-sm">
              <thead>
                <tr className="border-b bg-muted/50">
                  <th className="px-3 py-2 text-left font-medium">编号</th>
                  <th className="px-3 py-2 text-left font-medium">用户名</th>
                  <th className="px-3 py-2 text-left font-medium">角色</th>
                  <th className="px-3 py-2 text-left font-medium">收藏夹</th>
                  <th className="px-3 py-2 text-left font-medium">收藏</th>
                  <th className="px-3 py-2 text-left font-medium">推送</th>
                  <th className="px-3 py-2 text-left font-medium">注册时间</th>
                  <th className="px-3 py-2 text-left font-medium">操作</th>
                </tr>
              </thead>
              <tbody>
                {users.map((u) => (
                  <tr key={u.id} className="border-b last:border-0">
                    <td className="px-3 py-2">{u.id}</td>
                    <td className="px-3 py-2 font-medium">{u.username}</td>
                    <td className="px-3 py-2">
                      {u.is_admin ? (
                        <Badge>管理员</Badge>
                      ) : (
                        <Badge variant="secondary">用户</Badge>
                      )}
                    </td>
                    <td className="px-3 py-2">{u.folder_count}</td>
                    <td className="px-3 py-2">{u.favorite_count}</td>
                    <td className="px-3 py-2">
                      {u.notify_enabled ? (
                        <Badge variant="secondary" className="text-xs">已订阅</Badge>
                      ) : (
                        <span className="text-muted-foreground">—</span>
                      )}
                    </td>
                    <td className="px-3 py-2 text-muted-foreground">
                      {formatDate(u.created_at)}
                    </td>
                    <td className="px-3 py-2">
                      <div className="flex items-center gap-1">
                        <Button
                          variant="ghost"
                          size="sm"
                          title={u.is_admin ? '取消管理员' : '设为管理员'}
                          disabled={
                            toggleAdminMut.isPending || u.id === user!.id
                          }
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
                          disabled={u.id === user!.id}
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
        <DialogContent>
          <DialogHeader>
            <DialogTitle>重置密码</DialogTitle>
            <DialogDescription>
              为用户 #{resetPwUserId} 设置新密码
            </DialogDescription>
          </DialogHeader>
          <div className="space-y-3 py-2">
            <Input
              type="password"
              value={resetPwValue}
              onChange={(e) => setResetPwValue(e.target.value)}
              placeholder="新密码 (至少6位)"
            />
            <div className="flex gap-2 justify-end">
              <Button
                variant="outline"
                onClick={() => setResetDialogOpen(false)}
              >
                取消
              </Button>
              <Button
                disabled={
                  resetPwValue.length < 6 || resetPwMut.isPending
                }
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
              <p className="text-sm text-destructive">
                {resetPwMut.error instanceof Error
                  ? resetPwMut.error.message
                  : '重置失败'}
              </p>
            )}
          </div>
        </DialogContent>
      </Dialog>

      {/* Delete user dialog */}
      <Dialog open={deleteDialogOpen} onOpenChange={setDeleteDialogOpen}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>确认删除用户</DialogTitle>
            <DialogDescription>
              删除用户 #{deleteUserId} 及其所有数据（收藏夹、收藏、设置）。此操作不可恢复。
            </DialogDescription>
          </DialogHeader>
          <div className="flex gap-2 justify-end py-2">
            <Button
              variant="outline"
              onClick={() => setDeleteDialogOpen(false)}
            >
              取消
            </Button>
            <Button
              variant="destructive"
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

      {/* ── Invite code management ─────────────────── */}
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
            onClick={() => createCodeMut.mutate()}
            disabled={createCodeMut.isPending}
          >
            <Plus className="h-4 w-4 mr-1" />
            生成邀请码
          </Button>
          <div className="rounded-md border">
            <table className="w-full text-sm">
              <thead>
                <tr className="border-b bg-muted/50">
                  <th className="px-3 py-2 text-left font-medium">邀请码</th>
                  <th className="px-3 py-2 text-left font-medium">创建者</th>
                  <th className="px-3 py-2 text-left font-medium">状态</th>
                  <th className="px-3 py-2 text-left font-medium">使用者</th>
                  <th className="px-3 py-2 text-left font-medium">创建时间</th>
                  <th className="px-3 py-2 text-left font-medium">操作</th>
                </tr>
              </thead>
              <tbody>
                {inviteCodes.map((ic) => (
                  <tr key={ic.id} className="border-b last:border-0">
                    <td className="px-3 py-2 font-mono text-xs">
                      <span className="flex items-center gap-1">
                        {ic.code.slice(0, 8)}...
                        <button
                          type="button"
                          onClick={() => navigator.clipboard.writeText(ic.code)}
                          className="p-0.5 rounded hover:bg-muted"
                          title="复制"
                        >
                          <Copy className="h-3 w-3" />
                        </button>
                      </span>
                    </td>
                    <td className="px-3 py-2">
                      {ic.created_by_name ?? (
                        <span className="text-muted-foreground">系统</span>
                      )}
                    </td>
                    <td className="px-3 py-2">
                      {ic.used_by ? (
                        <Badge variant="secondary">已使用</Badge>
                      ) : (
                        <Badge>可用</Badge>
                      )}
                    </td>
                    <td className="px-3 py-2">
                      {ic.used_by_name ?? (
                        <span className="text-muted-foreground">—</span>
                      )}
                    </td>
                    <td className="px-3 py-2 text-muted-foreground">
                      {formatDate(ic.created_at)}
                    </td>
                    <td className="px-3 py-2">
                      {!ic.used_by && (
                        <Button
                          variant="ghost"
                          size="sm"
                          className="text-destructive hover:text-destructive"
                          disabled={deleteCodeMut.isPending}
                          onClick={() => deleteCodeMut.mutate(ic.id)}
                        >
                          <Trash2 className="h-4 w-4" />
                        </Button>
                      )}
                    </td>
                  </tr>
                ))}
                {inviteCodes.length === 0 && (
                  <tr>
                    <td
                      colSpan={6}
                      className="px-3 py-4 text-center text-muted-foreground"
                    >
                      暂无邀请码
                    </td>
                  </tr>
                )}
              </tbody>
            </table>
          </div>
        </CardContent>
      </Card>

      {token && (
        <>
          <ScheduledTasksCard token={token} />
          <AnnouncementsCard token={token} />
        </>
      )}
    </div>
  );
}
