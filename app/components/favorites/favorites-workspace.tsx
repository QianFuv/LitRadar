'use client';

/**
 * Favorites workspace that switches between article favorites and saved relation graphs.
 */

import { useFavoriteKind } from '@/components/favorites/favorite-kind-switch';
import { FavoritesPageContent } from '@/components/favorites/favorites-page-content';
import { RelationGraphFavoritesContent } from '@/components/favorites/relation-graph-favorites-content';
import { MotionDiv, MotionPresence, PANEL_VARIANTS } from '@/components/ui/motion';

/**
 * Render the favorites kind chosen in the URL, swapping kinds like tab panels.
 *
 * @param props - Authenticated user identifier.
 * @returns Article favorites or saved relation graphs.
 */
export function FavoritesWorkspace({ userId }: { userId: number }) {
  const [kind] = useFavoriteKind();

  return (
    <MotionPresence mode="wait">
      <MotionDiv
        key={kind}
        data-favorite-kind={kind}
        className="h-full min-h-0"
        variants={PANEL_VARIANTS}
        initial="hidden"
        animate="visible"
        exit="exit"
      >
        {kind === 'graphs' ? (
          <RelationGraphFavoritesContent />
        ) : (
          <FavoritesPageContent userId={userId} />
        )}
      </MotionDiv>
    </MotionPresence>
  );
}
