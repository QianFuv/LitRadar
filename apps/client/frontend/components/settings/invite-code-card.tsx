import { useState } from 'react';
import { useMutation, useQuery } from '@tanstack/react-query';
import { CircleX, Gift, RotateCcw } from '@/components/ui/animated-icons';

import {
  generateInviteCode,
  getInviteCode,
  revokeInviteCode,
  rotateInviteCode,
  type InviteCodeStatus,
} from '@/lib/api';
import {
  SettingsGroup,
  SettingsRow,
  SettingsSection,
  SettingsSectionContent,
  SettingsSectionDescription,
  SettingsSectionHeader,
  SettingsSectionTitle,
  StatusPill,
  settingsGroupItemStyle,
} from '@/components/settings/settings-section';
import type {
  SettingsCopyFeedback,
  SettingsCopyScope,
} from '@/components/settings/use-settings-copy';
import { Button } from '@/components/ui/button';
import { CopyButton } from '@/components/ui/copy-button';
import { ConfirmDialog } from '@/components/ui/confirm-dialog';
import { RevealItem } from '@/components/ui/reveal';
import { Skeleton } from '@/components/ui/skeleton';
import { Progress } from '@/components/ui/progress';
import * as stylex from '@stylexjs/stylex';
import { colors, lineHeights } from '@/lib/theme.stylex';

const INVITE_STATUS_LABELS: Record<InviteCodeStatus, string> = {
  active: '可用',
  expired: '已过期',
  revoked: '已撤销',
  exhausted: '已用尽',
};

/**
 * Format a Unix timestamp for invite lifecycle details.
 *
 * @param timestamp - Unix timestamp in seconds.
 * @returns Localized date and time.
 */
function formatInviteDate(timestamp: number): string {
  return new Date(timestamp * 1000).toLocaleString('zh-CN');
}

/**
 * Render and manage the current user's invite-code lifecycle.
 *
 * @param props - Shared copy feedback and action.
 * @returns Invite-code settings card.
 */
export function InviteCodeCard(props: {
  copyFeedback: SettingsCopyFeedback | null;
  handleCopy: (value: string, successMessage: string, scope: SettingsCopyScope) => Promise<void>;
}) {
  const state = useInviteViewState(props);
  const {
    inviteCodeData,
    isInviteCodeLoading,
    generateInviteMut,
    rotateInviteMut,
    revokeInviteMut,
    mutationError,
    isMutating,
  } = state;
  return (
    <SettingsSection>
      <SettingsSectionHeader>
        <SettingsSectionTitle icon={Gift}>邀请码</SettingsSectionTitle>
        <SettingsSectionDescription>
          邀请码默认有效 7 天且可注册 1 次，可随时轮换或永久撤销
        </SettingsSectionDescription>
      </SettingsSectionHeader>
      <SettingsSectionContent>
        <RevealItem
          isReady={!isInviteCodeLoading}
          placeholder={<Skeleton xstyle={styles.inviteCodeCardSkeleton} />}
        >
          {inviteCodeData ? (
            renderCurrentInviteCode(state, inviteCodeData)
          ) : (
            <SettingsGroup>
              <SettingsRow
                leading={
                  <span {...stylex.props(styles.inviteCodeCardSpan)}>
                    <Gift xstyle={styles.inviteCodeCardGift} aria-hidden="true" />
                  </span>
                }
                title="还没有邀请码"
                description="生成后即可分享给同事注册 LitRadar"
              >
                <Button
                  size="sm"
                  onClick={() => {
                    rotateInviteMut.reset();
                    revokeInviteMut.reset();
                    generateInviteMut.mutate();
                  }}
                  disabled={isMutating}
                >
                  生成邀请码
                </Button>
              </SettingsRow>
            </SettingsGroup>
          )}
        </RevealItem>
        {mutationError && (
          <p role="alert" {...stylex.props(styles.inviteCodeCardP)}>
            {mutationError instanceof Error ? mutationError.message : '邀请码操作失败'}
          </p>
        )}
        {renderInviteRevokeConfirmation(state)}
      </SettingsSectionContent>
    </SettingsSection>
  );
}
/** Own the original settings hooks and exact mutation/cache completion order. */
function useInviteViewState({
  copyFeedback,
  handleCopy,
}: {
  copyFeedback: SettingsCopyFeedback | null;
  handleCopy: (value: string, successMessage: string, scope: SettingsCopyScope) => Promise<void>;
}) {
  const [isRevokeOpen, setIsRevokeOpen] = useState(false);
  const {
    data: inviteCodeData,
    isPending: isInviteCodeLoading,
    refetch: refetchInviteCode,
  } = useQuery({
    queryKey: ['invite-code'],
    queryFn: () => getInviteCode(),
    enabled: true,
  });
  const generateInviteMut = useMutation({
    mutationFn: () => generateInviteCode(),
    onSuccess: () => void refetchInviteCode(),
  });
  const rotateInviteMut = useMutation({
    mutationFn: () => rotateInviteCode(),
    onSuccess: () => void refetchInviteCode(),
  });
  const revokeInviteMut = useMutation({
    mutationFn: () => revokeInviteCode(),
    onSuccess: () => {
      setIsRevokeOpen(false);
      void refetchInviteCode();
    },
  });
  const mutationError = generateInviteMut.error ?? rotateInviteMut.error ?? revokeInviteMut.error;
  const isMutating =
    generateInviteMut.isPending || rotateInviteMut.isPending || revokeInviteMut.isPending;

  return {
    copyFeedback,
    handleCopy,
    isRevokeOpen,
    setIsRevokeOpen,
    inviteCodeData,
    isInviteCodeLoading,
    generateInviteMut,
    rotateInviteMut,
    revokeInviteMut,
    mutationError,
    isMutating,
  };
}
type InviteViewState = ReturnType<typeof useInviteViewState>;

/** Retain invite-only copy feedback and accessible tone. */
function renderInviteCopyFeedback(state: Pick<InviteViewState, 'copyFeedback'>) {
  const { copyFeedback } = state;

  return (
    copyFeedback?.scope === 'invite' && (
      <p
        role={copyFeedback.tone === 'error' ? 'alert' : 'status'}
        {...stylex.props(
          copyFeedback.tone === 'error'
            ? styles.renderInviteCopyFeedbackPError
            : styles.renderInviteCopyFeedbackPNotError,
        )}
      >
        {copyFeedback.message}
      </p>
    )
  );
}

/** Retain status-dependent copying, lifecycle metadata and reset-before-action order. */
function renderCurrentInviteCode(
  state: InviteViewState,
  inviteCodeData: NonNullable<InviteViewState['inviteCodeData']>,
) {
  const {
    handleCopy,
    setIsRevokeOpen,
    generateInviteMut,
    rotateInviteMut,
    revokeInviteMut,
    isMutating,
  } = state;

  const usageRatio =
    inviteCodeData.max_uses > 0
      ? Math.min(1, inviteCodeData.use_count / inviteCodeData.max_uses)
      : 0;
  const isActive = inviteCodeData.status === 'active';

  return (
    <SettingsGroup>
      <div {...stylex.props(settingsGroupItemStyle, styles.renderCurrentInviteCodeDiv)}>
        <div
          {...stylex.props(
            styles.renderCurrentInviteCodeDiv2,
            isActive
              ? styles.renderCurrentInviteCodeDivActive
              : styles.renderCurrentInviteCodeDivNotActive,
          )}
        >
          <p {...stylex.props(styles.renderCurrentInviteCodeP)}>当前邀请码</p>
          <code
            {...stylex.props(
              styles.renderCurrentInviteCodeCode,
              isActive
                ? styles.renderCurrentInviteCodeCodeActive
                : styles.renderCurrentInviteCodeCodeNotActive,
            )}
          >
            {inviteCodeData.code}
          </code>
        </div>
        <CopyButton
          variant="outline"
          xstyle={styles.renderCurrentInviteCodeCopyButton}
          aria-label="复制邀请码"
          disabled={!isActive}
          isCopied={state.copyFeedback?.scope === 'invite' && state.copyFeedback.tone === 'success'}
          onCopy={() => void handleCopy(inviteCodeData.code, '邀请码已复制。', 'invite')}
        />
      </div>
      {state.copyFeedback?.scope === 'invite' && (
        <div
          {...stylex.props(
            settingsGroupItemStyle,
            state.copyFeedback.tone === 'error'
              ? styles.renderCurrentInviteCodeDivError
              : styles.renderCurrentInviteCodeDivNotError,
          )}
        >
          {renderInviteCopyFeedback(state)}
        </div>
      )}
      <dl {...stylex.props(settingsGroupItemStyle, styles.renderCurrentInviteCodeDl)}>
        <div
          {...stylex.props(
            styles.renderCurrentInviteCodeDlItem,
            styles.renderCurrentInviteCodeDiv3,
          )}
        >
          <dt {...stylex.props(styles.renderCurrentInviteCodeP)}>状态</dt>
          <dd {...stylex.props(styles.renderCurrentInviteCodeDd)}>
            <StatusPill tone={isActive ? 'success' : 'neutral'}>
              {INVITE_STATUS_LABELS[inviteCodeData.status]}
            </StatusPill>
          </dd>
        </div>
        <div
          {...stylex.props(
            styles.renderCurrentInviteCodeDlItem,
            styles.renderCurrentInviteCodeDiv3,
          )}
        >
          <dt {...stylex.props(styles.renderCurrentInviteCodeP)}>使用情况</dt>
          <dd {...stylex.props(styles.renderCurrentInviteCodeDd2)}>
            <span {...stylex.props(styles.renderCurrentInviteCodeSpan)}>
              已使用 {inviteCodeData.use_count}/{inviteCodeData.max_uses} 次
            </span>
            <Progress
              aria-hidden="true"
              xstyle={styles.renderCurrentInviteCodeProgress}
              value={usageRatio * 100}
            />
          </dd>
        </div>
        <div
          {...stylex.props(
            styles.renderCurrentInviteCodeDlItem,
            styles.renderCurrentInviteCodeDiv3,
          )}
        >
          <dt {...stylex.props(styles.renderCurrentInviteCodeP)}>有效期至</dt>
          <dd {...stylex.props(styles.renderCurrentInviteCodeDd3)}>
            {formatInviteDate(inviteCodeData.expires_at)}
          </dd>
        </div>
      </dl>
      <div {...stylex.props(settingsGroupItemStyle, styles.renderCurrentInviteCodeDiv4)}>
        {inviteCodeData.revoked_at === null && (
          <Button
            variant="ghost"
            size="sm"
            xstyle={styles.renderCurrentInviteCodeButton}
            disabled={isMutating}
            onClick={() => {
              generateInviteMut.reset();
              rotateInviteMut.reset();
              revokeInviteMut.reset();
              setIsRevokeOpen(true);
            }}
          >
            <CircleX xstyle={styles.renderCurrentInviteCodeCircleX} />
            撤销邀请码
          </Button>
        )}
        <Button
          variant="outline"
          size="sm"
          disabled={isMutating}
          onClick={() => {
            generateInviteMut.reset();
            revokeInviteMut.reset();
            rotateInviteMut.mutate();
          }}
        >
          <RotateCcw xstyle={styles.renderCurrentInviteCodeCircleX} />
          轮换邀请码
        </Button>
      </div>
    </SettingsGroup>
  );
}

/** Retain the explicit revoke confirmation and pending dismissal guard. */
function renderInviteRevokeConfirmation(
  state: Pick<InviteViewState, 'isRevokeOpen' | 'setIsRevokeOpen' | 'revokeInviteMut'>,
) {
  const { isRevokeOpen, setIsRevokeOpen, revokeInviteMut } = state;

  return (
    <ConfirmDialog
      open={isRevokeOpen}
      onOpenChange={(nextOpen) => {
        if (!revokeInviteMut.isPending) {
          setIsRevokeOpen(nextOpen);
        }
      }}
      title="撤销邀请码？"
      description="撤销后该邀请码将永久失效；如需新邀请码，可以随后轮换。"
      actionLabel="确认撤销"
      pendingLabel="撤销中…"
      isPending={revokeInviteMut.isPending}
      error={revokeInviteMut.error instanceof Error ? revokeInviteMut.error.message : null}
      onConfirm={() => revokeInviteMut.mutate()}
    />
  );
}

const styles = stylex.create({
  inviteCodeCardSkeleton: {
    height: '4rem',
    width: '100%',
    borderRadius: '12px',
  },
  inviteCodeCardSpan: {
    display: 'flex',
    width: '2.5rem',
    height: '2.5rem',
    flexShrink: 0,
    alignItems: 'center',
    justifyContent: 'center',
    borderRadius: 'calc(infinity * 1px)',
    backgroundColor: colors.info,
    color: colors.primary,
  },
  inviteCodeCardGift: {
    width: '1.25rem',
    height: '1.25rem',
  },
  inviteCodeCardP: {
    marginTop: '0.5rem',
    paddingInline: '0.25rem',
    fontSize: '0.875rem',
    lineHeight: lineHeights.textSm,
    color: colors.destructive,
  },
  renderInviteCopyFeedbackPError: {
    fontSize: '0.875rem',
    lineHeight: lineHeights.textSm,
    color: colors.destructive,
  },
  renderInviteCopyFeedbackPNotError: {
    fontSize: '0.875rem',
    lineHeight: lineHeights.textSm,
    color: colors.mutedForeground,
  },
  renderCurrentInviteCodeDiv: {
    display: 'flex',
    alignItems: 'stretch',
    gap: '0.75rem',
    paddingInline: {
      default: '1rem',
      '@media (min-width: 40rem)': '1.25rem',
    },
    paddingBlock: '1rem',
  },
  renderCurrentInviteCodeDiv2: {
    position: 'relative',
    minWidth: '0px',
    flex: '1',
    borderRadius: '8px',
    borderStyle: 'dashed',
    borderWidth: '1px',
    paddingInline: '1rem',
    paddingBlock: '0.75rem',
  },
  renderCurrentInviteCodeDivActive: {
    borderColor: `color-mix(in oklab, ${colors.primary} 50%, transparent)`,
    backgroundColor: colors.info,
  },
  renderCurrentInviteCodeDivNotActive: {
    backgroundColor: `color-mix(in oklab, ${colors.muted} 60%, transparent)`,
  },
  renderCurrentInviteCodeP: {
    fontSize: '0.75rem',
    lineHeight: lineHeights.textXs,
    color: colors.mutedForeground,
  },
  renderCurrentInviteCodeCode: {
    marginTop: '0.25rem',
    display: 'block',
    fontSize: {
      default: '1rem',
      '@media (min-width: 40rem)': '1.125rem',
    },
    lineHeight: {
      default: '1.5rem',
      '@media (min-width: 40rem)': '1.5rem',
    },
    fontWeight: 700,
    letterSpacing: '0.025em',
    wordBreak: 'break-all',
  },
  renderCurrentInviteCodeCodeActive: {
    color: colors.infoForeground,
  },
  renderCurrentInviteCodeCodeNotActive: {
    color: colors.mutedForeground,
    textDecorationLine: 'line-through',
  },
  renderCurrentInviteCodeCopyButton: {
    height: 'auto',
    width: '3rem',
    flexShrink: 0,
  },
  renderCurrentInviteCodeDivError: {
    paddingInline: {
      default: '1rem',
      '@media (min-width: 40rem)': '1.25rem',
    },
    paddingBlock: '0.625rem',
  },
  renderCurrentInviteCodeDivNotError: {
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
  renderCurrentInviteCodeDlItem: {
    borderTopStyle: {
      default: null,
      ':not(:last-child)': 'solid',
      '@media (min-width: 40rem)': {
        default: null,
        ':not(:last-child)': 'solid',
      },
    },
    borderTopWidth: {
      default: null,
      ':not(:last-child)': '0px',
      '@media (min-width: 40rem)': {
        default: null,
        ':not(:last-child)': '0px',
      },
    },
    borderBottomStyle: {
      default: null,
      ':not(:last-child)': 'solid',
      '@media (min-width: 40rem)': {
        default: null,
        ':not(:last-child)': 'solid',
      },
    },
    borderBottomWidth: {
      default: null,
      ':not(:last-child)': '1px',
      '@media (min-width: 40rem)': {
        default: null,
        ':not(:last-child)': '0px',
      },
    },
    borderInlineStartStyle: {
      default: null,
      '@media (min-width: 40rem)': {
        default: null,
        ':not(:last-child)': 'solid',
      },
    },
    borderInlineStartWidth: {
      default: null,
      '@media (min-width: 40rem)': {
        default: null,
        ':not(:last-child)': '0px',
      },
    },
    borderInlineEndStyle: {
      default: null,
      '@media (min-width: 40rem)': {
        default: null,
        ':not(:last-child)': 'solid',
      },
    },
    borderInlineEndWidth: {
      default: null,
      '@media (min-width: 40rem)': {
        default: null,
        ':not(:last-child)': '1px',
      },
    },
  },
  renderCurrentInviteCodeDl: {
    display: 'grid',
    gridTemplateColumns: {
      default: 'repeat(1, minmax(0, 1fr))',
      '@media (min-width: 40rem)': 'repeat(3, minmax(0, 1fr))',
    },
    fontSize: '0.875rem',
    lineHeight: lineHeights.textSm,
  },
  renderCurrentInviteCodeDiv3: {
    display: {
      default: 'flex',
      '@media (min-width: 40rem)': 'block',
    },
    alignItems: 'center',
    justifyContent: 'space-between',
    gap: '0.75rem',
    paddingInline: {
      default: '1rem',
      '@media (min-width: 40rem)': '1.25rem',
    },
    paddingBlock: '0.75rem',
  },
  renderCurrentInviteCodeDd: {
    marginTop: {
      default: null,
      '@media (min-width: 40rem)': '0.375rem',
    },
  },
  renderCurrentInviteCodeDd2: {
    display: 'flex',
    alignItems: 'center',
    gap: '0.5rem',
    marginTop: {
      default: null,
      '@media (min-width: 40rem)': '0.375rem',
    },
  },
  renderCurrentInviteCodeSpan: {
    fontVariantNumeric: 'tabular-nums',
  },
  renderCurrentInviteCodeProgress: {
    display: {
      default: 'none',
      '@media (min-width: 40rem)': 'block',
    },
    width: '4rem',
  },
  renderCurrentInviteCodeDd3: {
    fontVariantNumeric: 'tabular-nums',
    marginTop: {
      default: null,
      '@media (min-width: 40rem)': '0.375rem',
    },
  },
  renderCurrentInviteCodeDiv4: {
    display: 'flex',
    flexWrap: 'wrap',
    justifyContent: 'flex-end',
    gap: '0.5rem',
    backgroundColor: `color-mix(in oklab, ${colors.muted} 40%, transparent)`,
    paddingInline: {
      default: '1rem',
      '@media (min-width: 40rem)': '1.25rem',
    },
    paddingBlock: '0.75rem',
  },
  renderCurrentInviteCodeButton: {
    color: {
      default: colors.destructive,
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
  renderCurrentInviteCodeCircleX: {
    width: '1rem',
    height: '1rem',
  },
});
