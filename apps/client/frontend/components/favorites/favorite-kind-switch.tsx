/**
 * URL-backed switch between article favorites and saved relation graphs.
 */

import { FileText, Share2 } from '@/components/ui/animated-icons';
import { useSearch } from '@tanstack/react-router';

import { ToggleGroup, ToggleGroupItem } from '@/components/ui/toggle-group';
import {
  FAVORITE_KINDS,
  useUpdateWorkspaceSearch,
  WORKSPACE_ROUTE_ID,
  type FavoriteKind,
} from '@/lib/search-params';
import * as stylex from '@stylexjs/stylex';

const DEFAULT_FAVORITE_KIND = 'articles' satisfies FavoriteKind;

/**
 * Check whether a toggle value names a favorite kind.
 *
 * @param value - Selected toggle value.
 * @returns Whether the value is a favorite kind.
 */
function isFavoriteKind(value: string): value is FavoriteKind {
  return FAVORITE_KINDS.some((kind) => kind === value);
}

const FAVORITE_KIND_OPTIONS = [
  { kind: 'articles', label: '文章', icon: FileText },
  { kind: 'graphs', label: '关系图', icon: Share2 },
] as const;

/**
 * Read and change the favorite kind kept in the `kind` URL parameter.
 *
 * @returns Current kind and its setter.
 */
export function useFavoriteKind() {
  const kind = useSearch({
    from: WORKSPACE_ROUTE_ID,
    select: (search) => search.kind ?? DEFAULT_FAVORITE_KIND,
  });
  const updateSearch = useUpdateWorkspaceSearch();

  /**
   * Store a favorite kind, omitting the default from the URL.
   *
   * @param nextKind - Favorite kind to show.
   */
  const setKind = (nextKind: FavoriteKind) => {
    void updateSearch((search) => ({
      ...search,
      kind: nextKind === DEFAULT_FAVORITE_KIND ? undefined : nextKind,
    }));
  };
  return [kind, setKind] as const;
}

/**
 * Render the article / relation graph switch of the favorites page.
 *
 * @returns Segmented toggle group bound to the URL.
 */
export function FavoriteKindSwitch() {
  const [kind, setKind] = useFavoriteKind();

  return (
    <ToggleGroup
      type="single"
      aria-label="收藏类型"
      value={kind}
      onValueChange={(option) => {
        if (isFavoriteKind(option)) setKind(option);
      }}
    >
      {FAVORITE_KIND_OPTIONS.map(({ kind: option, label, icon: Icon }) => (
        <ToggleGroupItem key={option} value={option}>
          <Icon {...stylex.props(styles.favoriteKindSwitchIcon)} aria-hidden="true" />
          {label}
        </ToggleGroupItem>
      ))}
    </ToggleGroup>
  );
}

const styles = stylex.create({
  favoriteKindSwitchIcon: {
    width: '0.875rem',
    height: '0.875rem',
  },
});
