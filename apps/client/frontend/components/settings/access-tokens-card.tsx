import { useState } from 'react';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { Check, Key, KeyRound, Plus, Trash2 } from '@/components/ui/animated-icons';

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
import * as stylex from '@stylexjs/stylex';
import { colors, lineHeights, shadows } from '@/lib/theme.stylex';

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
        <div {...stylex.props(styles.accessTokensCardDiv)}>
          <div {...stylex.props(styles.accessTokensCardDiv2)}>
            <SettingsSectionTitle icon={Key}>访问令牌</SettingsSectionTitle>
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
              <Button size="sm" xstyle={styles.accessTokensCardButton}>
                <Plus {...stylex.props(styles.accessTokensCardPlus)} />
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
          placeholder={<Skeleton xstyle={styles.accessTokensCardSkeleton} />}
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
      {...stylex.props(styles.renderCreatedAccessTokenMotionDiv)}
      initial="hidden"
      animate="visible"
      exit="exit"
      variants={PANEL_VARIANTS}
    >
      <p {...stylex.props(styles.renderCreatedAccessTokenP)}>
        <span {...stylex.props(styles.renderCreatedAccessTokenSpan)}>
          <Check
            {...stylex.props(styles.renderCreatedAccessTokenCheck)}
            strokeWidth={3}
            aria-hidden="true"
          />
        </span>
        新令牌已创建：
      </p>
      <div {...stylex.props(styles.renderCreatedAccessTokenDiv)}>
        <code {...stylex.props(styles.renderCreatedAccessTokenCode)}>{newTokenValue}</code>
        <CopyButton
          variant="outline"
          xstyle={styles.renderCreatedAccessTokenCopyButton}
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
            {...stylex.props(
              copyFeedback.tone === 'error'
                ? styles.renderCreatedAccessTokenMotionParagraphError
                : styles.renderCreatedAccessTokenMotionParagraphNotError,
            )}
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
      {...stylex.props(styles.renderAccessTokenFormMotionForm)}
      initial="hidden"
      animate="visible"
      exit="exit"
      variants={PANEL_VARIANTS}
    >
      <div>
        <Label xstyle={styles.renderAccessTokenFormDivItem} htmlFor="access-token-name">
          名称
        </Label>
        <Input
          xstyle={styles.renderAccessTokenFormDivItem}
          id="access-token-name"
          name="access_token_name"
          autoComplete="off"
          spellCheck={false}
          value={tokenName}
          onChange={(e) => setTokenName(e.target.value)}
          aria-invalid={creationError ? true : undefined}
          placeholder="例如：接口集成"
        />
        <p {...stylex.props(styles.renderAccessTokenFormDivItem, styles.renderAccessTokenFormP)}>
          {tokenNameCodePointCount}/{ACCESS_TOKEN_NAME_MAX_CODE_POINTS} Unicode code points
        </p>
      </div>
      <div>
        <div
          {...stylex.props(styles.renderAccessTokenFormDivItem, styles.renderAccessTokenFormDiv)}
        >
          有效期
        </div>
        <ToggleGroup
          type="single"
          aria-label="访问令牌有效期"
          xstyle={[styles.renderAccessTokenFormDivItem, styles.renderAccessTokenFormToggleGroup]}
          value={String(tokenTtl)}
          onValueChange={(value) => {
            if (value) setTokenTtl(Number(value));
          }}
        >
          {TTL_OPTIONS.map((opt) => (
            <ToggleGroupItem
              key={opt.value}
              value={String(opt.value)}
              xstyle={styles.renderAccessTokenFormToggleGroupItem}
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
            {...stylex.props(styles.renderCreatedAccessTokenMotionParagraphError)}
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
        xstyle={styles.renderAccessTokenFormButton}
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
          {...stylex.props(styles.renderAccessTokenListMotionDiv)}
          initial="hidden"
          animate="visible"
          exit="exit"
          variants={REVEAL_VARIANTS}
        >
          <span {...stylex.props(styles.renderAccessTokenListSpan)}>
            <KeyRound {...stylex.props(styles.renderAccessTokenListKeyRound)} aria-hidden="true" />
          </span>
          <div>
            <p {...stylex.props(styles.renderAccessTokenListP)}>暂无访问令牌</p>
            <p {...stylex.props(styles.renderAccessTokenListP2)}>
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
              {...stylex.props(styles.renderAccessTokenListMotionDiv2)}
              initial="hidden"
              animate="visible"
              exit="exit"
              variants={DISCLOSURE_VARIANTS}
            >
              <span {...stylex.props(styles.renderAccessTokenListSpan2)}>
                <Key {...stylex.props(styles.accessTokensCardPlus)} aria-hidden="true" />
              </span>
              <div {...stylex.props(styles.renderAccessTokenListDiv)}>
                <p {...stylex.props(styles.renderAccessTokenListP3)}>{t.name || '（未命名）'}</p>
                <p {...stylex.props(styles.renderAccessTokenListP4)}>
                  到 {formatExpiry(t.expires_at)} 过期
                </p>
              </div>
              <StatusPill
                tone={remainingDays === 0 ? 'danger' : remainingDays <= 7 ? 'warning' : 'neutral'}
                xstyle={styles.renderAccessTokenListStatusPill}
              >
                {remainingDays === 0 ? '已过期' : `剩余 ${remainingDays} 天`}
              </StatusPill>
              <Button
                variant="ghost"
                size="icon"
                xstyle={styles.renderAccessTokenListButton}
                aria-label={`撤销访问令牌 ${t.name || t.id}`}
                disabled={revokeMut.isPending}
                onClick={() => {
                  revokeMut.reset();
                  setTokenToRevoke(t);
                }}
              >
                <Trash2 {...stylex.props(styles.accessTokensCardPlus)} />
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

const styles = stylex.create({
  accessTokensCardDiv: {
    display: 'flex',
    width: '100%',
    alignItems: 'flex-end',
    justifyContent: 'space-between',
    gap: '1rem',
  },
  accessTokensCardDiv2: {
    minWidth: '0px',
  },
  accessTokensCardButton: {
    flexShrink: 0,
  },
  accessTokensCardPlus: {
    width: '1rem',
    height: '1rem',
  },
  accessTokensCardSkeleton: {
    height: '4rem',
    width: '100%',
    borderRadius: '12px',
  },
  renderCreatedAccessTokenMotionDiv: {
    display: 'flex',
    flexDirection: 'column',
    rowGap: '0.75rem',
  },
  renderCreatedAccessTokenP: {
    display: 'flex',
    alignItems: 'center',
    gap: '0.5rem',
    fontSize: '0.875rem',
    lineHeight: lineHeights.textSm,
    fontWeight: 700,
    color: colors.successForeground,
  },
  renderCreatedAccessTokenSpan: {
    display: 'flex',
    width: '1.25rem',
    height: '1.25rem',
    alignItems: 'center',
    justifyContent: 'center',
    borderRadius: 'calc(infinity * 1px)',
    backgroundColor: colors.success,
  },
  renderCreatedAccessTokenCheck: {
    width: '0.75rem',
    height: '0.75rem',
  },
  renderCreatedAccessTokenDiv: {
    display: 'flex',
    alignItems: 'stretch',
    gap: '0.5rem',
  },
  renderCreatedAccessTokenCode: {
    minWidth: '0px',
    flex: '1',
    borderRadius: '8px',
    borderStyle: 'dashed',
    borderWidth: '1px',
    borderColor: `color-mix(in oklab, ${colors.primary} 50%, transparent)`,
    backgroundColor: colors.info,
    paddingInline: '0.75rem',
    paddingBlock: '0.625rem',
    fontSize: '0.75rem',
    lineHeight: '1.25rem',
    wordBreak: 'break-all',
    color: colors.infoForeground,
  },
  renderCreatedAccessTokenCopyButton: {
    height: 'auto',
    width: '2.75rem',
    flexShrink: 0,
  },
  renderCreatedAccessTokenMotionParagraphError: {
    fontSize: '0.875rem',
    lineHeight: lineHeights.textSm,
    color: colors.destructive,
  },
  renderCreatedAccessTokenMotionParagraphNotError: {
    position: 'absolute',
    width: '1px',
    height: '1px',
    padding: 0,
    margin: '-1px',
    overflow: 'hidden',
    clipPath: 'inset(50%)',
    whiteSpace: 'nowrap',
    borderWidth: 0,
  },
  renderAccessTokenFormMotionForm: {
    display: 'flex',
    flexDirection: 'column',
    rowGap: '1rem',
  },
  renderAccessTokenFormDivItem: {
    marginBottom: {
      default: null,
      ':not(:last-child)': '0.5rem',
    },
  },
  renderAccessTokenFormP: {
    fontSize: '0.75rem',
    lineHeight: lineHeights.textXs,
    color: colors.mutedForeground,
  },
  renderAccessTokenFormDiv: {
    fontSize: '0.875rem',
    lineHeight: lineHeights.textSm,
    fontWeight: 500,
  },
  renderAccessTokenFormToggleGroup: {
    display: 'grid',
    width: '100%',
    gridTemplateColumns: 'repeat(4, minmax(0, 1fr))',
    gap: '0.25rem',
    borderStyle: 'solid',
    borderWidth: '0px',
    backgroundColor: colors.muted,
    padding: '0.25rem',
  },
  renderAccessTokenFormToggleGroupItem: {
    height: {
      default: '2.25rem',
      '@media (min-width: 48rem)': '2.25rem',
    },
    justifyContent: 'center',
    fontSize: '0.875rem',
    lineHeight: lineHeights.textSm,
    backgroundColor: {
      default: null,
      ':is([data-state="on"])': colors.background,
    },
    fontWeight: {
      default: null,
      ':is([data-state="on"])': 700,
    },
    color: {
      default: colors.mutedForeground,
      ':is([data-state="on"])': colors.foreground,
      ':hover': {
        default: null,
        '@media (hover: hover)': colors.foreground,
      },
    },
    boxShadow: {
      default: null,
      ':focus-visible': {
        default: `0 0 0 2px ${colors.ring}`,
        ':is([data-state="on"])': `0 0 0 2px ${colors.ring}, ${shadows.paper}`,
      },
      ':is([data-state="on"])': shadows.paper,
    },
  },
  renderAccessTokenFormButton: {
    width: '100%',
  },
  renderAccessTokenListMotionDiv: {
    display: 'flex',
    flexDirection: 'column',
    alignItems: 'center',
    gap: '0.75rem',
    paddingInline: '1.5rem',
    paddingBlock: '2.5rem',
    textAlign: 'center',
  },
  renderAccessTokenListSpan: {
    display: 'flex',
    width: '3rem',
    height: '3rem',
    alignItems: 'center',
    justifyContent: 'center',
    borderRadius: 'calc(infinity * 1px)',
    backgroundColor: colors.muted,
    color: colors.mutedForeground,
  },
  renderAccessTokenListKeyRound: {
    width: '1.25rem',
    height: '1.25rem',
  },
  renderAccessTokenListP: {
    fontSize: '0.875rem',
    lineHeight: lineHeights.textSm,
    fontWeight: 700,
  },
  renderAccessTokenListP2: {
    marginTop: '0.25rem',
    fontSize: '0.75rem',
    lineHeight: lineHeights.textXs,
    color: colors.mutedForeground,
  },
  renderAccessTokenListMotionDiv2: {
    display: 'flex',
    alignItems: 'center',
    gap: '0.75rem',
    overflow: 'hidden',
    paddingInline: {
      default: '1rem',
      '@media (min-width: 40rem)': '1.25rem',
    },
    paddingBlock: '0.875rem',
  },
  renderAccessTokenListSpan2: {
    display: 'flex',
    width: '2.25rem',
    height: '2.25rem',
    flexShrink: 0,
    alignItems: 'center',
    justifyContent: 'center',
    borderRadius: '8px',
    backgroundColor: colors.muted,
    color: colors.mutedForeground,
  },
  renderAccessTokenListDiv: {
    minWidth: '0px',
    flex: '1',
  },
  renderAccessTokenListP3: {
    overflow: 'hidden',
    textOverflow: 'ellipsis',
    whiteSpace: 'nowrap',
    fontSize: '0.875rem',
    lineHeight: lineHeights.textSm,
    fontWeight: 700,
  },
  renderAccessTokenListP4: {
    fontSize: '0.75rem',
    lineHeight: lineHeights.textXs,
    color: colors.mutedForeground,
    fontVariantNumeric: 'tabular-nums',
  },
  renderAccessTokenListStatusPill: {
    display: {
      default: 'none',
      '@media (min-width: 40rem)': 'inline-flex',
    },
  },
  renderAccessTokenListButton: {
    width: '2.25rem',
    height: '2.25rem',
    flexShrink: 0,
    color: {
      default: colors.mutedForeground,
      ':hover': {
        default: null,
        '@media (hover: hover)': colors.destructive,
      },
    },
    backgroundColor: {
      default: null,
      ':active': {
        default: colors.accentPressed,
        ':hover': {
          default: null,
          '@media (hover: hover)': colors.accentPressed,
        },
      },
      ':hover': {
        default: null,
        '@media (hover: hover)': `color-mix(in oklab, ${colors.destructive} 10%, transparent)`,
      },
    },
  },
});
