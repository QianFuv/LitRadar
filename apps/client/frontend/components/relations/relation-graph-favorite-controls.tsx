/**
 * Save, update, fork, reset, and remove the relation graph favorite of the current workspace,
 * flag unsaved changes, and ask before leaving a changed saved graph.
 */

import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { Loader2 } from 'lucide-react';
import { Bookmark, BookmarkCheck } from '@/components/ui/animated-icons';
import { useNavigate } from 'react-router';
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
  getRelationGraphFavorite,
  updateRelationGraphFavorite,
  type RelationGraphFavorite,
} from '@/lib/api';
import {
  normalizeRelationGraph,
  relationGraphSignature,
  type RelationGraph,
} from '@/lib/relation-graph';

/** Longest saved title accepted by the server, in characters. */
const RELATION_GRAPH_TITLE_LIMIT = 200;

/** Query key prefix of saved relation graph pages. */
export const RELATION_GRAPH_FAVORITES_KEY = ['relation-graph-favorites'] as const;

/**
 * Query key of one saved relation graph.
 *
 * @param graphId - Saved graph id.
 * @returns Stable query key.
 */
export function relationGraphFavoriteKey(graphId: number | null) {
  return ['relation-graph-favorite', graphId] as const;
}

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
    queryKey: relationGraphFavoriteKey(savedId),
    queryFn: ({ signal }) => getRelationGraphFavorite(savedId as number, signal),
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
    queryClient.setQueryData(relationGraphFavoriteKey(favorite.id), favorite);
    void queryClient.invalidateQueries({ queryKey: RELATION_GRAPH_FAVORITES_KEY });
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
      queryClient.removeQueries({ queryKey: relationGraphFavoriteKey(graphId) });
      void queryClient.invalidateQueries({ queryKey: RELATION_GRAPH_FAVORITES_KEY });
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
            className="size-11 md:size-8"
            aria-label="收藏关系图"
            disabled={isBusy}
            onClick={favorite.create}
          >
            {isBusy ? (
              <Loader2 className="size-4 animate-spin" aria-hidden="true" />
            ) : (
              <Bookmark className="size-4" aria-hidden="true" />
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
            <DropdownMenuTrigger asChild>
              <Button
                type="button"
                variant="ghost"
                size="icon"
                className="relative size-11 text-primary-text md:size-8"
                aria-label={isDirty ? '已收藏的关系图操作（有未保存的改动）' : '已收藏的关系图操作'}
                disabled={isBusy}
              >
                {isBusy ? (
                  <Loader2 className="size-4 animate-spin" aria-hidden="true" />
                ) : (
                  <BookmarkCheck className="size-4" aria-hidden="true" />
                )}
                {isDirty && (
                  <span
                    aria-hidden="true"
                    data-testid="relation-graph-unsaved-dot"
                    className="absolute top-1.5 right-1.5 size-2 rounded-full bg-warning-foreground ring-2 ring-background md:top-1 md:right-1"
                  />
                )}
              </Button>
            </DropdownMenuTrigger>
          </IconTooltip>
          <DropdownMenuContent align="end" className="w-52">
            <DropdownMenuLabel className="truncate">
              {savedTitle ?? '已收藏的关系图'}
            </DropdownMenuLabel>
            <DropdownMenuSeparator />
            <DropdownMenuItem onSelect={() => void favorite.update()}>
              {isDirty ? '保存改动到收藏' : '更新收藏'}
            </DropdownMenuItem>
            <DropdownMenuItem onSelect={favorite.create}>另存为新收藏</DropdownMenuItem>
            <DropdownMenuItem disabled={!savedGraph || !isDirty} onSelect={handleReset}>
              重置为收藏时的状态
            </DropdownMenuItem>
            <DropdownMenuSeparator />
            <DropdownMenuItem isDestructive onSelect={favorite.remove}>
              取消收藏
            </DropdownMenuItem>
          </DropdownMenuContent>
        </DropdownMenu>
      )}
      <span role="status" className="sr-only" data-testid="relation-graph-favorite-status">
        {status}
      </span>
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
    navigate(href);
  };

  return (
    <Dialog
      open={pendingHref !== null}
      onOpenChange={(nextOpen) => {
        if (!nextOpen && !isBusy) setPendingHref(null);
      }}
    >
      <DialogContent className="md:max-w-md">
        <DialogHeader>
          <DialogTitle>关系图有未保存的改动</DialogTitle>
          <DialogDescription>
            {favorite.savedTitle
              ? `“${favorite.savedTitle}”在收藏后又有改动。离开前要把改动保存到收藏吗？`
              : '这张关系图在收藏后又有改动。离开前要把改动保存到收藏吗？'}
          </DialogDescription>
        </DialogHeader>
        <DialogFooter className="gap-2 sm:justify-between">
          <Button
            type="button"
            variant="ghost"
            disabled={isBusy}
            onClick={() => setPendingHref(null)}
          >
            继续编辑
          </Button>
          <div className="flex flex-col-reverse gap-2 sm:flex-row">
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
