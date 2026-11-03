'use client';

import { useState } from 'react';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { Check, Key, KeyRound, Plus, Trash2 } from 'lucide-react';

import { createAccessToken, getAccessTokens, revokeAccessToken, type AccessToken } from '@/lib/api';
import {
  SettingsGroup,
  SettingsSection,
  SettingsSectionContent,
  SettingsSectionDescription,
  SettingsSectionHeader,
  SettingsSectionTitle,
  StatusPill,
} from '@/components/settings/settings-section';
import { Button } from '@/components/ui/button';
import { CopyButton } from '@/components/ui/copy-button';
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
import {
  DISCLOSURE_VARIANTS,
  MotionDiv,
  MotionForm,
  MotionParagraph,
  MotionPresence,
  PANEL_VARIANTS,
  REVEAL_VARIANTS,
} from '@/components/ui/motion';
import type {
  SettingsCopyFeedback,
  SettingsCopyScope,
} from '@/components/settings/use-settings-copy';
import { RevealItem } from '@/components/ui/reveal';
import { Skeleton } from '@/components/ui/skeleton';
import { ToggleGroup, ToggleGroupItem } from '@/components/ui/toggle-group';

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
export function AccessTokensCard(props: {
  copyFeedback: SettingsCopyFeedback | null;
  handleCopy: (value: string, successMessage: string, scope: SettingsCopyScope) => Promise<void>;
}) {
  const state = useAccessTokenViewState(props);
  const { newTokenValue, setNewTokenValue, dialogOpen, setDialogOpen } = state;
  return (
    <SettingsSection>
      <SettingsSectionHeader>
        <div className="flex w-full items-end justify-between gap-4">
          <div className="min-w-0">
            <SettingsSectionTitle>访问令牌</SettingsSectionTitle>
            <SettingsSectionDescription>
              创建访问令牌，用于接口访问或第三方集成
            </SettingsSectionDescription>
          </div>
          <Dialog
            open={dialogOpen}
            onOpenChange={(open: boolean) => {
              setDialogOpen(open);
              if (!open) setNewTokenValue(null);
            }}
          >
            <DialogTrigger asChild>
              <Button size="sm" className="shrink-0">
                <Plus className="size-4" />
                新建
              </Button>
            </DialogTrigger>
            <DialogContent>
              <DialogHeader>
                <DialogTitle>创建访问令牌</DialogTitle>
                <DialogDescription>令牌仅显示一次，请妥善保管</DialogDescription>
              </DialogHeader>
              <MotionPresence mode="wait">
                {newTokenValue
                  ? renderCreatedAccessToken(state, newTokenValue)
                  : renderAccessTokenForm(state)}
              </MotionPresence>
            </DialogContent>
          </Dialog>
        </div>
      </SettingsSectionHeader>
      <SettingsSectionContent>
        <RevealItem
          isReady={!state.isTokensLoading}
          placeholder={<Skeleton className="h-16 w-full rounded-xl" />}
        >
          <SettingsGroup>{renderAccessTokenList(state)}</SettingsGroup>
        </RevealItem>
        {renderAccessTokenRevocation(state)}
      </SettingsSectionContent>
    </SettingsSection>
  );
}
/** Own the original settings hooks and exact mutation/cache completion order. */
function useAccessTokenViewState({
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
  const { data: tokens = [], isPending: isTokensLoading } = useQuery({
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

  return {
    copyFeedback,
    handleCopy,
    isTokensLoading,
    tokenName,
    setTokenName,
    tokenTtl,
    setTokenTtl,
    newTokenValue,
    setNewTokenValue,
    tokenToRevoke,
    setTokenToRevoke,
    dialogOpen,
    setDialogOpen,
    tokenNameCodePointCount,
    tokenNameError,
    tokens,
    createTokenMut,
    creationError,
    revokeMut,
  };
}
type AccessTokenViewState = ReturnType<typeof useAccessTokenViewState>;

/** Retain the one-time plaintext token and scoped copy feedback. */
function renderCreatedAccessToken(
  state: Pick<AccessTokenViewState, 'copyFeedback' | 'handleCopy'>,
  newTokenValue: string,
) {
  const { copyFeedback, handleCopy } = state;

  return (
    <MotionDiv
      key="token-created"
      data-motion-token-dialog-state="created"
      className="space-y-3"
      initial="hidden"
      animate="visible"
      exit="exit"
      variants={PANEL_VARIANTS}
    >
      <p className="flex items-center gap-2 text-sm font-bold text-success-foreground">
        <span className="flex size-5 items-center justify-center rounded-full bg-success">
          <Check className="size-3" strokeWidth={3} aria-hidden="true" />
        </span>
        新令牌已创建：
      </p>
      <div className="flex items-stretch gap-2">
        <code className="min-w-0 flex-1 rounded-lg border border-dashed border-primary/50 bg-info px-3 py-2.5 text-xs leading-5 break-all text-info-foreground">
          {newTokenValue}
        </code>
        <CopyButton
          variant="outline"
          className="h-auto w-11 shrink-0"
          aria-label="复制新访问令牌"
          isCopied={copyFeedback?.scope === 'token' && copyFeedback.tone === 'success'}
          onCopy={() => void handleCopy(newTokenValue, '访问令牌已复制。', 'token')}
        />
      </div>
      <MotionPresence>
        {copyFeedback?.scope === 'token' && (
          <MotionParagraph
            key="token-copy-feedback"
            data-motion-feedback="token-copy"
            role={copyFeedback.tone === 'error' ? 'alert' : 'status'}
            className={copyFeedback.tone === 'error' ? 'text-sm text-destructive' : 'sr-only'}
            initial="hidden"
            animate="visible"
            exit="exit"
            variants={REVEAL_VARIANTS}
          >
            {copyFeedback.message}
          </MotionParagraph>
        )}
      </MotionPresence>
    </MotionDiv>
  );
}

/** Retain the raw name, Unicode limit, TTL selection and submit admission guard. */
function renderAccessTokenForm(state: AccessTokenViewState) {
  const {
    tokenName,
    setTokenName,
    tokenTtl,
    setTokenTtl,
    tokenNameCodePointCount,
    tokenNameError,
    createTokenMut,
    creationError,
  } = state;

  return (
    <MotionForm
      key="token-form"
      data-motion-token-dialog-state="form"
      onSubmit={(e) => {
        e.preventDefault();
        if (tokenNameError) return;
        createTokenMut.mutate();
      }}
      className="space-y-4"
      initial="hidden"
      animate="visible"
      exit="exit"
      variants={PANEL_VARIANTS}
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
          {tokenNameCodePointCount}/{ACCESS_TOKEN_NAME_MAX_CODE_POINTS} Unicode code points
        </p>
      </div>
      <div className="space-y-2">
        <div className="text-sm font-medium">有效期</div>
        <ToggleGroup
          type="single"
          aria-label="访问令牌有效期"
          className="grid w-full grid-cols-4 gap-1 border-0 bg-muted p-1"
          value={String(tokenTtl)}
          onValueChange={(value) => {
            if (value) setTokenTtl(Number(value));
          }}
        >
          {TTL_OPTIONS.map((opt) => (
            <ToggleGroupItem
              key={opt.value}
              value={String(opt.value)}
              className="h-9 justify-center text-sm md:h-9 data-[state=on]:bg-background data-[state=on]:font-bold data-[state=on]:text-foreground data-[state=on]:shadow-paper data-[state=on]:hover:text-foreground"
            >
              {opt.label}
            </ToggleGroupItem>
          ))}
        </ToggleGroup>
      </div>
      <MotionPresence>
        {creationError && (
          <MotionParagraph
            key="token-creation-error"
            data-motion-feedback="token-creation"
            role="alert"
            className="text-sm text-destructive"
            initial="hidden"
            animate="visible"
            exit="exit"
            variants={REVEAL_VARIANTS}
          >
            {creationError}
          </MotionParagraph>
        )}
      </MotionPresence>
      <Button
        type="submit"
        className="w-full"
        disabled={createTokenMut.isPending || tokenNameError !== null}
      >
        创建
      </Button>
    </MotionForm>
  );
}

/** Retain empty/list branches and directly keyed revocation rows. */
function renderAccessTokenList(
  state: Pick<AccessTokenViewState, 'setTokenToRevoke' | 'tokens' | 'revokeMut'>,
) {
  const { setTokenToRevoke, tokens, revokeMut } = state;

  const nowSeconds = Date.now() / 1000;
  return (
    <MotionPresence>
      {tokens.length === 0 ? (
        <MotionDiv
          key="empty-tokens"
          className="flex flex-col items-center gap-3 px-6 py-10 text-center"
          initial="hidden"
          animate="visible"
          exit="exit"
          variants={REVEAL_VARIANTS}
        >
          <span className="flex size-12 items-center justify-center rounded-full bg-muted text-muted-foreground">
            <KeyRound className="size-5" aria-hidden="true" />
          </span>
          <div>
            <p className="text-sm font-bold">暂无访问令牌</p>
            <p className="mt-1 text-xs text-muted-foreground">
              令牌可用于脚本、自动化或第三方工具调用 LitRadar 接口
            </p>
          </div>
        </MotionDiv>
      ) : (
        tokens.map((t) => {
          const remainingDays = Math.max(0, Math.ceil((t.expires_at - nowSeconds) / 86400));
          return (
            <MotionDiv
              key={t.id}
              data-motion-token-key={t.id}
              className="flex items-center gap-3 overflow-hidden px-4 py-3.5 sm:px-5"
              initial="hidden"
              animate="visible"
              exit="exit"
              variants={DISCLOSURE_VARIANTS}
            >
              <span className="flex size-9 shrink-0 items-center justify-center rounded-lg bg-muted text-muted-foreground">
                <Key className="size-4" aria-hidden="true" />
              </span>
              <div className="min-w-0 flex-1">
                <p className="truncate text-sm font-bold">{t.name || '（未命名）'}</p>
                <p className="text-xs text-muted-foreground tabular-nums">
                  到 {formatExpiry(t.expires_at)} 过期
                </p>
              </div>
              <StatusPill
                tone={remainingDays === 0 ? 'danger' : remainingDays <= 7 ? 'warning' : 'neutral'}
                className="hidden sm:inline-flex"
              >
                {remainingDays === 0 ? '已过期' : `剩余 ${remainingDays} 天`}
              </StatusPill>
              <Button
                variant="ghost"
                size="icon"
                className="size-9 shrink-0 text-muted-foreground hover:bg-destructive/10 hover:text-destructive"
                aria-label={`撤销访问令牌 ${t.name || t.id}`}
                disabled={revokeMut.isPending}
                onClick={() => {
                  revokeMut.reset();
                  setTokenToRevoke(t);
                }}
              >
                <Trash2 className="size-4" />
              </Button>
            </MotionDiv>
          );
        })
      )}
    </MotionPresence>
  );
}

/** Retain confirmation target identity and pending-dismissal safeguards. */
function renderAccessTokenRevocation(
  state: Pick<AccessTokenViewState, 'tokenToRevoke' | 'setTokenToRevoke' | 'revokeMut'>,
) {
  const { tokenToRevoke, setTokenToRevoke, revokeMut } = state;

  return (
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
  );
}
