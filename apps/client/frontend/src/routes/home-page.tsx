/**
 * Query-routed protected workspaces with restrained view transitions.
 */

import { useQueryState } from 'nuqs';
import { Suspense, type ReactNode } from 'react';

import { SearchWorkspaceView } from '@/components/feature/search-workspace-view';
import {
  CFP_TRACKING_VIEW,
  FAVORITES_WORKSPACE,
  RELATIONS_WORKSPACE_VIEW,
  WEEKLY_UPDATES_VIEW,
  WorkspaceViewFallback,
} from '@/components/shell/lazy-workspaces';
import { MotionDiv, MotionPresence, PANEL_VARIANTS } from '@/components/ui/motion';
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
  let workspaceContent: ReactNode;

  if (view === 'favorites') {
    workspaceContent = user ? <FAVORITES_WORKSPACE.Component userId={user.id} /> : null;
  } else if (view === 'weekly-updates') {
    workspaceContent = <WEEKLY_UPDATES_VIEW.Component />;
  } else if (view === 'cfp-tracking') {
    workspaceContent = <CFP_TRACKING_VIEW.Component />;
  } else if (view === 'relations') {
    workspaceContent = <RELATIONS_WORKSPACE_VIEW.Component />;
  } else {
    workspaceContent = <SearchWorkspaceView />;
  }

  return (
    <MotionPresence mode="wait">
      <MotionDiv
        key={view}
        data-workspace-view={view}
        className="h-full min-h-0"
        variants={PANEL_VARIANTS}
        initial="hidden"
        animate="visible"
        exit="exit"
      >
        <Suspense fallback={<WorkspaceViewFallback />}>{workspaceContent}</Suspense>
      </MotionDiv>
    </MotionPresence>
  );
}
