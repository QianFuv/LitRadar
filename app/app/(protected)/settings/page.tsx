'use client';

import { useState } from 'react';
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import Link from 'next/link';
import {
  ArrowLeft,
  CheckCircle2,
  Copy,
  Key,
  Loader2,
  Plus,
  QrCode,
  RefreshCw,
  ShieldCheck,
  Ticket,
  Trash2,
  Unlink,
} from 'lucide-react';

import { useAuth } from '@/lib/auth-context';
import {
  changePassword,
  clearCnkiSession,
  createAccessToken,
  generateInviteCode,
  getAccessTokens,
  getCnkiSession,
  getInviteCode,
  pollCnkiLogin,
  revokeAccessToken,
  startCnkiLogin,
  type CnkiLoginStartResponse,
  type CnkiSessionStatus,
} from '@/lib/api';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import { Label } from '@/components/ui/label';
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/components/ui/card';
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
  DialogTrigger,
} from '@/components/ui/dialog';
import { Badge } from '@/components/ui/badge';

const TTL_OPTIONS = [
  { label: '7天', value: 7 * 86400 },
  { label: '30天', value: 30 * 86400 },
  { label: '90天', value: 90 * 86400 },
  { label: '1年', value: 365 * 86400 },
];

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

export default function SettingsPage() {
  const { user, token, logout } = useAuth();
  const queryClient = useQueryClient();

  // Password form
  const [oldPwd, setOldPwd] = useState('');
  const [newPwd, setNewPwd] = useState('');
  const [pwdMsg, setPwdMsg] = useState<string | null>(null);

  // Token form
  const [tokenName, setTokenName] = useState('');
  const [tokenTtl, setTokenTtl] = useState(TTL_OPTIONS[0].value);
  const [newTokenValue, setNewTokenValue] = useState<string | null>(null);
  const [dialogOpen, setDialogOpen] = useState(false);
  const [cnkiLogin, setCnkiLogin] = useState<CnkiLoginStartResponse | null>(null);
  const [cnkiMessage, setCnkiMessage] = useState<string | null>(null);
  const cnkiSessionQueryKey = ['cnki-session', user?.id] as const;
  const currentCnkiSessionQueryKey = ['cnki-session', 'current'] as const;

  const { data: tokens = [] } = useQuery({
    queryKey: ['access-tokens'],
    queryFn: () => getAccessTokens(token!),
    enabled: !!token,
  });

  const {
    data: cnkiSession,
    isLoading: isCnkiSessionLoading,
    isError: isCnkiSessionError,
    error: cnkiSessionError,
    refetch: refetchCnkiSession,
  } = useQuery({
    queryKey: cnkiSessionQueryKey,
    queryFn: () => getCnkiSession(token!),
    enabled: !!token,
  });

  const { data: inviteCodeData, refetch: refetchInviteCode } = useQuery({
    queryKey: ['invite-code'],
    queryFn: () => getInviteCode(token!),
    enabled: !!token,
  });

  const generateInviteMut = useMutation({
    mutationFn: () => generateInviteCode(token!),
    onSuccess: () => refetchInviteCode(),
  });

  const changePwdMut = useMutation({
    mutationFn: () => changePassword(token!, oldPwd, newPwd),
    onSuccess: () => {
      setPwdMsg('密码修改成功，请重新登录');
      setTimeout(() => {
        void logout();
      }, 1500);
    },
    onError: (err) => {
      setPwdMsg(err instanceof Error ? err.message : '修改失败');
    },
  });

  const createTokenMut = useMutation({
    mutationFn: () => createAccessToken(token!, tokenName.trim(), tokenTtl),
    onSuccess: (data) => {
      setNewTokenValue(data.token);
      queryClient.invalidateQueries({ queryKey: ['access-tokens'] });
      setTokenName('');
    },
  });

  const revokeMut = useMutation({
    mutationFn: (id: number) => revokeAccessToken(token!, id),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ['access-tokens'] }),
  });

  const startCnkiLoginMut = useMutation({
    mutationFn: () => startCnkiLogin(token!),
    onSuccess: (data) => {
      setCnkiLogin(data);
      setCnkiMessage(null);
      queryClient.setQueryData(cnkiSessionQueryKey, data.session);
      queryClient.setQueryData(currentCnkiSessionQueryKey, data.session);
      queryClient.removeQueries({ queryKey: ['article-access'] });
      queryClient.invalidateQueries({ queryKey: ['article-access'] });
    },
    onError: (err) => {
      setCnkiMessage(err instanceof Error ? err.message : '启动知网登录失败');
    },
  });

  const pollCnkiLoginMut = useMutation({
    mutationFn: () => pollCnkiLogin(token!, 15, 1.5),
    onSuccess: (data) => {
      setCnkiLogin(null);
      setCnkiMessage(data.session.status === 'active' ? '登录已完成' : data.status);
      queryClient.setQueryData(cnkiSessionQueryKey, data.session);
      queryClient.setQueryData(currentCnkiSessionQueryKey, data.session);
      queryClient.invalidateQueries({ queryKey: ['cnki-session'] });
      queryClient.removeQueries({ queryKey: ['article-access'] });
      queryClient.invalidateQueries({ queryKey: ['article-access'] });
    },
    onError: (err) => {
      setCnkiMessage(err instanceof Error ? err.message : '确认知网登录失败');
    },
  });

  const clearCnkiSessionMut = useMutation({
    mutationFn: () => clearCnkiSession(token!),
    onSuccess: (data) => {
      setCnkiLogin(null);
      setCnkiMessage('登录状态已清除');
      queryClient.setQueryData(cnkiSessionQueryKey, data);
      queryClient.setQueryData(currentCnkiSessionQueryKey, data);
      queryClient.invalidateQueries({ queryKey: ['cnki-session'] });
      queryClient.removeQueries({ queryKey: ['article-access'] });
      queryClient.invalidateQueries({ queryKey: ['article-access'] });
    },
    onError: (err) => {
      setCnkiMessage(err instanceof Error ? err.message : '清除知网登录失败');
    },
  });

  if (!user) {
    return (
      <main
        id="main-content"
        className="flex flex-col items-center justify-center min-h-[60vh] gap-4"
      >
        <p className="text-muted-foreground">请先登录</p>
        <Button asChild>
          <Link href="/login?next=/settings">登录</Link>
        </Button>
      </main>
    );
  }

  return (
    <main id="main-content" className="mx-auto max-w-3xl space-y-4 p-4 sm:space-y-6 sm:p-6">
      <div className="flex items-center gap-2 sm:gap-3">
        <Button variant="ghost" size="icon" aria-label="返回首页" asChild>
          <Link href="/">
            <ArrowLeft className="h-5 w-5" />
          </Link>
        </Button>
        <h1 className="text-2xl font-bold">账号设置</h1>
      </div>

      {/* Account info */}
      <Card>
        <CardHeader>
          <CardTitle>账号信息</CardTitle>
        </CardHeader>
        <CardContent>
          <div className="text-sm">
            用户名: <span className="font-medium">{user.username}</span>
          </div>
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <div className="flex flex-col gap-3 sm:flex-row sm:items-start sm:justify-between">
            <div>
              <CardTitle className="flex items-center gap-2">
                <ShieldCheck className="h-5 w-5" />
                浙江图书馆 CNKI
              </CardTitle>
              <CardDescription>用于中文数据库文章全文获取</CardDescription>
            </div>
            <Badge variant={getCnkiStatusVariant(cnkiSession)}>
              {isCnkiSessionLoading ? '检查中' : getCnkiStatusLabel(cnkiSession)}
            </Badge>
          </div>
        </CardHeader>
        <CardContent className="space-y-4">
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
            <p role="status" className="text-sm text-muted-foreground">
              {cnkiMessage}
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
                      完成登录
                    </Button>
                    <Button
                      variant="ghost"
                      size="sm"
                      aria-label="复制 CNKI 登录二维码内容"
                      onClick={() => navigator.clipboard.writeText(cnkiLogin.qr_code)}
                    >
                      <Copy className="h-4 w-4" />
                      复制
                    </Button>
                  </div>
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
                onClick={() => clearCnkiSessionMut.mutate()}
                disabled={clearCnkiSessionMut.isPending}
              >
                <Unlink className="h-4 w-4" />
                清除
              </Button>
            )}
          </div>
        </CardContent>
      </Card>

      {/* Change password */}
      <Card>
        <CardHeader>
          <CardTitle>修改密码</CardTitle>
        </CardHeader>
        <CardContent>
          <form
            onSubmit={(e) => {
              e.preventDefault();
              setPwdMsg(null);
              changePwdMut.mutate();
            }}
            className="space-y-4 max-w-sm"
          >
            <div className="space-y-2">
              <Label htmlFor="old-password">原密码</Label>
              <Input
                id="old-password"
                type="password"
                autoComplete="current-password"
                value={oldPwd}
                onChange={(e) => setOldPwd(e.target.value)}
                aria-invalid={changePwdMut.isError}
                required
              />
            </div>
            <div className="space-y-2">
              <Label htmlFor="new-password">新密码</Label>
              <Input
                id="new-password"
                type="password"
                autoComplete="new-password"
                value={newPwd}
                onChange={(e) => setNewPwd(e.target.value)}
                placeholder="至少6位"
                aria-invalid={changePwdMut.isError}
                required
              />
            </div>
            {pwdMsg && (
              <p
                role={changePwdMut.isError ? 'alert' : 'status'}
                className="text-sm text-muted-foreground"
              >
                {pwdMsg}
              </p>
            )}
            <Button type="submit" disabled={changePwdMut.isPending}>
              修改密码
            </Button>
          </form>
        </CardContent>
      </Card>

      {/* Invite code */}
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
                  onClick={() => navigator.clipboard.writeText(inviteCodeData.code)}
                >
                  <Copy className="h-4 w-4" />
                </Button>
              </div>
              <p className="text-xs text-muted-foreground">
                {inviteCodeData.used ? '此邀请码已被使用' : '此邀请码尚未使用'}
              </p>
            </div>
          ) : (
            <Button
              onClick={() => generateInviteMut.mutate()}
              disabled={generateInviteMut.isPending}
            >
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

      {/* Access tokens */}
      <Card>
        <CardHeader>
          <div className="flex flex-col gap-3 sm:flex-row sm:items-start sm:justify-between">
            <div>
              <CardTitle>访问令牌</CardTitle>
              <CardDescription>创建访问令牌，用于接口访问或第三方集成</CardDescription>
            </div>
            <Dialog
              open={dialogOpen}
              onOpenChange={(open: boolean) => {
                setDialogOpen(open);
                if (!open) setNewTokenValue(null);
              }}
            >
              <DialogTrigger asChild>
                <Button variant="outline" size="sm" className="w-full sm:w-auto">
                  <Plus className="h-4 w-4 mr-1" />
                  新建
                </Button>
              </DialogTrigger>
              <DialogContent>
                <DialogHeader>
                  <DialogTitle>创建访问令牌</DialogTitle>
                  <DialogDescription>令牌仅显示一次，请妥善保管</DialogDescription>
                </DialogHeader>
                {newTokenValue ? (
                  <div className="space-y-3">
                    <p className="text-sm text-muted-foreground">新令牌已创建：</p>
                    <div className="flex flex-col gap-2 sm:flex-row sm:items-center">
                      <code className="flex-1 rounded bg-muted p-2 text-xs break-all">
                        {newTokenValue}
                      </code>
                      <Button
                        variant="outline"
                        size="icon"
                        className="self-start sm:self-auto"
                        aria-label="复制新访问令牌"
                        onClick={() => navigator.clipboard.writeText(newTokenValue)}
                      >
                        <Copy className="h-4 w-4" />
                      </Button>
                    </div>
                  </div>
                ) : (
                  <form
                    onSubmit={(e) => {
                      e.preventDefault();
                      createTokenMut.mutate();
                    }}
                    className="space-y-4"
                  >
                    <div className="space-y-2">
                      <Label htmlFor="access-token-name">名称</Label>
                      <Input
                        id="access-token-name"
                        value={tokenName}
                        onChange={(e) => setTokenName(e.target.value)}
                        placeholder="例如：接口集成"
                      />
                    </div>
                    <div className="space-y-2">
                      <div className="text-sm font-medium">有效期</div>
                      <div
                        className="flex gap-2 flex-wrap"
                        role="group"
                        aria-label="访问令牌有效期"
                      >
                        {TTL_OPTIONS.map((opt) => (
                          <Button
                            type="button"
                            key={opt.value}
                            variant={tokenTtl === opt.value ? 'default' : 'outline'}
                            size="sm"
                            onClick={() => setTokenTtl(opt.value)}
                          >
                            {opt.label}
                          </Button>
                        ))}
                      </div>
                    </div>
                    <Button type="submit" disabled={createTokenMut.isPending}>
                      创建
                    </Button>
                  </form>
                )}
              </DialogContent>
            </Dialog>
          </div>
        </CardHeader>
        <CardContent>
          {tokens.length === 0 ? (
            <p className="text-sm text-muted-foreground">暂无访问令牌</p>
          ) : (
            <div className="space-y-2">
              {tokens.map((t) => (
                <div
                  key={t.id}
                  className="flex flex-col gap-3 rounded-md border px-3 py-2 sm:flex-row sm:items-center sm:justify-between"
                >
                  <div className="flex min-w-0 flex-wrap items-center gap-2">
                    <Key className="h-4 w-4 text-muted-foreground" />
                    <span className="break-all text-sm">{t.name || '（未命名）'}</span>
                    <Badge variant="outline" className="text-[10px]">
                      到 {formatExpiry(t.expires_at)} 过期
                    </Badge>
                  </div>
                  <Button
                    variant="ghost"
                    size="icon"
                    className="h-7 w-7 self-end text-destructive sm:self-auto"
                    aria-label={`撤销访问令牌 ${t.name || t.id}`}
                    onClick={() => revokeMut.mutate(t.id)}
                  >
                    <Trash2 className="h-3.5 w-3.5" />
                  </Button>
                </div>
              ))}
            </div>
          )}
        </CardContent>
      </Card>
    </main>
  );
}
