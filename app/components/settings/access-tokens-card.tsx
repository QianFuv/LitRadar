'use client';

import { useState } from 'react';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { Copy, Key, Plus, Trash2 } from 'lucide-react';

import { createAccessToken, getAccessTokens, revokeAccessToken, type AccessToken } from '@/lib/api';
import { Badge } from '@/components/ui/badge';
import { Button } from '@/components/ui/button';
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/components/ui/card';
import { ConfirmDialog } from '@/components/ui/confirm-dialog';
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
  DialogTrigger,
} from '@/components/ui/dialog';
import { Input } from '@/components/ui/input';
import { Label } from '@/components/ui/label';
import type {
  SettingsCopyFeedback,
  SettingsCopyScope,
} from '@/components/settings/use-settings-copy';

const TTL_OPTIONS = [
  { label: '7天', value: 7 * 86400 },
  { label: '30天', value: 30 * 86400 },
  { label: '90天', value: 90 * 86400 },
  { label: '1年', value: 365 * 86400 },
];
const ACCESS_TOKEN_NAME_MAX_CODE_POINTS = 100;
const ACCESS_TOKEN_NAME_LENGTH_DETAIL = 'Access token name must be at most 100 Unicode code points';

/**
 * Format an access-token expiry timestamp.
 *
 * @param ts - Unix timestamp in seconds.
 * @returns Localized expiry time.
 */
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
 * Render and manage current-user access tokens.
 *
 * @param props - Shared copy feedback and action.
 * @returns Access-token settings card.
 */
export function AccessTokensCard({
  copyFeedback,
  handleCopy,
}: {
  copyFeedback: SettingsCopyFeedback | null;
  handleCopy: (value: string, successMessage: string, scope: SettingsCopyScope) => Promise<void>;
}) {
  const queryClient = useQueryClient();
  const [tokenName, setTokenName] = useState('');
  const [tokenTtl, setTokenTtl] = useState(TTL_OPTIONS[0].value);
  const [newTokenValue, setNewTokenValue] = useState<string | null>(null);
  const [tokenToRevoke, setTokenToRevoke] = useState<AccessToken | null>(null);
  const [dialogOpen, setDialogOpen] = useState(false);
  const tokenNameCodePointCount = Array.from(tokenName).length;
  const tokenNameError =
    tokenNameCodePointCount > ACCESS_TOKEN_NAME_MAX_CODE_POINTS
      ? ACCESS_TOKEN_NAME_LENGTH_DETAIL
      : null;
  const { data: tokens = [] } = useQuery({
    queryKey: ['access-tokens'],
    queryFn: () => getAccessTokens(),
    enabled: true,
  });
  const createTokenMut = useMutation({
    mutationFn: () => createAccessToken(tokenName, tokenTtl),
    onSuccess: (data) => {
      setNewTokenValue(data.token);
      queryClient.invalidateQueries({ queryKey: ['access-tokens'] });
      setTokenName('');
    },
  });
  const creationError = tokenNameError ?? createTokenMut.error?.message ?? null;
  const revokeMut = useMutation({
    mutationFn: (id: number) => revokeAccessToken(id),
    onSuccess: (_data, tokenId) => {
      queryClient.invalidateQueries({ queryKey: ['access-tokens'] });
      setTokenToRevoke((current) => (current?.id === tokenId ? null : current));
    },
  });

  return (
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
                      onClick={() => void handleCopy(newTokenValue, '访问令牌已复制。', 'token')}
                    >
                      <Copy className="h-4 w-4" />
                    </Button>
                  </div>
                  {copyFeedback?.scope === 'token' && (
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
                </div>
              ) : (
                <form
                  onSubmit={(e) => {
                    e.preventDefault();
                    if (tokenNameError) return;
                    createTokenMut.mutate();
                  }}
                  className="space-y-4"
                >
                  <div className="space-y-2">
                    <Label htmlFor="access-token-name">名称</Label>
                    <Input
                      id="access-token-name"
                      name="access_token_name"
                      autoComplete="off"
                      spellCheck={false}
                      value={tokenName}
                      onChange={(e) => setTokenName(e.target.value)}
                      aria-invalid={creationError ? true : undefined}
                      placeholder="例如：接口集成"
                    />
                    <p className="text-xs text-muted-foreground">
                      {tokenNameCodePointCount}/{ACCESS_TOKEN_NAME_MAX_CODE_POINTS} Unicode code
                      points
                    </p>
                  </div>
                  <div className="space-y-2">
                    <div className="text-sm font-medium">有效期</div>
                    <div className="flex gap-2 flex-wrap" role="group" aria-label="访问令牌有效期">
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
                  {creationError && (
                    <p role="alert" className="text-sm text-destructive">
                      {creationError}
                    </p>
                  )}
                  <Button
                    type="submit"
                    disabled={createTokenMut.isPending || tokenNameError !== null}
                  >
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
                  disabled={revokeMut.isPending}
                  onClick={() => {
                    revokeMut.reset();
                    setTokenToRevoke(t);
                  }}
                >
                  <Trash2 className="h-3.5 w-3.5" />
                </Button>
              </div>
            ))}
          </div>
        )}
        <ConfirmDialog
          open={tokenToRevoke !== null}
          onOpenChange={(nextOpen) => {
            if (!nextOpen && !revokeMut.isPending) {
              setTokenToRevoke(null);
            }
          }}
          title="撤销访问令牌？"
          description={`确认撤销访问令牌“${tokenToRevoke?.name || tokenToRevoke?.id || ''}”？撤销后使用该令牌的客户端将立即失去访问权限。`}
          actionLabel="确认撤销"
          pendingLabel="撤销中…"
          isPending={revokeMut.isPending}
          error={revokeMut.error instanceof Error ? revokeMut.error.message : null}
          onConfirm={() => {
            if (tokenToRevoke) {
              revokeMut.mutate(tokenToRevoke.id);
            }
          }}
        />
      </CardContent>
    </Card>
  );
}
