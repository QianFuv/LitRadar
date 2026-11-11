import { useState } from 'react';
import { z } from 'zod';
import { epochMsAfterDays } from '@/lib/date';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { CircleX, Gift, Plus } from '@/components/ui/animated-icons';

import {
  adminCreateInviteCode,
  adminRevokeInviteCode,
  type AdminInviteCode,
  type InviteCodeStatus,
  adminStatsQuery,
  adminInviteCodesQuery,
} from '@/lib/api';
import { copyTextToClipboard } from '@/lib/clipboard';
import { AdminListHeader } from '@/components/admin/admin-list-header';
import {
  SettingsGroup,
  StatusPill,
  settingsGroupItemStyle,
} from '@/components/settings/settings-section';
import { Button } from '@/components/ui/button';
import { IconTooltip } from '@/components/ui/tooltip';
import { CopyButton } from '@/components/ui/copy-button';
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/components/ui/card';
import { ConfirmDialog } from '@/components/ui/confirm-dialog';
import {
  DISCLOSURE_VARIANTS,
  MotionDiv,
  MotionParagraph,
  MotionPresence,
  REVEAL_VARIANTS,
} from '@/components/ui/motion';
import { Progress } from '@/components/ui/progress';
import { RevealItem } from '@/components/ui/reveal';
import { LoadingText } from '@/components/ui/loading-text';
import * as stylex from '@stylexjs/stylex';
import { colors, lineHeights } from '@/lib/theme.stylex';
import { useAppForm } from '@/lib/form';
import { useTransientState } from '@/lib/use-transient-state';

/** Time copy feedback stays visible after the last copy. */
const COPY_FEEDBACK_MS = 3000;

const INVITE_STATUS_LABELS: Record<InviteCodeStatus, string> = {
  active: '可用',
  expired: '已过期',
  revoked: '已撤销',
  exhausted: '已用尽',
};

/** Invite policy limits: whole days of validity and registrations per code. */
const INVITE_POLICY_SCHEMA = z.object({
  validDays: z
    .number('有效天数必须为 1 到 365 的整数。')
    .int('有效天数必须为 1 到 365 的整数。')
    .min(1, '有效天数必须为 1 到 365 的整数。')
    .max(365, '有效天数必须为 1 到 365 的整数。'),
  maxUses: z
    .number('使用次数必须为 1 到 1000 的整数。')
    .int('使用次数必须为 1 到 1000 的整数。')
    .min(1, '使用次数必须为 1 到 1000 的整数。')
    .max(1000, '使用次数必须为 1 到 1000 的整数。'),
});

/** Draft invite policy; an emptied number field is null until it is filled again. */
type InvitePolicyDraft = { validDays: number | null; maxUses: number | null };

const DEFAULT_INVITE_POLICY: InvitePolicyDraft = {
  validDays: 7,
  maxUses: 1,
};

/**
 * Format a Unix timestamp for administrator tables.
 *
 * @param timestamp - Unix timestamp in seconds.
 * @returns Localized date and time.
 */
function formatDate(timestamp: number): string {
  return new Date(timestamp * 1000).toLocaleString('zh-CN');
}

/**
 * Render administrator invite-code creation and lifecycle management.
 *
 * @param props - Whether administrator queries may run.
 * @returns Invite-code management card.
 */
export function AdminInviteCodesCard(props: { isEnabled: boolean }) {
  const state = useAdminInviteViewState(props);
  const { policyForm, inviteCodesError, isLoading, createCodeMut } = state;
  return (
    <Card>
      <CardHeader>
        <CardTitle xstyle={styles.adminInviteCodesCardCardTitle}>
          <Gift xstyle={styles.adminInviteCodesCardGift} />
          邀请码管理
        </CardTitle>
        <CardDescription>创建有期限和使用次数上限的邀请码，并保留撤销历史</CardDescription>
      </CardHeader>
      <CardContent xstyle={styles.adminInviteCodesCardCardContent}>
        <SettingsGroup>
          <div {...stylex.props(settingsGroupItemStyle, styles.adminInviteCodesCardDiv)}>
            <div {...stylex.props(styles.adminInviteCodesCardDiv2)}>
              <span {...stylex.props(styles.adminInviteCodesCardSpan)}>
                <Gift xstyle={styles.adminInviteCodesCardGift2} aria-hidden="true" />
              </span>
              <div>
                <p {...stylex.props(styles.adminInviteCodesCardP)}>生成新邀请码</p>
                <p {...stylex.props(styles.adminInviteCodesCardP2)}>设置有效期与可注册次数</p>
              </div>
            </div>
            <form
              noValidate
              aria-label="生成新邀请码"
              onSubmit={(event) => {
                event.preventDefault();
                void policyForm.handleSubmit();
              }}
              {...stylex.props(styles.adminInviteCodesCardDiv3)}
            >
              <div>
                <label
                  htmlFor="invite-valid-days"
                  {...stylex.props(
                    styles.adminInviteCodesCardLabelItem,
                    styles.adminInviteCodesCardSpan2,
                  )}
                >
                  有效天数
                </label>
                <policyForm.AppField name="validDays">
                  {(field) => (
                    <field.NumberField
                      id="invite-valid-days"
                      min={1}
                      max={365}
                      step={1}
                      fieldXstyle={styles.adminInviteCodesCardField}
                      xstyle={styles.adminInviteCodesCardInput}
                    />
                  )}
                </policyForm.AppField>
              </div>
              <div>
                <label
                  htmlFor="invite-max-uses"
                  {...stylex.props(
                    styles.adminInviteCodesCardLabelItem,
                    styles.adminInviteCodesCardSpan2,
                  )}
                >
                  最多使用次数
                </label>
                <policyForm.AppField name="maxUses">
                  {(field) => (
                    <field.NumberField
                      id="invite-max-uses"
                      min={1}
                      max={1000}
                      step={1}
                      fieldXstyle={styles.adminInviteCodesCardField}
                      xstyle={styles.adminInviteCodesCardInput}
                    />
                  )}
                </policyForm.AppField>
              </div>
              <policyForm.Subscribe selector={(formState) => formState.canSubmit}>
                {(canSubmit) => (
                  <Button
                    type="submit"
                    xstyle={styles.adminInviteCodesCardButton}
                    disabled={createCodeMut.isPending || !canSubmit}
                  >
                    <Plus xstyle={styles.adminInviteCodesCardPlus} />
                    生成邀请码
                  </Button>
                )}
              </policyForm.Subscribe>
            </form>
          </div>
        </SettingsGroup>
        {inviteCodesError instanceof Error && (
          <p role="alert" {...stylex.props(styles.adminInviteCodesCardP3)}>
            {inviteCodesError.message}
          </p>
        )}
        <MotionPresence>
          {createCodeMut.error instanceof Error && (
            <MotionParagraph
              key="invite-create-error"
              data-motion-feedback="invite-create"
              role="alert"
              {...stylex.props(styles.adminInviteCodesCardP3)}
              initial="hidden"
              animate="visible"
              exit="exit"
              variants={REVEAL_VARIANTS}
            >
              {createCodeMut.error.message}
            </MotionParagraph>
          )}
        </MotionPresence>
        {renderAdminInviteCopyFeedback(state)}
        <RevealItem
          isReady={!isLoading}
          xstyle={styles.adminInviteCodesCardCardContent}
          placeholder={
            <output {...stylex.props(styles.statusOutput, styles.adminInviteCodesCardP4)}>
              <LoadingText text="加载中…" />
            </output>
          }
        >
          {state.inviteRows.length > 0 && (
            <AdminListHeader
              summary={`共 ${state.inviteRows.length} 个邀请码 · ${
                state.inviteRows.filter(({ inviteCode }) => inviteCode.status === 'active').length
              } 个可用`}
            />
          )}
          {renderAdminInviteList(state)}
        </RevealItem>
        {renderAdminInviteRevocation(state)}
      </CardContent>
    </Card>
  );
}
/** Own the original hooks, draft stores and exact completion/cache order. */
function useAdminInviteViewState({ isEnabled }: { isEnabled: boolean }) {
  const queryClient = useQueryClient();
  const [copyFeedback, setCopyFeedback] = useTransientState<{
    code: string;
    message: string;
    tone: 'error' | 'success';
  } | null>(null, COPY_FEEDBACK_MS);
  const [inviteCodeToRevoke, setInviteCodeToRevoke] = useState<AdminInviteCode | null>(null);
  const {
    data: inviteCodes,
    error: inviteCodesError,
    isLoading,
  } = useQuery({
    ...adminInviteCodesQuery(),
    enabled: isEnabled,
  });
  const inviteRows = (inviteCodes ?? []).map((inviteCode) => ({
    inviteCode,
    expiresAtLabel: formatDate(inviteCode.expires_at),
    createdAtLabel: formatDate(inviteCode.created_at),
  }));
  const createCodeMut = useMutation({
    mutationFn: (policy: z.output<typeof INVITE_POLICY_SCHEMA>) =>
      adminCreateInviteCode({
        expires_at: epochMsAfterDays(policy.validDays) / 1000,
        max_uses: policy.maxUses,
      }),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: adminInviteCodesQuery().queryKey });
      void queryClient.invalidateQueries({ queryKey: adminStatsQuery().queryKey });
    },
  });
  const revokeCodeMut = useMutation({
    mutationFn: (codeId: number) => adminRevokeInviteCode(codeId),
    onSuccess: (_data, codeId) => {
      void queryClient.invalidateQueries({ queryKey: adminInviteCodesQuery().queryKey });
      void queryClient.invalidateQueries({ queryKey: adminStatsQuery().queryKey });
      setInviteCodeToRevoke((current) => (current?.id === codeId ? null : current));
    },
  });

  const policyForm = useAppForm({
    defaultValues: DEFAULT_INVITE_POLICY,
    validators: { onChange: INVITE_POLICY_SCHEMA },
    onSubmit: ({ value }) => createCodeMut.mutate(INVITE_POLICY_SCHEMA.parse(value)),
  });

  /** Copy an invite code and surface deterministic feedback. */
  const handleCopyInviteCode = async (code: string) => {
    try {
      await copyTextToClipboard(code);
      setCopyFeedback({ code, message: '邀请码已复制。', tone: 'success' });
    } catch {
      setCopyFeedback({ code, message: '复制失败，请手动选择文本复制。', tone: 'error' });
    }
  };

  return {
    policyForm,
    copyFeedback,
    inviteCodeToRevoke,
    setInviteCodeToRevoke,
    inviteRows,
    inviteCodesError,
    isLoading,
    createCodeMut,
    revokeCodeMut,
    handleCopyInviteCode,
  };
}
type AdminInviteViewState = ReturnType<typeof useAdminInviteViewState>;

/** Retain accessible clipboard outcomes and unchanged feedback timer ownership. */
function renderAdminInviteCopyFeedback(state: Pick<AdminInviteViewState, 'copyFeedback'>) {
  const { copyFeedback } = state;

  return (
    <MotionPresence>
      {copyFeedback && (
        <MotionParagraph
          key="invite-copy-feedback"
          data-motion-feedback="invite-copy"
          role={copyFeedback.tone === 'error' ? 'alert' : 'status'}
          {...stylex.props(
            copyFeedback.tone === 'error'
              ? styles.adminInviteCodesCardP3
              : styles.renderAdminInviteCopyFeedbackMotionParagraphNotError,
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
  );
}

/** Retain full-code copying, lifecycle metadata and revoke admission in one list. */
function renderAdminInviteList(state: AdminInviteViewState) {
  const { copyFeedback, setInviteCodeToRevoke, inviteRows, revokeCodeMut, handleCopyInviteCode } =
    state;

  return (
    <SettingsGroup>
      <MotionPresence>
        {inviteRows.length === 0 ? (
          <MotionDiv
            key="empty-invite-codes"
            {...stylex.props(settingsGroupItemStyle, styles.renderAdminInviteListMotionDiv)}
            initial="hidden"
            animate="visible"
            exit="exit"
            variants={REVEAL_VARIANTS}
          >
            暂无邀请码
          </MotionDiv>
        ) : (
          inviteRows.map(({ inviteCode, expiresAtLabel, createdAtLabel }) => {
            const isActive = inviteCode.status === 'active';
            const usageRatio =
              inviteCode.max_uses > 0 ? Math.min(1, inviteCode.use_count / inviteCode.max_uses) : 0;
            return (
              <MotionDiv
                key={inviteCode.id}
                data-motion-invite-key={inviteCode.id}
                {...stylex.props(settingsGroupItemStyle, styles.renderAdminInviteListMotionDiv2)}
                initial="hidden"
                animate="visible"
                exit="exit"
                variants={DISCLOSURE_VARIANTS}
              >
                <div {...stylex.props(styles.renderAdminInviteListDiv)}>
                  <span
                    aria-hidden="true"
                    {...stylex.props(
                      styles.renderAdminInviteListSpan,
                      isActive
                        ? styles.renderAdminInviteListSpanActive
                        : styles.renderAdminInviteListSpanNotActive,
                    )}
                  >
                    <Gift xstyle={styles.adminInviteCodesCardPlus} />
                  </span>
                  <div {...stylex.props(styles.renderAdminInviteListDiv2)}>
                    <div {...stylex.props(styles.renderAdminInviteListDiv3)}>
                      <code
                        {...stylex.props(
                          styles.renderAdminInviteListCode,
                          !isActive && styles.renderAdminInviteListCodeInactive,
                        )}
                      >
                        {inviteCode.code}
                      </code>
                      <IconTooltip label="复制">
                        <CopyButton
                          variant="ghost"
                          size="icon"
                          xstyle={styles.renderAdminInviteListCopyButton}
                          iconXstyle={styles.copyIcon}
                          aria-label="复制邀请码"
                          disabled={!isActive}
                          isCopied={
                            copyFeedback?.tone === 'success' &&
                            copyFeedback.code === inviteCode.code
                          }
                          onCopy={() => void handleCopyInviteCode(inviteCode.code)}
                        />
                      </IconTooltip>
                    </div>
                    <p {...stylex.props(styles.renderAdminInviteListP)}>
                      创建者 {inviteCode.created_by_name ?? '系统'} · 首位使用者{' '}
                      {inviteCode.used_by_name ?? '—'} · 创建于 <span>{createdAtLabel}</span> ·
                      有效期至 <span>{expiresAtLabel}</span>
                    </p>
                  </div>
                </div>
                <div {...stylex.props(styles.renderAdminInviteListDiv4)}>
                  <div {...stylex.props(styles.renderAdminInviteListDiv5)}>
                    <p {...stylex.props(styles.renderAdminInviteListP2)}>
                      已使用 {inviteCode.use_count}/{inviteCode.max_uses}
                    </p>
                    <Progress
                      aria-hidden="true"
                      xstyle={styles.renderAdminInviteListProgress}
                      value={usageRatio * 100}
                    />
                  </div>
                  <StatusPill
                    tone={isActive ? 'success' : 'neutral'}
                    xstyle={styles.renderAdminInviteListStatusPill}
                  >
                    {INVITE_STATUS_LABELS[inviteCode.status]}
                  </StatusPill>
                  <div {...stylex.props(styles.renderAdminInviteListDiv6)}>
                    {inviteCode.revoked_at === null && (
                      <IconTooltip label="撤销邀请码">
                        <Button
                          variant="ghost"
                          size="icon"
                          xstyle={styles.renderAdminInviteListButton}
                          aria-label={`撤销邀请码 ${inviteCode.code}`}
                          disabled={revokeCodeMut.isPending}
                          onClick={() => {
                            revokeCodeMut.reset();
                            setInviteCodeToRevoke(inviteCode);
                          }}
                        >
                          <CircleX xstyle={styles.adminInviteCodesCardPlus} />
                        </Button>
                      </IconTooltip>
                    )}
                  </div>
                </div>
              </MotionDiv>
            );
          })
        )}
      </MotionPresence>
    </SettingsGroup>
  );
}

/** Retain target identity, pending guard and permanent-revocation feedback. */
function renderAdminInviteRevocation(
  state: Pick<
    AdminInviteViewState,
    'inviteCodeToRevoke' | 'setInviteCodeToRevoke' | 'revokeCodeMut'
  >,
) {
  const { inviteCodeToRevoke, setInviteCodeToRevoke, revokeCodeMut } = state;

  return (
    <ConfirmDialog
      open={inviteCodeToRevoke !== null}
      onOpenChange={(nextOpen) => {
        if (!nextOpen && !revokeCodeMut.isPending) {
          setInviteCodeToRevoke(null);
        }
      }}
      title="撤销邀请码？"
      description={`确认永久撤销邀请码 ${inviteCodeToRevoke?.code ?? ''}？历史记录会保留。`}
      actionLabel="确认撤销"
      pendingLabel="撤销中…"
      isPending={revokeCodeMut.isPending}
      error={revokeCodeMut.error instanceof Error ? revokeCodeMut.error.message : null}
      onConfirm={() => {
        if (inviteCodeToRevoke) {
          revokeCodeMut.mutate(inviteCodeToRevoke.id);
        }
      }}
    />
  );
}

const styles = stylex.create({
  statusOutput: {
    display: 'block',
  },
  copyIcon: {
    width: '0.875rem',
    height: '0.875rem',
  },
  adminInviteCodesCardCardTitle: {
    display: 'flex',
    alignItems: 'center',
    gap: '0.5rem',
  },
  adminInviteCodesCardGift: {
    height: '1.25rem',
    width: '1.25rem',
  },
  adminInviteCodesCardCardContent: {
    display: 'flex',
    flexDirection: 'column',
    rowGap: '1rem',
  },
  adminInviteCodesCardDiv: {
    display: 'flex',
    flexDirection: {
      default: 'column',
      '@media (min-width: 64rem)': 'row',
    },
    gap: '1rem',
    paddingInline: {
      default: '1rem',
      '@media (min-width: 40rem)': '1.25rem',
    },
    paddingBlock: '1rem',
    alignItems: {
      default: null,
      '@media (min-width: 64rem)': 'flex-end',
    },
    justifyContent: {
      default: null,
      '@media (min-width: 64rem)': 'space-between',
    },
  },
  adminInviteCodesCardDiv2: {
    display: 'flex',
    alignItems: 'center',
    gap: '0.75rem',
  },
  adminInviteCodesCardSpan: {
    display: 'flex',
    width: '2.5rem',
    height: '2.5rem',
    flexShrink: 0,
    alignItems: 'center',
    justifyContent: 'center',
    borderRadius: '12px',
    backgroundColor: colors.primary,
    color: colors.primaryForeground,
  },
  adminInviteCodesCardGift2: {
    width: '1.25rem',
    height: '1.25rem',
  },
  adminInviteCodesCardP: {
    fontSize: '0.875rem',
    lineHeight: lineHeights.textSm,
    fontWeight: 700,
  },
  adminInviteCodesCardP2: {
    fontSize: '0.75rem',
    lineHeight: lineHeights.textXs,
    color: colors.mutedForeground,
  },
  adminInviteCodesCardDiv3: {
    display: 'flex',
    flexWrap: 'wrap',
    alignItems: 'flex-end',
    gap: '0.75rem',
  },
  adminInviteCodesCardLabelItem: {
    marginBottom: {
      default: null,
      ':not(:last-child)': '0.25rem',
    },
  },
  adminInviteCodesCardSpan2: {
    display: 'block',
    fontSize: '0.75rem',
    lineHeight: lineHeights.textXs,
    color: colors.mutedForeground,
  },
  adminInviteCodesCardField: {
    width: '6rem',
  },
  adminInviteCodesCardInput: {
    fontVariantNumeric: 'tabular-nums',
  },
  adminInviteCodesCardButton: {
    flex: {
      default: '1',
      '@media (min-width: 40rem)': 'none',
    },
  },
  adminInviteCodesCardPlus: {
    width: '1rem',
    height: '1rem',
  },
  adminInviteCodesCardP3: {
    fontSize: '0.875rem',
    lineHeight: lineHeights.textSm,
    color: colors.destructive,
  },
  adminInviteCodesCardP4: {
    fontSize: '0.875rem',
    lineHeight: lineHeights.textSm,
    color: colors.mutedForeground,
  },
  renderAdminInviteCopyFeedbackMotionParagraphNotError: {
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
  renderAdminInviteListMotionDiv: {
    paddingInline: '1.25rem',
    paddingBlock: '2.5rem',
    textAlign: 'center',
    fontSize: '0.875rem',
    lineHeight: lineHeights.textSm,
    color: colors.mutedForeground,
  },
  renderAdminInviteListMotionDiv2: {
    display: 'flex',
    flexDirection: {
      default: 'column',
      '@media (min-width: 48rem)': 'row',
    },
    gap: {
      default: '0.75rem',
      '@media (min-width: 48rem)': '1.25rem',
    },
    overflow: 'hidden',
    paddingInline: {
      default: '1rem',
      '@media (min-width: 40rem)': '1.25rem',
    },
    paddingBlock: '1rem',
    alignItems: {
      default: null,
      '@media (min-width: 48rem)': 'center',
    },
  },
  renderAdminInviteListDiv: {
    display: 'flex',
    minWidth: '0px',
    flex: '1',
    alignItems: 'flex-start',
    gap: '0.75rem',
  },
  renderAdminInviteListSpan: {
    marginTop: '0.125rem',
    display: 'flex',
    width: '2.25rem',
    height: '2.25rem',
    flexShrink: 0,
    alignItems: 'center',
    justifyContent: 'center',
    borderRadius: '8px',
  },
  renderAdminInviteListSpanActive: {
    backgroundColor: colors.info,
    color: colors.primary,
  },
  renderAdminInviteListSpanNotActive: {
    backgroundColor: colors.muted,
    color: colors.mutedForeground,
  },
  renderAdminInviteListDiv2: {
    minWidth: '0px',
    flex: '1',
  },
  renderAdminInviteListDiv3: {
    display: 'flex',
    alignItems: 'center',
    gap: '0.25rem',
  },
  renderAdminInviteListCode: {
    minWidth: '0px',
    fontSize: '0.875rem',
    lineHeight: lineHeights.textSm,
    fontWeight: 700,
    wordBreak: 'break-all',
  },
  renderAdminInviteListCodeInactive: {
    color: colors.mutedForeground,
    textDecorationLine: 'line-through',
  },
  renderAdminInviteListCopyButton: {
    width: '1.75rem',
    height: '1.75rem',
    flexShrink: 0,
    color: {
      default: colors.mutedForeground,
      ':hover': {
        default: null,
        '@media (hover: hover)': colors.accentForeground,
      },
    },
  },
  renderAdminInviteListP: {
    marginTop: '0.125rem',
    fontSize: '0.75rem',
    lineHeight: 1.625,
    color: colors.mutedForeground,
  },
  renderAdminInviteListDiv4: {
    display: 'flex',
    alignItems: 'center',
    gap: '1rem',
    paddingLeft: {
      default: '3rem',
      '@media (min-width: 48rem)': '0px',
    },
  },
  renderAdminInviteListDiv5: {
    width: '5rem',
  },
  renderAdminInviteListP2: {
    fontSize: '0.75rem',
    lineHeight: lineHeights.textXs,
    color: colors.mutedForeground,
    fontVariantNumeric: 'tabular-nums',
  },
  renderAdminInviteListProgress: {
    marginTop: '0.25rem',
  },
  renderAdminInviteListStatusPill: {
    width: '5rem',
    justifyContent: 'center',
  },
  renderAdminInviteListDiv6: {
    marginLeft: {
      default: 'auto',
      '@media (min-width: 48rem)': '0px',
    },
    width: '2.25rem',
  },
  renderAdminInviteListButton: {
    width: '2.25rem',
    height: '2.25rem',
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
