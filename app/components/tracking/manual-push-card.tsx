'use client';

/**
 * Manual tracking-push section.
 */

import { Download } from 'lucide-react';

import {
  SettingsSection,
  SettingsSectionContent,
  SettingsSectionDescription,
  SettingsSectionHeader,
  SettingsSectionTitle,
} from '@/components/settings/settings-section';
import type { TrackingPageViewModel } from '@/components/tracking/use-tracking-page';
import { Button } from '@/components/ui/button';

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
          <Button
            className="w-full sm:w-auto"
            onClick={() => model.mutation.mutate()}
            disabled={
              model.mutation.isPending ||
              model.isPolling ||
              (model.requiresTrackingFolder && !model.trackingFolder)
            }
          >
            <Download className="mr-1 h-4 w-4" />
            {model.label}
          </Button>
        </div>
        {model.result && (
          <div
            role={model.mutation.isError ? 'alert' : 'status'}
            className="rounded-md border px-3 py-2 text-sm"
          >
            {model.result}
          </div>
        )}
      </SettingsSectionContent>
    </SettingsSection>
  );
}
