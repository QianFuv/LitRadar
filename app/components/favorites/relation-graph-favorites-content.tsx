'use client';

/**
 * Saved relation graph list of the favorites page: open, rename, and delete saved graphs.
 */

import { useInfiniteQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import { MoreHorizontal, Share2 } from '@/components/ui/animated-icons';
import Link from 'next/link';
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
import { buildSavedRelationHref } from '@/lib/relation-graph';

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
          <div className="space-y-3">
            <div className="min-w-0">
              <h1 className="text-lg leading-7 font-bold">我的收藏</h1>
              <p className="text-xs text-muted-foreground">保存的关系图会记住展开的分支和位置</p>
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
            <div role="status" className="space-y-3 px-4 py-4 sm:px-5">
              <span className="sr-only">正在加载收藏的关系图</span>
              <Skeleton className="h-14 w-full" />
              <Skeleton className="h-14 w-full" />
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
        <DialogContent className="md:max-w-md">
          <DialogHeader>
            <DialogTitle>重命名关系图</DialogTitle>
            <DialogDescription>为“{renameTarget?.title ?? ''}”输入新的名称</DialogDescription>
          </DialogHeader>
          <form
            className="space-y-4"
            onSubmit={(event) => {
              event.preventDefault();
              if (renameTarget && renameDraft.trim()) {
                renameMut.mutate({ id: renameTarget.id, title: renameDraft.trim() });
              }
            }}
          >
            <Input
              aria-label="关系图名称"
              name="relation_graph_title"
              autoComplete="off"
              maxLength={200}
              value={renameDraft}
              onChange={(event) => setRenameDraft(event.target.value)}
            />
            {renameMut.error instanceof Error && (
              <p role="alert" className="text-sm text-destructive">
                {renameMut.error.message}
              </p>
            )}
            <DialogFooter>
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
            className="motion-control group/graph relative flex items-start gap-3 border-b border-border/60 px-4 py-4 transition-[background-color] hover:bg-accent/50 sm:px-5"
          >
            <span
              aria-hidden="true"
              className="mt-0.5 flex size-8 shrink-0 items-center justify-center rounded-lg bg-accent text-muted-foreground"
            >
              <Share2 className="size-4" />
            </span>
            <div className="min-w-0 flex-1">
              <Link
                href={buildSavedRelationHref(favorite)}
                prefetch={false}
                className="line-clamp-2 text-[15px] leading-6 font-bold text-foreground outline-none after:absolute after:inset-0 focus-visible:underline"
              >
                {favorite.title}
              </Link>
              <p className="mt-1 text-xs text-muted-foreground">
                {favorite.node_count} 篇 · {favorite.db_name} · 更新于{' '}
                {formatSavedTime(favorite.updated_at)}
              </p>
            </div>
            <DropdownMenu>
              <DropdownMenuTrigger asChild>
                <Button
                  type="button"
                  variant="ghost"
                  size="icon"
                  className="relative z-10 size-11 shrink-0 md:size-8"
                  aria-label={`管理关系图 ${favorite.title}`}
                >
                  <MoreHorizontal className="size-4" aria-hidden="true" />
                </Button>
              </DropdownMenuTrigger>
              <DropdownMenuContent align="end">
                <DropdownMenuItem onSelect={() => onRename(favorite)}>重命名</DropdownMenuItem>
                <DropdownMenuItem isDestructive onSelect={() => onDelete(favorite)}>
                  删除
                </DropdownMenuItem>
              </DropdownMenuContent>
            </DropdownMenu>
          </li>
        ))}
      </ul>
      {pagesQuery.hasNextPage && (
        <div className="flex justify-center py-4">
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
