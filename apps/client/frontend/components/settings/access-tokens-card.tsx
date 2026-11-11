import { useState } from 'react';
import { z } from 'zod';
import { daysUntilEpochSeconds } from '@/lib/date';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { Check, Key, KeyRound, Plus, Trash2 } from '@/components/ui/animated-icons';

import {
  createAccessToken,
  accessTokensQuery,
  revokeAccessToken,
  type AccessToken,
} from '@/lib/api';
import {
  SettingsGroup,
  SettingsSection,
  SettingsSectionContent,
  SettingsSectionDescription,
  SettingsSectionHeader,
  SettingsSectionTitle,
  StatusPill,
  settingsGroupItemStyle,
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
import { useAppForm } from '@/lib/form';
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
 * Count the Unicode code points of a token name, the unit the server limits.
 *
 * @param name - Token name.
 * @returns Code point count.
 */
function countCodePoints(name: string): number {
  return Array.from(name).length;
}

/** Token creation fields; the name may be empty but not longer than the server allows. */
const ACCESS_TOKEN_FORM_SCHEMA = z.object({
  name: z
    .string()
    .refine(
      (name) => countCodePoints(name) <= ACCESS_TOKEN_NAME_MAX_CODE_POINTS,
      ACCESS_TOKEN_NAME_LENGTH_DETAIL,
    ),
  ttl: z.number(),
});

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
            <DialogTrigger render={<Button size="sm" xstyle={styles.accessTokensCardButton} />}>
              <Plus xstyle={styles.accessTokensCardPlus} />
              新建
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
  const [newTokenValue, setNewTokenValue] = useState<string | null>(null);
  const [tokenToRevoke, setTokenToRevoke] = useState<AccessToken | null>(null);
  const [dialogOpen, setDialogOpen] = useState(false);
  const { data: tokens = [], isPending: isTokensLoading } = useQuery({
    ...accessTokensQuery(),
  });
  const createTokenMut = useMutation({
    mutationFn: (values: z.output<typeof ACCESS_TOKEN_FORM_SCHEMA>) =>
      createAccessToken(values.name, values.ttl),
    onSuccess: (data) => {
      setNewTokenValue(data.token);
      void queryClient.invalidateQueries({ queryKey: accessTokensQuery().queryKey });
      tokenForm.resetField('name');
    },
  });
  const tokenForm = useAppForm({
    defaultValues: { name: '', ttl: TTL_OPTIONS[0].value },
    validators: { onChange: ACCESS_TOKEN_FORM_SCHEMA },
    onSubmit: ({ value }) => createTokenMut.mutate(ACCESS_TOKEN_FORM_SCHEMA.parse(value)),
  });
  const creationError = createTokenMut.error?.message ?? null;
  const revokeMut = useMutation({
    mutationFn: (id: number) => revokeAccessToken(id),
    onSuccess: (_data, tokenId) => {
      void queryClient.invalidateQueries({ queryKey: accessTokensQuery().queryKey });
      setTokenToRevoke((current) => (current?.id === tokenId ? null : current));
    },
  });

  return {
    copyFeedback,
    handleCopy,
    isTokensLoading,
    tokenForm,
    newTokenValue,
    setNewTokenValue,
    tokenToRevoke,
    setTokenToRevoke,
    dialogOpen,
    setDialogOpen,
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
          <Check xstyle={styles.renderCreatedAccessTokenCheck} strokeWidth={3} aria-hidden="true" />
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
  const { tokenForm, createTokenMut, creationError } = state;

  return (
    <MotionForm
      key="token-form"
      data-motion-token-dialog-state="form"
      noValidate
      onSubmit={(event) => {
        event.preventDefault();
        void tokenForm.handleSubmit();
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
        <tokenForm.AppField name="name">
          {(field) => (
            <field.TextField
              xstyle={styles.renderAccessTokenFormDivItem}
              id="access-token-name"
              name="access_token_name"
              autoComplete="off"
              spellCheck={false}
              placeholder="例如：接口集成"
            />
          )}
        </tokenForm.AppField>
        <tokenForm.Subscribe selector={(formState) => countCodePoints(formState.values.name)}>
          {(codePointCount) => (
            <p
              {...stylex.props(styles.renderAccessTokenFormDivItem, styles.renderAccessTokenFormP)}
            >
              {codePointCount}/{ACCESS_TOKEN_NAME_MAX_CODE_POINTS} Unicode code points
            </p>
          )}
        </tokenForm.Subscribe>
      </div>
      <div>
        <div
          {...stylex.props(styles.renderAccessTokenFormDivItem, styles.renderAccessTokenFormDiv)}
        >
          有效期
        </div>
        <tokenForm.Field name="ttl">
          {(field) => (
            <ToggleGroup
              aria-label="访问令牌有效期"
              xstyle={[
                styles.renderAccessTokenFormDivItem,
                styles.renderAccessTokenFormToggleGroup,
              ]}
              value={String(field.state.value)}
              onValueChange={(value) => field.handleChange(Number(value))}
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
          )}
        </tokenForm.Field>
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
      <tokenForm.Subscribe selector={(formState) => formState.canSubmit}>
        {(canSubmit) => (
          <Button
            type="submit"
            xstyle={styles.renderAccessTokenFormButton}
            disabled={createTokenMut.isPending || !canSubmit}
          >
            创建
          </Button>
        )}
      </tokenForm.Subscribe>
    </MotionForm>
  );
}

/** Retain empty/list branches and directly keyed revocation rows. */
function renderAccessTokenList(
  state: Pick<AccessTokenViewState, 'setTokenToRevoke' | 'tokens' | 'revokeMut'>,
) {
  const { setTokenToRevoke, tokens, revokeMut } = state;

  return (
    <MotionPresence>
      {tokens.length === 0 ? (
        <MotionDiv
          key="empty-tokens"
          {...stylex.props(settingsGroupItemStyle, styles.renderAccessTokenListMotionDiv)}
          initial="hidden"
          animate="visible"
          exit="exit"
          variants={REVEAL_VARIANTS}
        >
          <span {...stylex.props(styles.renderAccessTokenListSpan)}>
            <KeyRound xstyle={styles.renderAccessTokenListKeyRound} aria-hidden="true" />
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
          const remainingDays = daysUntilEpochSeconds(t.expires_at);
          return (
            <MotionDiv
              key={t.id}
              data-motion-token-key={t.id}
              {...stylex.props(settingsGroupItemStyle, styles.renderAccessTokenListMotionDiv2)}
              initial="hidden"
              animate="visible"
              exit="exit"
              variants={DISCLOSURE_VARIANTS}
            >
              <span {...stylex.props(styles.renderAccessTokenListSpan2)}>
                <Key xstyle={styles.accessTokensCardPlus} aria-hidden="true" />
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
                <Trash2 xstyle={styles.accessTokensCardPlus} />
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
      ':is([data-pressed])': colors.background,
    },
    fontWeight: {
      default: null,
      ':is([data-pressed])': 700,
    },
    color: {
      default: colors.mutedForeground,
      ':is([data-pressed])': colors.foreground,
      ':hover': {
        default: null,
        '@media (hover: hover)': colors.foreground,
      },
    },
    boxShadow: {
      default: null,
      ':focus-visible': {
        default: `0 0 0 2px ${colors.ring}`,
        ':is([data-pressed])': `0 0 0 2px ${colors.ring}, ${shadows.paper}`,
      },
      ':is([data-pressed])': shadows.paper,
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
