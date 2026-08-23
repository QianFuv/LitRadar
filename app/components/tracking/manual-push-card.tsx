'use client';

/**
 * Manual tracking-push section.
 */

import { Download, ShieldCheck, X } from 'lucide-react';

import {
  SettingsSection,
  SettingsSectionContent,
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

  return (
    <SettingsSection>
      <SettingsSectionHeader>
        <SettingsSectionTitle>手动推送</SettingsSectionTitle>
        <SettingsSectionDescription>{model.description}</SettingsSectionDescription>
      </SettingsSectionHeader>
      <SettingsSectionContent className="space-y-4">
        <div className="flex flex-col gap-3 sm:flex-row sm:items-center sm:justify-between">
          <div className="text-sm text-muted-foreground">
            可推送文章: {model.weeklyArticlesAvailable ?? '…'} 篇
          </div>
          <div className="flex w-full gap-2 sm:w-auto">
            {model.status?.status === 'unknown' && model.status.job_id && (
              <Button
                className="flex-1 sm:flex-none"
                variant="outline"
                onClick={() => model.acknowledgeUnknownMutation.mutate(model.status?.job_id ?? '')}
                disabled={model.acknowledgeUnknownMutation.isPending}
              >
                <ShieldCheck className="mr-1 h-4 w-4" />
                {model.acknowledgeUnknownMutation.isPending ? '确认中…' : '确认未知并继续'}
              </Button>
            )}
            {model.status?.can_cancel && model.status.job_id && (
              <Button
                className="flex-1 sm:flex-none"
                variant="outline"
                onClick={() => model.cancelMutation.mutate(model.status?.job_id ?? '')}
                disabled={model.cancelMutation.isPending}
              >
                <X className="mr-1 h-4 w-4" />
                {model.cancelMutation.isPending ? '取消中…' : '取消任务'}
              </Button>
            )}
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
              <Download className="mr-1 h-4 w-4" />
              {model.label}
            </Button>
          </div>
        </div>
        <MotionPresence>
          {model.result && (
            <MotionDiv
              key="manual-push-result"
              data-motion-feedback="manual-push"
              role={model.hasError ? 'alert' : 'status'}
              className="rounded-md border px-3 py-2 text-sm"
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
      </SettingsSectionContent>
    </SettingsSection>
  );
}
