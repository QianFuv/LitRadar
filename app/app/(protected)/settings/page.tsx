'use client';

import { useState } from 'react';
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import Link from 'next/link';
import { ArrowLeft, Copy, Key, Plus, Ticket, Trash2 } from 'lucide-react';

import { useAuth } from '@/lib/auth-context';
import {
  changePassword,
  getAccessTokens,
  createAccessToken,
  revokeAccessToken,
  getInviteCode,
  generateInviteCode,
} from '@/lib/api';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import { Label } from '@/components/ui/label';
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

  const { data: tokens = [] } = useQuery({
    queryKey: ['access-tokens'],
    queryFn: () => getAccessTokens(token!),
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

  if (!user) {
    return (
      <div className="flex flex-col items-center justify-center min-h-[60vh] gap-4">
        <p className="text-muted-foreground">请先登录</p>
        <Button asChild>
          <Link href="/login?next=/settings">登录</Link>
        </Button>
      </div>
    );
  }

  return (
    <div className="max-w-3xl mx-auto p-6 space-y-6">
      <div className="flex items-center gap-3">
        <Button variant="ghost" size="icon" asChild>
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
              <Label>原密码</Label>
              <Input
                type="password"
                value={oldPwd}
                onChange={(e) => setOldPwd(e.target.value)}
                required
              />
            </div>
            <div className="space-y-2">
              <Label>新密码</Label>
              <Input
                type="password"
                value={newPwd}
                onChange={(e) => setNewPwd(e.target.value)}
                placeholder="至少6位"
                required
              />
            </div>
            {pwdMsg && (
              <p className="text-sm text-muted-foreground">{pwdMsg}</p>
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
              <CardDescription>
                每个用户可以生成一个邀请码，供他人注册使用
              </CardDescription>
            </div>
          </div>
        </CardHeader>
        <CardContent>
          {inviteCodeData ? (
            <div className="space-y-3">
              <div className="flex items-center gap-2">
                <code className="flex-1 text-sm bg-muted p-2 rounded break-all">
                  {inviteCodeData.code}
                </code>
                <Button
                  variant="outline"
                  size="icon"
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
            <p className="text-sm text-destructive mt-2">
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
          <div className="flex items-center justify-between">
            <div>
              <CardTitle>访问令牌</CardTitle>
              <CardDescription>
                创建访问令牌，用于接口访问或第三方集成
              </CardDescription>
            </div>
            <Dialog open={dialogOpen} onOpenChange={(open) => {
              setDialogOpen(open);
              if (!open) setNewTokenValue(null);
            }}>
              <DialogTrigger asChild>
                <Button variant="outline" size="sm">
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
                    <div className="flex items-center gap-2">
                      <code className="flex-1 text-xs bg-muted p-2 rounded break-all">
                        {newTokenValue}
                      </code>
                      <Button
                        variant="outline"
                        size="icon"
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
                      <Label>名称</Label>
                      <Input
                        value={tokenName}
                        onChange={(e) => setTokenName(e.target.value)}
                        placeholder="例如：接口集成"
                      />
                    </div>
                    <div className="space-y-2">
                      <Label>有效期</Label>
                      <div className="flex gap-2 flex-wrap">
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
                  className="flex items-center justify-between rounded-md border px-3 py-2"
                >
                  <div className="flex items-center gap-2">
                    <Key className="h-4 w-4 text-muted-foreground" />
                    <span className="text-sm">{t.name || '（未命名）'}</span>
                    <Badge variant="outline" className="text-[10px]">
                      {formatExpiry(t.expires_at)} 过期
                    </Badge>
                  </div>
                  <Button
                    variant="ghost"
                    size="icon"
                    className="h-7 w-7 text-destructive"
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
    </div>
  );
}
