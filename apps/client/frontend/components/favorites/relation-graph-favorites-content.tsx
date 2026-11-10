/**
 * Saved relation graph list of the favorites page: open, rename, and delete saved graphs.
 */

import { useInfiniteQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import { MoreHorizontal, Share2 } from '@/components/ui/animated-icons';
import { Link } from '@tanstack/react-router';
import { useState } from 'react';

import { FavoriteKindSwitch } from '@/components/favorites/favorite-kind-switch';
import { ReaderPlaceholder } from '@/components/feature/reader-placeholder';
import { WorkspaceShell } from '@/components/feature/workspace-shell';
import {
  RELATION_GRAPH_FAVORITES_KEY,
  relationGraphFavoriteKey,
} from '@/components/relations/relation-graph-favorite-controls';
import { Button } from '@/components/ui/button';
import { ConfirmDialog } from '@/components/ui/confirm-dialog';
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog';
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from '@/components/ui/dropdown-menu';
import { Input } from '@/components/ui/input';
import { RevealItem } from '@/components/ui/reveal';
import { Skeleton } from '@/components/ui/skeleton';
import { StateMessage } from '@/components/ui/state-message';
import {
  deleteRelationGraphFavorite,
  getRelationGraphFavoritePage,
  updateRelationGraphFavorite,
  type RelationGraphFavoriteSummary,
} from '@/lib/api';
import { buildSavedRelationSearch } from '@/lib/relation-graph';
import * as stylex from '@stylexjs/stylex';
import { colors, lineHeights } from '@/lib/theme.stylex';

/** Saved graphs fetched per page. */
const RELATION_GRAPH_PAGE_SIZE = 50;

/**
 * Format a Unix timestamp in seconds as a local date and time.
 *
 * @param seconds - Unix timestamp in seconds.
 * @returns Localized date and time.
 */
function formatSavedTime(seconds: number): string {
  return new Date(seconds * 1000).toLocaleString('zh-CN', {
    year: 'numeric',
    month: '2-digit',
    day: '2-digit',
    hour: '2-digit',
    minute: '2-digit',
  });
}

/**
 * Render the user's saved relation graphs with open, rename, and delete actions.
 *
 * @returns Favorites workspace for saved relation graphs.
 */
export function RelationGraphFavoritesContent() {
  const queryClient = useQueryClient();
  const [renameTarget, setRenameTarget] = useState<RelationGraphFavoriteSummary | null>(null);
  const [renameDraft, setRenameDraft] = useState('');
  const [deleteTarget, setDeleteTarget] = useState<RelationGraphFavoriteSummary | null>(null);
  const pagesQuery = useInfiniteQuery({
    queryKey: RELATION_GRAPH_FAVORITES_KEY,
    queryFn: ({ pageParam, signal }) =>
      getRelationGraphFavoritePage(RELATION_GRAPH_PAGE_SIZE, pageParam, signal),
    initialPageParam: null as string | null,
    getNextPageParam: (page) => page.next_cursor,
  });
  const favorites = pagesQuery.data?.pages.flatMap((page) => page.items) ?? [];

  const renameMut = useMutation({
    mutationFn: ({ id, title }: { id: number; title: string }) =>
      updateRelationGraphFavorite(id, { title }),
    onSuccess: (favorite) => {
      queryClient.setQueryData(relationGraphFavoriteKey(favorite.id), favorite);
      void queryClient.invalidateQueries({ queryKey: RELATION_GRAPH_FAVORITES_KEY });
      setRenameTarget(null);
    },
  });
  const deleteMut = useMutation({
    mutationFn: (graphId: number) => deleteRelationGraphFavorite(graphId),
    onSuccess: (_data, graphId) => {
      queryClient.removeQueries({ queryKey: relationGraphFavoriteKey(graphId) });
      void queryClient.invalidateQueries({ queryKey: RELATION_GRAPH_FAVORITES_KEY });
      setDeleteTarget(null);
    },
  });

  /**
   * Open the rename dialog prefilled with the current title.
   *
   * @param favorite - Saved graph to rename.
   */
  const startRename = (favorite: RelationGraphFavoriteSummary) => {
    renameMut.reset();
    setRenameDraft(favorite.title);
    setRenameTarget(favorite);
  };

  return (
    <>
      <WorkspaceShell
        listLabel="收藏的关系图"
        emphasis="list"
        header={
          <div {...stylex.props(styles.relationGraphFavoritesContentDiv)}>
            <div {...stylex.props(styles.relationGraphFavoritesContentDiv2)}>
              <h1 {...stylex.props(styles.relationGraphFavoritesContentH1)}>我的收藏</h1>
              <p {...stylex.props(styles.relationGraphFavoritesContentP)}>
                保存的关系图会记住展开的分支和位置
              </p>
            </div>
            <FavoriteKindSwitch />
          </div>
        }
        readerPlaceholder={
          <ReaderPlaceholder
            title="打开一张关系图继续探索"
            description="点击左侧的关系图回到收藏时的网络，可在关系图页面更新或重置收藏。"
          />
        }
      >
        <RevealItem
          isReady={!pagesQuery.isPending}
          placeholder={
            <div role="status" {...stylex.props(styles.relationGraphFavoritesContentDiv3)}>
              <span
                {...stylex.props(
                  styles.relationGraphFavoritesContentDivItem,
                  styles.relationGraphFavoritesContentSpan,
                )}
              >
                正在加载收藏的关系图
              </span>
              <Skeleton
                xstyle={[
                  styles.relationGraphFavoritesContentDivItem,
                  styles.relationGraphFavoritesContentSkeleton,
                ]}
              />
              <Skeleton
                xstyle={[
                  styles.relationGraphFavoritesContentDivItem,
                  styles.relationGraphFavoritesContentSkeleton,
                ]}
              />
            </div>
          }
        >
          {renderRelationGraphFavoriteList(pagesQuery, favorites, startRename, setDeleteTarget)}
        </RevealItem>
      </WorkspaceShell>
      <Dialog
        open={renameTarget !== null}
        onOpenChange={(nextOpen) => {
          if (!nextOpen && !renameMut.isPending) setRenameTarget(null);
        }}
      >
        <DialogContent xstyle={styles.relationGraphFavoritesContentDialogContent}>
          <DialogHeader>
            <DialogTitle>重命名关系图</DialogTitle>
            <DialogDescription>为“{renameTarget?.title ?? ''}”输入新的名称</DialogDescription>
          </DialogHeader>
          <form
            onSubmit={(event) => {
              event.preventDefault();
              if (renameTarget && renameDraft.trim()) {
                renameMut.mutate({ id: renameTarget.id, title: renameDraft.trim() });
              }
            }}
          >
            <Input
              xstyle={styles.relationGraphFavoritesContentFormItem}
              aria-label="关系图名称"
              name="relation_graph_title"
              autoComplete="off"
              maxLength={200}
              value={renameDraft}
              onChange={(event) => setRenameDraft(event.target.value)}
            />
            {renameMut.error instanceof Error && (
              <p
                role="alert"
                {...stylex.props(
                  styles.relationGraphFavoritesContentFormItem,
                  styles.relationGraphFavoritesContentP2,
                )}
              >
                {renameMut.error.message}
              </p>
            )}
            <DialogFooter xstyle={styles.relationGraphFavoritesContentFormItem}>
              <Button type="submit" disabled={renameMut.isPending || !renameDraft.trim()}>
                {renameMut.isPending ? '保存中…' : '保存'}
              </Button>
            </DialogFooter>
          </form>
        </DialogContent>
      </Dialog>
      <ConfirmDialog
        open={deleteTarget !== null}
        onOpenChange={(nextOpen) => {
          if (!nextOpen && !deleteMut.isPending) setDeleteTarget(null);
        }}
        title="删除收藏的关系图？"
        description={`确认删除“${deleteTarget?.title ?? ''}”？删除后无法恢复收藏时的网络。`}
        actionLabel="确认删除"
        pendingLabel="删除中…"
        isPending={deleteMut.isPending}
        error={deleteMut.error instanceof Error ? deleteMut.error.message : null}
        onConfirm={() => {
          if (deleteTarget) deleteMut.mutate(deleteTarget.id);
        }}
      />
    </>
  );
}

/**
 * Render the loading, error, empty, or populated state of the saved graph list.
 *
 * @param pagesQuery - Infinite page query.
 * @param favorites - Loaded saved graphs.
 * @param onRename - Start renaming a saved graph.
 * @param onDelete - Ask to delete a saved graph.
 * @returns List body.
 */
function renderRelationGraphFavoriteList(
  pagesQuery: {
    isPending: boolean;
    isError: boolean;
    error: unknown;
    hasNextPage: boolean;
    isFetchingNextPage: boolean;
    fetchNextPage: () => unknown;
  },
  favorites: RelationGraphFavoriteSummary[],
  onRename: (favorite: RelationGraphFavoriteSummary) => void,
  onDelete: (favorite: RelationGraphFavoriteSummary) => void,
) {
  if (pagesQuery.isPending) return null;
  if (pagesQuery.isError) {
    return (
      <StateMessage
        tone="danger"
        title="无法加载收藏的关系图"
        description={pagesQuery.error instanceof Error ? pagesQuery.error.message : '未知错误'}
      />
    );
  }
  if (favorites.length === 0) {
    return (
      <StateMessage
        title="还没有收藏的关系图"
        description="在关系图页面点击右上角的收藏按钮，保存当前展开的网络。"
      />
    );
  }
  return (
    <>
      <ul aria-label="收藏的关系图">
        {favorites.map((favorite) => (
          <li
            key={favorite.id}
            {...stylex.props(styles.renderRelationGraphFavoriteListLi, stylex.defaultMarker())}
          >
            <span aria-hidden="true" {...stylex.props(styles.renderRelationGraphFavoriteListSpan)}>
              <Share2 xstyle={styles.renderRelationGraphFavoriteListShare2} />
            </span>
            <div {...stylex.props(styles.renderRelationGraphFavoriteListDiv)}>
              <Link
                to="/"
                search={buildSavedRelationSearch(favorite)}
                {...stylex.props(styles.renderRelationGraphFavoriteListLink)}
              >
                {favorite.title}
              </Link>
              <p {...stylex.props(styles.renderRelationGraphFavoriteListP)}>
                {favorite.node_count} 篇 · {favorite.db_name} · 更新于{' '}
                {formatSavedTime(favorite.updated_at)}
              </p>
            </div>
            <DropdownMenu>
              <DropdownMenuTrigger
                render={
                  <Button
                    type="button"
                    variant="ghost"
                    size="icon"
                    xstyle={styles.renderRelationGraphFavoriteListButton}
                    aria-label={`管理关系图 ${favorite.title}`}
                  />
                }
              >
                <MoreHorizontal
                  xstyle={styles.renderRelationGraphFavoriteListShare2}
                  aria-hidden="true"
                />
              </DropdownMenuTrigger>
              <DropdownMenuContent align="end">
                <DropdownMenuItem onClick={() => onRename(favorite)}>重命名</DropdownMenuItem>
                <DropdownMenuItem isDestructive onClick={() => onDelete(favorite)}>
                  删除
                </DropdownMenuItem>
              </DropdownMenuContent>
            </DropdownMenu>
          </li>
        ))}
      </ul>
      {pagesQuery.hasNextPage && (
        <div {...stylex.props(styles.renderRelationGraphFavoriteListDiv2)}>
          <Button
            type="button"
            variant="outline"
            size="sm"
            disabled={pagesQuery.isFetchingNextPage}
            onClick={() => void pagesQuery.fetchNextPage()}
          >
            {pagesQuery.isFetchingNextPage ? '加载中…' : '加载更多'}
          </Button>
        </div>
      )}
    </>
  );
}

const styles = stylex.create({
  relationGraphFavoritesContentDiv: {
    display: 'flex',
    flexDirection: 'column',
    rowGap: '0.75rem',
  },
  relationGraphFavoritesContentDiv2: {
    minWidth: '0px',
  },
  relationGraphFavoritesContentH1: {
    fontSize: '1.125rem',
    lineHeight: '1.75rem',
    fontWeight: 700,
  },
  relationGraphFavoritesContentP: {
    fontSize: '0.75rem',
    lineHeight: lineHeights.textXs,
    color: colors.mutedForeground,
  },
  relationGraphFavoritesContentDivItem: {
    marginBottom: {
      default: null,
      ':not(:last-child)': '0.75rem',
    },
  },
  relationGraphFavoritesContentDiv3: {
    paddingInline: {
      default: '1rem',
      '@media (min-width: 40rem)': '1.25rem',
    },
    paddingBlock: '1rem',
  },
  relationGraphFavoritesContentSpan: {
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
  relationGraphFavoritesContentSkeleton: {
    height: '3.5rem',
    width: '100%',
  },
  relationGraphFavoritesContentDialogContent: {
    maxWidth: {
      default: 'none',
      '@media (min-width: 48rem)': '28rem',
    },
  },
  relationGraphFavoritesContentFormItem: {
    marginBottom: {
      default: null,
      ':not(:last-child)': '1rem',
    },
  },
  relationGraphFavoritesContentP2: {
    fontSize: '0.875rem',
    lineHeight: lineHeights.textSm,
    color: colors.destructive,
  },
  renderRelationGraphFavoriteListLi: {
    position: 'relative',
    display: 'flex',
    alignItems: 'flex-start',
    gap: '0.75rem',
    borderBottomStyle: 'solid',
    borderBottomWidth: '1px',
    borderColor: `color-mix(in oklab, ${colors.border} 60%, transparent)`,
    paddingInline: {
      default: '1rem',
      '@media (min-width: 40rem)': '1.25rem',
    },
    paddingBlock: '1rem',
    transitionProperty: 'background-color',
    transitionTimingFunction: 'cubic-bezier(0.25, 0.1, 0.25, 1)',
    transitionDuration: '200ms',
    backgroundColor: {
      default: null,
      ':hover': {
        default: null,
        '@media (hover: hover)': `color-mix(in oklab, ${colors.accent} 50%, transparent)`,
      },
    },
  },
  renderRelationGraphFavoriteListSpan: {
    marginTop: '0.125rem',
    display: 'flex',
    width: '2rem',
    height: '2rem',
    flexShrink: 0,
    alignItems: 'center',
    justifyContent: 'center',
    borderRadius: '8px',
    backgroundColor: colors.accent,
    color: colors.mutedForeground,
  },
  renderRelationGraphFavoriteListShare2: {
    width: '1rem',
    height: '1rem',
  },
  renderRelationGraphFavoriteListDiv: {
    minWidth: '0px',
    flex: '1',
  },
  renderRelationGraphFavoriteListLink: {
    overflow: 'hidden',
    display: '-webkit-box',
    WebkitBoxOrient: 'vertical',
    WebkitLineClamp: 2,
    fontSize: '15px',
    lineHeight: '1.5rem',
    fontWeight: 700,
    color: colors.foreground,
    outlineStyle: 'none',
    textDecorationLine: {
      default: null,
      ':focus-visible': 'underline',
    },
    '::after': {
      content: '""',
      position: 'absolute',
      inset: '0px',
    },
  },
  renderRelationGraphFavoriteListP: {
    marginTop: '0.25rem',
    fontSize: '0.75rem',
    lineHeight: lineHeights.textXs,
    color: colors.mutedForeground,
  },
  renderRelationGraphFavoriteListButton: {
    position: 'relative',
    zIndex: 10,
    width: {
      default: '2.75rem',
      '@media (min-width: 48rem)': '2rem',
    },
    height: {
      default: '2.75rem',
      '@media (min-width: 48rem)': '2rem',
    },
    flexShrink: 0,
  },
  renderRelationGraphFavoriteListDiv2: {
    display: 'flex',
    justifyContent: 'center',
    paddingBlock: '1rem',
  },
});
