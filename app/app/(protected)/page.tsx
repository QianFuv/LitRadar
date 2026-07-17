'use client';

import { SearchWorkspaceView } from '@/components/feature/search-workspace-view';
import { FavoritesPageContent } from '@/components/favorites/favorites-page-content';
import { WeeklyUpdatesView } from '@/components/weekly/weekly-updates-view';
import { useAuth } from '@/lib/auth-context';
import { WORKSPACE_VIEW_PARSER } from '@/lib/workspace-view';
import { useQueryState } from 'nuqs';

/**
 * Render the article search workspace with responsive filters and results.
 *
 * @returns Protected homepage search UI.
 */
export default function Home() {
  const { user } = useAuth();
  const [view] = useQueryState('view', WORKSPACE_VIEW_PARSER);

  if (view === 'favorites') {
    return user ? <FavoritesPageContent userId={user.id} /> : null;
  }

  if (view === 'weekly-updates') {
    return <WeeklyUpdatesView />;
  }

  return <SearchWorkspaceView />;
}
