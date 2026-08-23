'use client';

/**
 * Query-routed protected workspaces with restrained view transitions.
 */

import { useQueryState } from 'nuqs';
import type { ReactNode } from 'react';

import { SearchWorkspaceView } from '@/components/feature/search-workspace-view';
import { FavoritesPageContent } from '@/components/favorites/favorites-page-content';
import {
  FADE_UP_VARIANTS,
  MOTION_DURATION_SECONDS,
  MotionDiv,
  MotionPresence,
  useMotionTransition,
} from '@/components/ui/motion';
import { WeeklyUpdatesView } from '@/components/weekly/weekly-updates-view';
import { useAuth } from '@/lib/auth-context';
import { WORKSPACE_VIEW_PARSER } from '@/lib/workspace-view';

/**
 * Render the article search workspace with responsive filters and results.
 *
 * @returns Protected homepage search UI.
 */
export default function Home() {
  const { user } = useAuth();
  const [view] = useQueryState('view', WORKSPACE_VIEW_PARSER);
  const transition = useMotionTransition(MOTION_DURATION_SECONDS.base);
  let workspaceContent: ReactNode;

  if (view === 'favorites') {
    workspaceContent = user ? <FavoritesPageContent userId={user.id} /> : null;
  } else if (view === 'weekly-updates') {
    workspaceContent = <WeeklyUpdatesView />;
  } else {
    workspaceContent = <SearchWorkspaceView />;
  }

  return (
    <MotionPresence mode="wait">
      <MotionDiv
        key={view}
        data-workspace-view={view}
        className="h-dvh"
        variants={FADE_UP_VARIANTS}
        initial="hidden"
        animate="visible"
        exit={{ opacity: 0, pointerEvents: 'none', y: -4 }}
        transition={transition}
      >
        {workspaceContent}
      </MotionDiv>
    </MotionPresence>
  );
}
