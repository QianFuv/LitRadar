/**
 * Shared tracking-settings session rendered by the tracking and notification categories.
 */

import { Save } from '@/components/ui/animated-icons';
import { useEffect, useMemo } from 'react';

import { DeliverySettingsSection } from '@/components/tracking/delivery-settings-section';
import { ManualPushCard } from '@/components/tracking/manual-push-card';
import { RecommendationSettingsSection } from '@/components/tracking/recommendation-settings-section';
import { TrackingFolderCard } from '@/components/tracking/tracking-folder-card';
import { TrackingHelpCard } from '@/components/tracking/tracking-help-card';
import { useTrackingPage } from '@/components/tracking/use-tracking-page';
import { Button } from '@/components/ui/button';
import { MotionDiv, MotionPresence, PANEL_VARIANTS } from '@/components/ui/motion';
import { RevealItem } from '@/components/ui/reveal';
import { Skeleton } from '@/components/ui/skeleton';
import * as stylex from '@stylexjs/stylex';
import { colors, lineHeights } from '@/lib/theme.stylex';

type TrackingSettingsSectionId = 'tracking' | 'notifications';

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

  const state = { trackingPage, recommendation, isInitialLoading, isInitialError, section };
  return (
    <div {...stylex.props(styles.trackingSettingsContentDiv)}>
      <fieldset
        {...stylex.props(styles.trackingSettingsContentFieldset)}
        disabled={recommendation.save.mutation.isPending}
      >
        <MotionPresence mode="wait">
          <MotionDiv
            key={section}
            variants={PANEL_VARIANTS}
            initial="hidden"
            animate="visible"
            exit="exit"
          >
            {renderTrackingCategory(state)}
          </MotionDiv>
        </MotionPresence>
      </fieldset>

      <RevealItem isReady={!isInitialLoading} placeholder={null}>
        {!isInitialError && renderTrackingSaveBar(state)}
      </RevealItem>
    </div>
  );
}

/**
 * Render the loading placeholder of one tracking block.
 *
 * @param heightStyle - Height of the block being loaded.
 * @returns Status placeholder with a skeleton of the block.
 */
function renderTrackingBlockPlaceholder(heightStyle: stylex.StyleXStyles) {
  return (
    <output {...stylex.props(styles.statusOutput, styles.renderTrackingBlockPlaceholderDiv)}>
      <span {...stylex.props(styles.renderTrackingBlockPlaceholderSpan)}>
        正在加载已保存的推荐配置…
      </span>
      <Skeleton xstyle={[styles.trackingBlockSkeleton, heightStyle]} />
    </output>
  );
}

/** Retain draft-aware initial loading/error and category content precedence. */
function renderTrackingCategory(state: TrackingSettingsViewState) {
  const { section, trackingPage, recommendation, isInitialLoading, isInitialError } = state;
  if (isInitialError)
    return (
      <div role="alert" {...stylex.props(styles.renderTrackingCategoryDiv)}>
        {recommendation.notificationQuery.error instanceof Error
          ? recommendation.notificationQuery.error.message
          : '加载推荐配置失败'}
      </div>
    );
  if (section === 'tracking')
    return (
      <>
        <TrackingHelpCard />
        <RevealItem
          isReady={!trackingPage.folder.isLoading && !trackingPage.isStatusLoading}
          placeholder={renderTrackingBlockPlaceholder(styles.blockHeightSmall)}
        >
          <TrackingFolderCard model={trackingPage.folder} />
        </RevealItem>
        <RevealItem
          isReady={
            !isInitialLoading &&
            !recommendation.databaseSelection.query.isPending &&
            !recommendation.ai.endpoints.query.isPending
          }
          placeholder={renderTrackingBlockPlaceholder(styles.blockHeightLarge)}
        >
          <RecommendationSettingsSection model={recommendation} />
        </RevealItem>
      </>
    );
  return (
    <>
      <RevealItem
        isReady={!isInitialLoading}
        placeholder={renderTrackingBlockPlaceholder(styles.blockHeightMedium)}
      >
        <DeliverySettingsSection model={recommendation} />
      </RevealItem>
      <RevealItem
        isReady={!trackingPage.manualPush.isLoading && !trackingPage.isStatusLoading}
        placeholder={renderTrackingBlockPlaceholder(styles.blockHeightSmall)}
      >
        <ManualPushCard model={trackingPage.manualPush} />
      </RevealItem>
    </>
  );
}

/** Retain unavailable database, saved and failed-save feedback order. */
function renderTrackingSaveFeedback(state: TrackingSettingsViewState) {
  const { recommendation } = state;

  return (
    <div {...stylex.props(styles.renderTrackingSaveFeedbackDiv)}>
      {recommendation.databaseSelection.unavailable.length > 0 && (
        <span role="alert" {...stylex.props(styles.renderTrackingSaveFeedbackSpan)}>
          推送数据库已不可用，请在“文献追踪”中重新选择或明确设为全部。
        </span>
      )}
      {recommendation.save.didSave && (
        <output {...stylex.props(styles.renderTrackingSaveFeedbackSpan2)}>已保存</output>
      )}
      {recommendation.save.mutation.isError && (
        <span role="alert" {...stylex.props(styles.renderTrackingSaveFeedbackSpan)}>
          {recommendation.save.mutation.error instanceof Error
            ? recommendation.save.mutation.error.message
            : '保存失败'}
        </span>
      )}
    </div>
  );
}

/** Retain discard/save admission guards and draft ownership. */
function renderTrackingSaveBar(state: TrackingSettingsViewState) {
  const { trackingPage, recommendation } = state;

  return (
    <div {...stylex.props(styles.renderTrackingSaveBarDiv)}>
      {renderTrackingSaveFeedback(state)}
      <Button
        type="button"
        variant="outline"
        xstyle={styles.renderTrackingSaveBarButton}
        disabled={!trackingPage.hasUnsavedSettings || recommendation.save.mutation.isPending}
        onClick={trackingPage.discardSettings}
      >
        取消更改
      </Button>
      <Button
        type="button"
        xstyle={styles.renderTrackingSaveBarButton}
        disabled={
          !trackingPage.hasUnsavedSettings ||
          recommendation.save.mutation.isPending ||
          recommendation.databaseSelection.unavailable.length > 0
        }
        onClick={() => recommendation.save.mutation.mutate()}
      >
        <Save xstyle={styles.renderTrackingSaveBarSave} />
        {recommendation.save.mutation.isPending ? '保存中…' : '保存更改'}
      </Button>
    </div>
  );
}

type TrackingSettingsViewState = {
  trackingPage: ReturnType<typeof useTrackingPage>;
  recommendation: ReturnType<typeof useTrackingPage>['recommendation'];
  isInitialLoading: boolean;
  isInitialError: boolean;
  section: TrackingSettingsSectionId;
};

const styles = stylex.create({
  statusOutput: {
    display: 'block',
  },
  trackingBlockSkeleton: {
    width: '100%',
    borderRadius: '12px',
  },
  blockHeightSmall: {
    height: '8rem',
  },
  blockHeightMedium: {
    height: '16rem',
  },
  blockHeightLarge: {
    height: '24rem',
  },
  trackingSettingsContentDiv: {
    display: 'flex',
    minHeight: '100%',
    flexDirection: 'column',
  },
  trackingSettingsContentFieldset: {
    margin: '0px',
    minWidth: '0px',
    flex: '1',
    borderStyle: 'solid',
    borderWidth: '0px',
    padding: '0px',
  },
  renderTrackingBlockPlaceholderDiv: {
    paddingBlock: '1rem',
  },
  renderTrackingBlockPlaceholderSpan: {
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
  renderTrackingCategoryDiv: {
    borderRadius: '6px',
    borderStyle: 'solid',
    borderWidth: '1px',
    borderColor: `color-mix(in oklab, ${colors.destructive} 50%, transparent)`,
    paddingInline: '0.75rem',
    paddingBlock: '1rem',
    fontSize: '0.875rem',
    lineHeight: lineHeights.textSm,
    color: colors.destructive,
  },
  renderTrackingSaveFeedbackDiv: {
    gridColumnStart: 'span 2',
    gridColumnEnd: 'span 2',
    flex: '1',
    fontSize: '0.875rem',
    lineHeight: lineHeights.textSm,
    display: {
      default: null,
      ':empty': 'none',
    },
  },
  renderTrackingSaveFeedbackSpan: {
    color: colors.destructive,
  },
  renderTrackingSaveFeedbackSpan2: {
    color: colors.successForeground,
  },
  renderTrackingSaveBarDiv: {
    position: 'sticky',
    bottom: '0px',
    marginTop: '2.5rem',
    display: {
      default: 'grid',
      '@media (min-width: 40rem)': 'flex',
    },
    gridTemplateColumns: 'repeat(2, minmax(0, 1fr))',
    gap: {
      default: '0.5rem',
      '@media (min-width: 40rem)': '0.75rem',
    },
    borderTopStyle: 'solid',
    borderTopWidth: '1px',
    backgroundColor: colors.background,
    paddingBlock: {
      default: '0.75rem',
      '@media (min-width: 40rem)': '1rem',
    },
    alignItems: {
      default: null,
      '@media (min-width: 40rem)': 'center',
    },
    justifyContent: {
      default: null,
      '@media (min-width: 40rem)': 'flex-end',
    },
  },
  renderTrackingSaveBarButton: {
    width: {
      default: '100%',
      '@media (min-width: 40rem)': 'auto',
    },
  },
  renderTrackingSaveBarSave: {
    width: '1rem',
    height: '1rem',
  },
});
