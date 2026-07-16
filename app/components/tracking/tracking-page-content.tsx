'use client';

/**
 * Tracking page composition and cross-section navigation coordination.
 */

import { ArrowLeft, Radar } from 'lucide-react';
import Link from 'next/link';
import { useRouter } from 'next/navigation';
import { useState } from 'react';

import { ManualPushCard } from '@/components/tracking/manual-push-card';
import { RecommendationSettingsCard } from '@/components/tracking/recommendation-settings-card';
import { TrackingFolderCard } from '@/components/tracking/tracking-folder-card';
import { TrackingHelpCard } from '@/components/tracking/tracking-help-card';
import { useTrackingPage } from '@/components/tracking/use-tracking-page';
import { Button } from '@/components/ui/button';
import { ConfirmDialog } from '@/components/ui/confirm-dialog';

/**
 * Render the tracking page from grouped section view models.
 *
 * @param props - Authenticated user identifier.
 * @returns Tracking page composition and unsaved-leave confirmation.
 */
export function TrackingPageContent({ userId }: { userId: number }) {
  const router = useRouter();
  const [isLeaveConfirmOpen, setIsLeaveConfirmOpen] = useState(false);
  const trackingPage = useTrackingPage(userId);

  return (
    <main
      id="main-content"
      className="mx-auto max-w-3xl space-y-4 p-4 sm:space-y-6 sm:p-6"
      style={{
        paddingBottom:
          'calc(6rem + var(--safe-area-inset-bottom, env(safe-area-inset-bottom, 0px)))',
      }}
    >
      <div className="flex items-start gap-2 sm:gap-3">
        <Button variant="ghost" size="icon" aria-label="返回首页" asChild>
          <Link
            href="/"
            onClick={(event) => {
              if (trackingPage.hasUnsavedSettings) {
                event.preventDefault();
                setIsLeaveConfirmOpen(true);
              }
            }}
          >
            <ArrowLeft className="h-5 w-5" />
          </Link>
        </Button>
        <h1 className="flex items-center gap-2 text-2xl font-bold">
          <Radar className="h-6 w-6" />
          文献追踪
        </h1>
      </div>

      <TrackingFolderCard model={trackingPage.folder} />
      <ManualPushCard model={trackingPage.manualPush} />
      <RecommendationSettingsCard model={trackingPage.recommendation} />
      <TrackingHelpCard />

      <ConfirmDialog
        open={isLeaveConfirmOpen}
        onOpenChange={setIsLeaveConfirmOpen}
        title="离开未保存的配置？"
        description="当前推荐配置尚未保存，确认离开？未保存的更改将会丢失。"
        actionLabel="确认离开"
        onConfirm={() => {
          setIsLeaveConfirmOpen(false);
          router.push('/');
        }}
      />
    </main>
  );
}
