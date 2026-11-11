/**
 * Save, update, fork, reset, and remove the relation graph favorite of the current workspace,
 * flag unsaved changes, and ask before leaving a changed saved graph.
 */

import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { Bookmark, BookmarkCheck, Loader2 } from '@/components/ui/animated-icons';
import { useNavigate } from '@tanstack/react-router';
import { useEffect, useEffectEvent, useMemo, useState } from 'react';

import { Button } from '@/components/ui/button';
import { IconTooltip } from '@/components/ui/tooltip';
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
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from '@/components/ui/dropdown-menu';
import {
  ApiError,
  createRelationGraphFavorite,
  deleteRelationGraphFavorite,
  relationGraphFavoriteQuery,
  updateRelationGraphFavorite,
  type RelationGraphFavorite,
  RELATION_GRAPH_FAVORITES_QUERY_PREFIX,
} from '@/lib/api';
import {
  normalizeRelationGraph,
  relationGraphSignature,
  type RelationGraph,
} from '@/lib/relation-graph';
import * as stylex from '@stylexjs/stylex';
import { animations } from '@/lib/animations.stylex';
import { colors } from '@/lib/theme.stylex';

/** Longest saved title accepted by the server, in characters. */
const RELATION_GRAPH_TITLE_LIMIT = 200;

/**
 * Build the default saved title from the center article's title.
 *
 * @param rootTitle - Display title of the center article.
 * @returns Title within the server limit.
 */
function buildDefaultTitle(rootTitle: string): string {
  const prefix = '以「';
  const suffix = '」为中心的关系图';
  const room = RELATION_GRAPH_TITLE_LIMIT - prefix.length - suffix.length - 1;
  const characters = Array.from(rootTitle);
  const title = characters.length > room ? `${characters.slice(0, room).join('')}…` : rootTitle;
  return `${prefix}${title}${suffix}`;
}

type UseRelationGraphFavoriteOptions = {
  dbName: string;
  graph: RelationGraph;
  rootTitle: string;
  savedId: number | null;
  onError: (message: string) => void;
  onSavedIdChange: (savedId: number | null) => void;
};

/**
 * Own the saved-graph link of the workspace: its saved snapshot, whether the current graph has
 * drifted from it, and the save, update, remove, and reset actions.
 *
 * @param options - Current graph, its database and center title, the linked favorite, and callbacks.
 * @returns Favorite state and actions shared by the control and the leave guard.
 */
function useRelationGraphFavorite({
  dbName,
  graph,
  rootTitle,
  savedId,
  onError,
  onSavedIdChange,
}: UseRelationGraphFavoriteOptions) {
  const queryClient = useQueryClient();
  const [status, setStatus] = useState('');
  const savedQuery = useQuery({
    ...relationGraphFavoriteQuery(savedId ?? 0),
    enabled: savedId !== null,
    staleTime: Number.POSITIVE_INFINITY,
    retry: false,
  });
  const savedGraph = useMemo(
    () => normalizeRelationGraph(savedQuery.data?.graph),
    [savedQuery.data],
  );
  const isDirty =
    savedId !== null &&
    savedGraph !== null &&
    relationGraphSignature(savedGraph) !== relationGraphSignature(graph);
  const snapshot = {
    graph: { ...graph },
    node_count: Object.keys(graph.nodes).length,
  };

  /**
   * Cache a saved graph and refresh the favorites list.
   *
   * @param favorite - Saved graph returned by the server.
   */
  const remember = (favorite: RelationGraphFavorite) => {
    queryClient.setQueryData(relationGraphFavoriteQuery(favorite.id).queryKey, favorite);
    void queryClient.invalidateQueries({ queryKey: RELATION_GRAPH_FAVORITES_QUERY_PREFIX });
  };

  /**
   * Show a failure, unlinking the graph when its favorite no longer exists.
   *
   * @param error - Failure from a favorite request.
   */
  const reportFailure = (error: unknown) => {
    setStatus('');
    if (error instanceof ApiError && error.status === 404) {
      onSavedIdChange(null);
      onError('这张关系图的收藏已被删除，可以重新收藏。');
      return;
    }
    onError(error instanceof Error ? error.message : '收藏操作失败，请稍后重试。');
  };

  const createMut = useMutation({
    mutationFn: () =>
      createRelationGraphFavorite({
        title: buildDefaultTitle(rootTitle),
        db_name: dbName,
        root_article_id: Number(graph.rootId),
        ...snapshot,
      }),
    onSuccess: (favorite) => {
      remember(favorite);
      onSavedIdChange(favorite.id);
      setStatus('已收藏关系图。');
    },
    onError: reportFailure,
  });
  const updateMut = useMutation({
    mutationFn: (graphId: number) => updateRelationGraphFavorite(graphId, snapshot),
    onSuccess: (favorite) => {
      remember(favorite);
      setStatus('已更新收藏。');
    },
    onError: reportFailure,
  });
  const deleteMut = useMutation({
    mutationFn: (graphId: number) => deleteRelationGraphFavorite(graphId),
    onSuccess: (_data, graphId) => {
      queryClient.removeQueries({ queryKey: relationGraphFavoriteQuery(graphId).queryKey });
      void queryClient.invalidateQueries({ queryKey: RELATION_GRAPH_FAVORITES_QUERY_PREFIX });
      onSavedIdChange(null);
      setStatus('已取消收藏关系图。');
    },
    onError: reportFailure,
  });

  /**
   * Overwrite the linked favorite with the current graph.
   *
   * @returns Whether the update succeeded.
   */
  const update = async (): Promise<boolean> => {
    if (savedId === null) return false;
    try {
      await updateMut.mutateAsync(savedId);
      return true;
    } catch {
      return false;
    }
  };

  return {
    savedId,
    savedTitle: savedQuery.data?.title ?? null,
    savedGraph,
    isDirty,
    isBusy: createMut.isPending || updateMut.isPending || deleteMut.isPending,
    status,
    setStatus,
    create: () => createMut.mutate(),
    update,
    remove: () => {
      if (savedId !== null) deleteMut.mutate(savedId);
    },
    reportError: onError,
  };
}

/** Saved-graph state and actions returned by `useRelationGraphFavorite`. */
type RelationGraphFavoriteState = ReturnType<typeof useRelationGraphFavorite>;

/**
 * Render the graph favorite control and its leave guard for the current workspace graph.
 *
 * @param props - Current graph, its database and center title, the linked favorite, and callbacks.
 * @returns Favorite buttons and the unsaved-changes leave guard; failures go to `onError`.
 */
export function RelationGraphFavoriteControls({
  onReset,
  ...options
}: UseRelationGraphFavoriteOptions & { onReset: (graph: RelationGraph) => void }) {
  const favorite = useRelationGraphFavorite(options);
  return (
    <>
      <RelationGraphFavoriteButtons favorite={favorite} onReset={onReset} />
      <RelationGraphLeaveGuard favorite={favorite} />
    </>
  );
}

/**
 * Render the graph favorite buttons: a save button for unsaved graphs; for saved graphs a menu to
 * update, save as new, reset, or remove, whose trigger carries a dot once the graph differs from
 * its saved snapshot.
 *
 * @param props - Favorite state and the reset callback.
 * @returns Favorite buttons with an announced outcome message.
 */
function RelationGraphFavoriteButtons({
  favorite,
  onReset,
}: {
  favorite: RelationGraphFavoriteState;
  onReset: (graph: RelationGraph) => void;
}) {
  const { savedId, savedTitle, savedGraph, isDirty, isBusy, status } = favorite;

  /** Restore the graph exactly as it was saved. */
  const handleReset = () => {
    if (!savedGraph) {
      favorite.reportError('无法读取收藏时的关系图。');
      return;
    }
    onReset(savedGraph);
    favorite.setStatus('已重置为收藏时的状态。');
  };

  return (
    <>
      {savedId === null ? (
        <IconTooltip label="收藏关系图">
          <Button
            type="button"
            variant="ghost"
            size="icon"
            xstyle={styles.relationGraphFavoriteButtonsButton}
            aria-label="收藏关系图"
            disabled={isBusy}
            onClick={favorite.create}
          >
            {isBusy ? (
              <Loader2 xstyle={styles.relationGraphFavoriteButtonsLoader2} aria-hidden="true" />
            ) : (
              <Bookmark xstyle={styles.relationGraphFavoriteButtonsBookmark} aria-hidden="true" />
            )}
          </Button>
        </IconTooltip>
      ) : (
        <DropdownMenu>
          <IconTooltip
            label={
              savedTitle ? `已收藏：${savedTitle}${isDirty ? '（有未保存的改动）' : ''}` : '已收藏'
            }
          >
            <DropdownMenuTrigger
              render={
                <Button
                  type="button"
                  variant="ghost"
                  size="icon"
                  xstyle={styles.relationGraphFavoriteButtonsButton2}
                  aria-label={
                    isDirty ? '已收藏的关系图操作（有未保存的改动）' : '已收藏的关系图操作'
                  }
                  disabled={isBusy}
                />
              }
            >
              {isBusy ? (
                <Loader2 xstyle={styles.relationGraphFavoriteButtonsLoader2} aria-hidden="true" />
              ) : (
                <BookmarkCheck
                  xstyle={styles.relationGraphFavoriteButtonsBookmark}
                  aria-hidden="true"
                />
              )}
              {isDirty && (
                <span
                  aria-hidden="true"
                  data-testid="relation-graph-unsaved-dot"
                  {...stylex.props(styles.relationGraphFavoriteButtonsSpan)}
                />
              )}
            </DropdownMenuTrigger>
          </IconTooltip>
          <DropdownMenuContent
            align="end"
            xstyle={styles.relationGraphFavoriteButtonsDropdownMenuContent}
          >
            <DropdownMenuLabel xstyle={styles.relationGraphFavoriteButtonsDropdownMenuLabel}>
              {savedTitle ?? '已收藏的关系图'}
            </DropdownMenuLabel>
            <DropdownMenuSeparator />
            <DropdownMenuItem onClick={() => void favorite.update()}>
              {isDirty ? '保存改动到收藏' : '更新收藏'}
            </DropdownMenuItem>
            <DropdownMenuItem onClick={favorite.create}>另存为新收藏</DropdownMenuItem>
            <DropdownMenuItem disabled={!savedGraph || !isDirty} onClick={handleReset}>
              重置为收藏时的状态
            </DropdownMenuItem>
            <DropdownMenuSeparator />
            <DropdownMenuItem isDestructive onClick={favorite.remove}>
              取消收藏
            </DropdownMenuItem>
          </DropdownMenuContent>
        </DropdownMenu>
      )}
      <output
        {...stylex.props(styles.relationGraphFavoriteButtonsSpan2)}
        data-testid="relation-graph-favorite-status"
      >
        {status}
      </output>
    </>
  );
}

/**
 * Find the in-app destination of a plain left click on a link that leaves the current graph.
 *
 * @param event - Click captured on the document.
 * @returns Destination URL, or null when the click should proceed normally.
 */
function findLeavingHref(event: MouseEvent): string | null {
  if (
    event.defaultPrevented ||
    event.button !== 0 ||
    event.metaKey ||
    event.ctrlKey ||
    event.shiftKey ||
    event.altKey
  ) {
    return null;
  }
  const anchor = event.target instanceof Element ? event.target.closest('a[href]') : null;
  if (!(anchor instanceof HTMLAnchorElement) || (anchor.target && anchor.target !== '_self')) {
    return null;
  }
  const destination = new URL(anchor.href, window.location.href);
  if (destination.origin !== window.location.origin || anchor.hasAttribute('download')) {
    return null;
  }
  const current = new URL(window.location.href);
  const isSameGraph =
    destination.pathname === current.pathname &&
    destination.searchParams.get('view') === 'relations' &&
    destination.searchParams.get('article') === current.searchParams.get('article') &&
    destination.searchParams.get('db') === current.searchParams.get('db');
  return isSameGraph ? null : `${destination.pathname}${destination.search}${destination.hash}`;
}

/**
 * Ask before leaving a saved graph that has unsaved changes: in-app links open a dialog that can
 * save first, and closing or reloading the tab triggers the browser's own confirmation.
 *
 * @param props - Favorite state of the workspace.
 * @returns Leave confirmation dialog.
 */
function RelationGraphLeaveGuard({ favorite }: { favorite: RelationGraphFavoriteState }) {
  const navigate = useNavigate();
  const [pendingHref, setPendingHref] = useState<string | null>(null);
  const { isDirty, isBusy } = favorite;

  const interceptClick = useEffectEvent((event: MouseEvent) => {
    if (!isDirty) return;
    const href = findLeavingHref(event);
    if (!href) return;
    event.preventDefault();
    event.stopPropagation();
    setPendingHref(href);
  });

  useEffect(() => {
    const handleClick = (event: MouseEvent) => interceptClick(event);
    document.addEventListener('click', handleClick, true);
    return () => document.removeEventListener('click', handleClick, true);
  }, []);

  useEffect(() => {
    if (!isDirty) return;
    const handleBeforeUnload = (event: BeforeUnloadEvent) => {
      event.preventDefault();
      event.returnValue = '';
    };
    window.addEventListener('beforeunload', handleBeforeUnload);
    return () => window.removeEventListener('beforeunload', handleBeforeUnload);
  }, [isDirty]);

  /**
   * Continue to the intercepted destination.
   *
   * @param href - Destination URL.
   */
  const leave = (href: string) => {
    setPendingHref(null);
    void navigate({ href });
  };

  return (
    <Dialog
      open={pendingHref !== null}
      onOpenChange={(nextOpen) => {
        if (!nextOpen && !isBusy) setPendingHref(null);
      }}
    >
      <DialogContent xstyle={styles.relationGraphLeaveGuardDialogContent}>
        <DialogHeader>
          <DialogTitle>关系图有未保存的改动</DialogTitle>
          <DialogDescription>
            {favorite.savedTitle
              ? `“${favorite.savedTitle}”在收藏后又有改动。离开前要把改动保存到收藏吗？`
              : '这张关系图在收藏后又有改动。离开前要把改动保存到收藏吗？'}
          </DialogDescription>
        </DialogHeader>
        <DialogFooter xstyle={styles.relationGraphLeaveGuardDialogFooter}>
          <Button
            type="button"
            variant="ghost"
            disabled={isBusy}
            onClick={() => setPendingHref(null)}
          >
            继续编辑
          </Button>
          <div {...stylex.props(styles.relationGraphLeaveGuardDiv)}>
            <Button
              type="button"
              variant="outline"
              disabled={isBusy}
              onClick={() => pendingHref && leave(pendingHref)}
            >
              不保存
            </Button>
            <Button
              type="button"
              disabled={isBusy}
              onClick={() => {
                const href = pendingHref;
                void favorite.update().then((isSaved) => {
                  if (isSaved && href) leave(href);
                  else setPendingHref(null);
                });
              }}
            >
              {isBusy ? '保存中…' : '保存并离开'}
            </Button>
          </div>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

const styles = stylex.create({
  relationGraphFavoriteButtonsButton: {
    width: {
      default: '2.75rem',
      '@media (min-width: 48rem)': '2rem',
    },
    height: {
      default: '2.75rem',
      '@media (min-width: 48rem)': '2rem',
    },
  },
  relationGraphFavoriteButtonsLoader2: {
    width: '1rem',
    height: '1rem',
    animationName: animations.spin,
    animationDuration: '1s',
    animationTimingFunction: 'linear',
    animationIterationCount: 'infinite',
  },
  relationGraphFavoriteButtonsBookmark: {
    width: '1rem',
    height: '1rem',
  },
  relationGraphFavoriteButtonsButton2: {
    position: 'relative',
    width: {
      default: '2.75rem',
      '@media (min-width: 48rem)': '2rem',
    },
    height: {
      default: '2.75rem',
      '@media (min-width: 48rem)': '2rem',
    },
    color: {
      default: colors.primaryText,
      ':hover': {
        default: null,
        '@media (hover: hover)': colors.accentForeground,
      },
    },
  },
  relationGraphFavoriteButtonsSpan: {
    position: 'absolute',
    top: {
      default: '0.375rem',
      '@media (min-width: 48rem)': '0.25rem',
    },
    right: {
      default: '0.375rem',
      '@media (min-width: 48rem)': '0.25rem',
    },
    width: '0.5rem',
    height: '0.5rem',
    borderRadius: 'calc(infinity * 1px)',
    backgroundColor: colors.warningForeground,
    boxShadow: `0 0 0 2px ${colors.background}`,
  },
  relationGraphFavoriteButtonsDropdownMenuContent: {
    width: '13rem',
  },
  relationGraphFavoriteButtonsDropdownMenuLabel: {
    overflow: 'hidden',
    textOverflow: 'ellipsis',
    whiteSpace: 'nowrap',
  },
  relationGraphFavoriteButtonsSpan2: {
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
  relationGraphLeaveGuardDialogContent: {
    maxWidth: {
      default: 'none',
      '@media (min-width: 48rem)': '28rem',
    },
  },
  relationGraphLeaveGuardDialogFooter: {
    gap: '0.5rem',
    justifyContent: {
      default: null,
      '@media (min-width: 40rem)': 'space-between',
    },
  },
  relationGraphLeaveGuardDiv: {
    display: 'flex',
    flexDirection: {
      default: 'column-reverse',
      '@media (min-width: 40rem)': 'row',
    },
    gap: '0.5rem',
  },
});
