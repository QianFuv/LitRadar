/**
 * Query-routed protected workspaces with restrained view transitions.
 */

import { useSearch } from '@tanstack/react-router';
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
import { WORKSPACE_ROUTE_ID } from '@/lib/search-params';
import { DEFAULT_WORKSPACE_VIEW } from '@/lib/workspace-view';
import * as stylex from '@stylexjs/stylex';

/**
 * Render the article search workspace with responsive filters and results.
 *
 * @returns Protected homepage search UI.
 */
export default function Home() {
  const { user } = useAuth();
  const view = useSearch({
    from: WORKSPACE_ROUTE_ID,
    select: (search) => search.view ?? DEFAULT_WORKSPACE_VIEW,
  });
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
        {...stylex.props(styles.homeMotionDiv)}
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

const styles = stylex.create({
  homeMotionDiv: {
    height: '100%',
    minHeight: '0px',
  },
});
