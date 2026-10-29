'use client';

/**
 * Manual tracking-push section.
 */

import { Download, ShieldCheck, X } from 'lucide-react';

import {
  SettingsGroup,
  SettingsSection,
  SettingsSectionDescription,
  SettingsSectionHeader,
  SettingsSectionTitle,
} from '@/components/settings/settings-section';
import type { TrackingPageViewModel } from '@/components/tracking/use-tracking-page';
import { Button } from '@/components/ui/button';
import {
  FADE_UP_VARIANTS,
  MOTION_DURATION_SECONDS,
  MotionDiv,
  MotionPresence,
  useMotionTransition,
} from '@/components/ui/motion';

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
  const feedbackTransition = useMotionTransition(MOTION_DURATION_SECONDS.fast);

  const state = { model, feedbackTransition };
  return (
    <SettingsSection>
      <SettingsSectionHeader>
        <SettingsSectionTitle>手动推送</SettingsSectionTitle>
        <SettingsSectionDescription>{model.description}</SettingsSectionDescription>
      </SettingsSectionHeader>
      <SettingsGroup>
        <div className="flex flex-col gap-4 px-4 py-5 sm:flex-row sm:items-center sm:justify-between sm:px-5">
          <div className="flex items-center gap-4">
            <span className="text-4xl leading-none font-bold tabular-nums">
              {model.weeklyArticlesAvailable ?? '…'}
            </span>
            <span className="text-xs leading-relaxed text-muted-foreground">
              本周可推送文章（篇）
            </span>
          </div>
          <div className="flex w-full gap-2 sm:w-auto">
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
              className={
                model.hasError
                  ? 'bg-destructive/5 px-4 py-3 text-sm text-destructive sm:px-5'
                  : 'bg-muted/40 px-4 py-3 text-sm sm:px-5'
              }
              initial="hidden"
              animate="visible"
              exit="exit"
              variants={FADE_UP_VARIANTS}
              transition={feedbackTransition}
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
      className="flex-1 sm:flex-none"
      variant="outline"
      onClick={() => model.acknowledgeUnknownMutation.mutate(model.status?.job_id ?? '')}
      disabled={model.acknowledgeUnknownMutation.isPending}
    >
      <ShieldCheck className="size-4" />
      {model.acknowledgeUnknownMutation.isPending ? '确认中…' : '确认未知并继续'}
    </Button>
  );
}

/** Retain the cancellable job action and its original pending guard. */
function renderManualPushCancel(state: ManualPushViewState) {
  const { model } = state;

  return (
    <Button
      className="flex-1 sm:flex-none"
      variant="outline"
      onClick={() => model.cancelMutation.mutate(model.status?.job_id ?? '')}
      disabled={model.cancelMutation.isPending}
    >
      <X className="size-4" />
      {model.cancelMutation.isPending ? '取消中…' : '取消任务'}
    </Button>
  );
}

/** Retain every original push admission guard without adding cancel-pending blocking. */
function renderManualPushTrigger(state: ManualPushViewState) {
  const { model } = state;

  return (
    <Button
      className="flex-1 sm:flex-none"
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
      <Download className="size-4" />
      {model.label}
    </Button>
  );
}

type ManualPushViewState = ManualPushCardProps & {
  feedbackTransition: ReturnType<typeof useMotionTransition>;
};
