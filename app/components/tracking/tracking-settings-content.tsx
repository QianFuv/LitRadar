'use client';

/**
 * Shared tracking-settings session rendered by the tracking and notification categories.
 */

import { Save } from 'lucide-react';
import { useEffect, useMemo } from 'react';

import { DeliverySettingsSection } from '@/components/tracking/delivery-settings-section';
import { ManualPushCard } from '@/components/tracking/manual-push-card';
import { RecommendationSettingsSection } from '@/components/tracking/recommendation-settings-section';
import { TrackingFolderCard } from '@/components/tracking/tracking-folder-card';
import { TrackingHelpCard } from '@/components/tracking/tracking-help-card';
import { useTrackingPage } from '@/components/tracking/use-tracking-page';
import { Button } from '@/components/ui/button';

export type TrackingSettingsSectionId = 'tracking' | 'notifications';

/** Actions and state exposed to the settings-center transition guard. */
export type TrackingSettingsController = {
  discardSettings: () => void;
  hasUnsavedSettings: boolean;
  isSaving: boolean;
};

type TrackingSettingsContentProps = {
  onControllerChange?: (controller: TrackingSettingsController | null) => void;
  section: TrackingSettingsSectionId;
  userId: number;
};

/**
 * Render a shared tracking draft across recommendation and delivery categories.
 *
 * @param props - Authenticated user, active tracking category, and guard callback.
 * @returns Tracking category content with a shared sticky save bar.
 */
export function TrackingSettingsContent({
  onControllerChange,
  section,
  userId,
}: TrackingSettingsContentProps) {
  const trackingPage = useTrackingPage(userId);
  const controller = useMemo<TrackingSettingsController>(
    () => ({
      discardSettings: trackingPage.discardSettings,
      hasUnsavedSettings: trackingPage.hasUnsavedSettings,
      isSaving: trackingPage.recommendation.save.mutation.isPending,
    }),
    [
      trackingPage.discardSettings,
      trackingPage.hasUnsavedSettings,
      trackingPage.recommendation.save.mutation.isPending,
    ],
  );

  useEffect(() => {
    onControllerChange?.(controller);
  }, [controller, onControllerChange]);

  useEffect(
    () => () => {
      onControllerChange?.(null);
    },
    [onControllerChange],
  );

  const recommendation = trackingPage.recommendation;
  const isInitialLoading = recommendation.notificationQuery.isPending && !recommendation.hasDraft;
  const isInitialError = recommendation.notificationQuery.isError && !recommendation.hasDraft;

  return (
    <div className="flex min-h-full flex-col">
      <div className="flex-1">
        {isInitialLoading ? (
          <div role="status" className="rounded-md border px-3 py-4 text-sm text-muted-foreground">
            正在加载已保存的推荐配置…
          </div>
        ) : isInitialError ? (
          <div
            role="alert"
            className="rounded-md border border-destructive/50 px-3 py-4 text-sm text-destructive"
          >
            {recommendation.notificationQuery.error instanceof Error
              ? recommendation.notificationQuery.error.message
              : '加载推荐配置失败'}
          </div>
        ) : section === 'tracking' ? (
          <>
            <TrackingFolderCard model={trackingPage.folder} />
            <RecommendationSettingsSection model={recommendation} />
            <TrackingHelpCard />
          </>
        ) : (
          <>
            <DeliverySettingsSection model={recommendation} />
            <ManualPushCard model={trackingPage.manualPush} />
          </>
        )}
      </div>

      {!isInitialLoading && !isInitialError && (
        <div className="sticky bottom-0 -mx-5 mt-6 flex flex-col gap-3 border-t bg-background/95 px-5 py-4 backdrop-blur-sm sm:flex-row sm:items-center sm:justify-end md:-mx-8 md:px-8">
          <div className="min-h-5 flex-1 text-sm">
            {recommendation.save.didSave && (
              <span role="status" className="text-green-600 dark:text-green-400">
                已保存
              </span>
            )}
            {recommendation.save.mutation.isError && (
              <span role="alert" className="text-destructive">
                {recommendation.save.mutation.error instanceof Error
                  ? recommendation.save.mutation.error.message
                  : '保存失败'}
              </span>
            )}
          </div>
          <Button
            type="button"
            variant="outline"
            className="w-full sm:w-auto"
            disabled={!trackingPage.hasUnsavedSettings || recommendation.save.mutation.isPending}
            onClick={trackingPage.discardSettings}
          >
            取消更改
          </Button>
          <Button
            type="button"
            className="w-full sm:w-auto"
            disabled={!trackingPage.hasUnsavedSettings || recommendation.save.mutation.isPending}
            onClick={() => recommendation.save.mutation.mutate()}
          >
            <Save className="size-4" />
            {recommendation.save.mutation.isPending ? '保存中…' : '保存更改'}
          </Button>
        </div>
      )}
    </div>
  );
}
