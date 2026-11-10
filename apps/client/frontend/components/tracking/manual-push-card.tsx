/**
 * Manual tracking-push section.
 */

import { Download, ShieldCheck, X, SendHorizontal } from '@/components/ui/animated-icons';

import {
  SettingsGroup,
  SettingsSection,
  SettingsSectionDescription,
  SettingsSectionHeader,
  SettingsSectionTitle,
  settingsGroupItemStyle,
} from '@/components/settings/settings-section';
import type { TrackingPageViewModel } from '@/components/tracking/use-tracking-page';
import { Button } from '@/components/ui/button';
import { MotionDiv, MotionPresence, REVEAL_VARIANTS } from '@/components/ui/motion';
import * as stylex from '@stylexjs/stylex';
import { colors, lineHeights } from '@/lib/theme.stylex';

type ManualPushCardProps = {
  model: TrackingPageViewModel['manualPush'];
};

/**
 * Render manual push status and trigger controls.
 *
 * @param props - Manual-push-specific tracking view model.
 * @returns Manual push card.
 */
export function ManualPushCard({ model }: ManualPushCardProps) {
  const state = { model };
  return (
    <SettingsSection>
      <SettingsSectionHeader>
        <SettingsSectionTitle icon={SendHorizontal}>手动推送</SettingsSectionTitle>
        <SettingsSectionDescription>{model.description}</SettingsSectionDescription>
      </SettingsSectionHeader>
      <SettingsGroup>
        <div {...stylex.props(settingsGroupItemStyle, styles.manualPushCardDiv)}>
          <div {...stylex.props(styles.manualPushCardDiv2)}>
            <span {...stylex.props(styles.manualPushCardSpan)}>
              {model.weeklyArticlesAvailable ?? '…'}
            </span>
            <span {...stylex.props(styles.manualPushCardSpan2)}>本周可推送文章（篇）</span>
          </div>
          <div {...stylex.props(styles.manualPushCardDiv3)}>
            {model.status?.status === 'unknown' &&
              model.status.job_id &&
              renderUnknownPushAcknowledgement(state)}
            {model.status?.can_cancel && model.status.job_id && renderManualPushCancel(state)}
            {renderManualPushTrigger(state)}
          </div>
        </div>
        <MotionPresence>
          {model.result && (
            <MotionDiv
              key="manual-push-result"
              data-motion-feedback="manual-push"
              role={model.hasError ? 'alert' : 'status'}
              {...stylex.props(
                settingsGroupItemStyle,
                model.hasError
                  ? styles.manualPushCardMotionDivError
                  : styles.manualPushCardMotionDivNotError,
              )}
              initial="hidden"
              animate="visible"
              exit="exit"
              variants={REVEAL_VARIANTS}
            >
              {model.result}
            </MotionDiv>
          )}
        </MotionPresence>
      </SettingsGroup>
    </SettingsSection>
  );
}

/** Retain the explicit unknown-result acknowledgement action. */
function renderUnknownPushAcknowledgement(state: ManualPushViewState) {
  const { model } = state;

  return (
    <Button
      xstyle={styles.renderUnknownPushAcknowledgementButton}
      variant="outline"
      onClick={() => model.acknowledgeUnknownMutation.mutate(model.status?.job_id ?? '')}
      disabled={model.acknowledgeUnknownMutation.isPending}
    >
      <ShieldCheck xstyle={styles.renderUnknownPushAcknowledgementShieldCheck} />
      {model.acknowledgeUnknownMutation.isPending ? '确认中…' : '确认未知并继续'}
    </Button>
  );
}

/** Retain the cancellable job action and its original pending guard. */
function renderManualPushCancel(state: ManualPushViewState) {
  const { model } = state;

  return (
    <Button
      xstyle={styles.renderUnknownPushAcknowledgementButton}
      variant="outline"
      onClick={() => model.cancelMutation.mutate(model.status?.job_id ?? '')}
      disabled={model.cancelMutation.isPending}
    >
      <X xstyle={styles.renderUnknownPushAcknowledgementShieldCheck} />
      {model.cancelMutation.isPending ? '取消中…' : '取消任务'}
    </Button>
  );
}

/** Retain every original push admission guard without adding cancel-pending blocking. */
function renderManualPushTrigger(state: ManualPushViewState) {
  const { model } = state;

  return (
    <Button
      xstyle={styles.renderUnknownPushAcknowledgementButton}
      onClick={() => model.mutation.mutate()}
      disabled={
        model.isLoading ||
        model.mutation.isPending ||
        model.acknowledgeUnknownMutation.isPending ||
        model.isPolling ||
        model.status?.status === 'unknown' ||
        (model.requiresTrackingFolder && !model.trackingFolder)
      }
    >
      <Download xstyle={styles.renderUnknownPushAcknowledgementShieldCheck} />
      {model.label}
    </Button>
  );
}

type ManualPushViewState = ManualPushCardProps & {};

const styles = stylex.create({
  manualPushCardDiv: {
    display: 'flex',
    flexDirection: {
      default: 'column',
      '@media (min-width: 40rem)': 'row',
    },
    gap: '1rem',
    paddingInline: {
      default: '1rem',
      '@media (min-width: 40rem)': '1.25rem',
    },
    paddingBlock: '1.25rem',
    alignItems: {
      default: null,
      '@media (min-width: 40rem)': 'center',
    },
    justifyContent: {
      default: null,
      '@media (min-width: 40rem)': 'space-between',
    },
  },
  manualPushCardDiv2: {
    display: 'flex',
    alignItems: 'center',
    gap: '1rem',
  },
  manualPushCardSpan: {
    fontSize: '2.25rem',
    lineHeight: 1,
    fontWeight: 700,
    fontVariantNumeric: 'tabular-nums',
  },
  manualPushCardSpan2: {
    fontSize: '0.75rem',
    lineHeight: 1.625,
    color: colors.mutedForeground,
  },
  manualPushCardDiv3: {
    display: 'flex',
    width: {
      default: '100%',
      '@media (min-width: 40rem)': 'auto',
    },
    gap: '0.5rem',
  },
  manualPushCardMotionDivError: {
    backgroundColor: `color-mix(in oklab, ${colors.destructive} 5%, transparent)`,
    paddingInline: {
      default: '1rem',
      '@media (min-width: 40rem)': '1.25rem',
    },
    paddingBlock: '0.75rem',
    fontSize: '0.875rem',
    lineHeight: lineHeights.textSm,
    color: colors.destructive,
  },
  manualPushCardMotionDivNotError: {
    backgroundColor: `color-mix(in oklab, ${colors.muted} 40%, transparent)`,
    paddingInline: {
      default: '1rem',
      '@media (min-width: 40rem)': '1.25rem',
    },
    paddingBlock: '0.75rem',
    fontSize: '0.875rem',
    lineHeight: lineHeights.textSm,
  },
  renderUnknownPushAcknowledgementButton: {
    flex: {
      default: '1',
      '@media (min-width: 40rem)': 'none',
    },
  },
  renderUnknownPushAcknowledgementShieldCheck: {
    width: '1rem',
    height: '1rem',
  },
});
