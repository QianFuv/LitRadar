'use client';

/**
 * Stable, accessible per-folder favorite controls and removal confirmation.
 */

import { useRef, useState, type MouseEvent } from 'react';
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import { Star } from 'lucide-react';

import { useAuth } from '@/lib/auth-context';
import {
  checkFavorite,
  addFavorite,
  removeFavorite,
  getFolders,
  type ArticleId,
  type FavoriteCheck,
  type Folder,
} from '@/lib/api';
import { Button } from '@/components/ui/button';
import { ConfirmDialog } from '@/components/ui/confirm-dialog';
import { Popover, PopoverContent, PopoverTrigger } from '@/components/ui/popover';
import { readFreshFavoriteCheck } from '@/components/feature/use-favorite-checks';
import { cn } from '@/lib/utils';

/**
 * Render per-folder favorite membership controls for an authenticated user.
 *
 * @param props - Article identity, source database, and optional cached folder ids.
 * @returns Favorite popover and destructive removal confirmation.
 */
export function FavoriteButton(props: FavoriteButtonProps) {
  const queryState = useFavoriteButtonState(props);
  if (!queryState.user) return null;
  const state = { ...queryState, ...getFavoriteButtonPresentation(queryState) };
  return renderFavoriteButton(state);
}

type FavoriteButtonProps = {
  articleId: ArticleId;
  dbName: string;
  initialFolderIds?: number[];
  isFavoriteStateUnavailable?: boolean;
};
/** Own the unchanged cache, optimistic state, queries, mutations and confirmation ref. */
function useFavoriteButtonState({
  articleId,
  dbName,
  initialFolderIds = [],
  isFavoriteStateUnavailable = false,
}: FavoriteButtonProps) {
  const { user } = useAuth();
  const queryClient = useQueryClient();
  const db = dbName;
  const [open, setOpen] = useState(false);
  const triggerRef = useRef<HTMLButtonElement>(null);
  const [folderToRemove, setFolderToRemove] = useState<Folder | null>(null);
  const queryKey = ['fav-check', user?.id, db, articleId] as const;
  const initialFolderIdsValue = Array.from(new Set(initialFolderIds)).sort((a, b) => a - b);
  const [optimisticFolderIds, setOptimisticFolderIds] = useState<number[] | null>(null);
  const cachedFolderIds =
    queryClient.getQueryData<FavoriteCheck[]>(queryKey)?.map((item) => item.folder_id) ?? null;

  const {
    data: checks,
    error: checksError,
    isFetching: isCheckingFavorite,
    refetch: refetchChecks,
  } = useQuery({
    queryKey,
    queryFn: () => checkFavorite(articleId, db),
    enabled: !!user && open,
    staleTime: 5 * 60 * 1000,
  });

  const {
    data: folders = [],
    isPending: isFoldersPending,
    error: foldersError,
    refetch: refetchFolders,
  } = useQuery({
    queryKey: ['folders', user?.id],
    queryFn: () => getFolders(),
    enabled: !!user && open,
  });

  const addMut = useMutation({
    mutationFn: (folderId: number) => addFavorite(folderId, articleId, db),
    onSuccess: async (_, folderId) => {
      const batchKey = ['fav-check-batch', user?.id, db];
      await Promise.all([
        queryClient.cancelQueries({ queryKey, exact: true }),
        queryClient.cancelQueries({ queryKey: batchKey }),
      ]);
      const folderName = folders.find((folder) => folder.id === folderId)?.name ?? '';
      setOptimisticFolderIds((current) => {
        const baseFolderIds =
          current ??
          checks?.map((item) => item.folder_id) ??
          cachedFolderIds ??
          initialFolderIdsValue;
        return baseFolderIds.includes(folderId) ? baseFolderIds : [...baseFolderIds, folderId];
      });
      queryClient.setQueryData(queryKey, (current: FavoriteCheck[] = []) => {
        if (current.some((item) => item.folder_id === folderId)) {
          return current;
        }
        return [...current, { folder_id: folderId, folder_name: folderName }];
      });
      queryClient.removeQueries({ queryKey: batchKey, type: 'inactive' });
      await queryClient.invalidateQueries({ queryKey: batchKey });
      queryClient.invalidateQueries({ queryKey: ['folders'] });
      queryClient.invalidateQueries({ queryKey: ['folder-articles', folderId] });
    },
  });

  const removeMut = useMutation({
    mutationFn: (folderId: number) => removeFavorite(folderId, articleId, db),
    onSuccess: async (_, folderId) => {
      const batchKey = ['fav-check-batch', user?.id, db];
      await Promise.all([
        queryClient.cancelQueries({ queryKey, exact: true }),
        queryClient.cancelQueries({ queryKey: batchKey }),
      ]);
      setOptimisticFolderIds((current) => {
        const baseFolderIds =
          current ??
          checks?.map((item) => item.folder_id) ??
          cachedFolderIds ??
          initialFolderIdsValue;
        return baseFolderIds.filter((id) => id !== folderId);
      });
      queryClient.setQueryData(queryKey, (current: FavoriteCheck[] = []) =>
        current.filter((item) => item.folder_id !== folderId),
      );
      queryClient.removeQueries({ queryKey: batchKey, type: 'inactive' });
      await queryClient.invalidateQueries({ queryKey: batchKey });
      queryClient.invalidateQueries({ queryKey: ['folders'] });
      queryClient.invalidateQueries({ queryKey: ['folder-articles', folderId] });
      setFolderToRemove((current) => (current?.id === folderId ? null : current));
    },
  });

  return {
    user,
    queryClient,
    open,
    setOpen,
    triggerRef,
    folderToRemove,
    setFolderToRemove,
    queryKey,
    initialFolderIdsValue,
    optimisticFolderIds,
    cachedFolderIds,
    checks,
    checksError,
    isCheckingFavorite,
    refetchChecks,
    folders,
    isFoldersPending,
    foldersError,
    refetchFolders,
    addMut,
    removeMut,
    isFavoriteStateUnavailable,
  };
}

type FavoriteButtonQueryState = ReturnType<typeof useFavoriteButtonState>;
type FavoriteButtonViewState = FavoriteButtonQueryState &
  ReturnType<typeof getFavoriteButtonPresentation>;
/** Derive authenticated membership presentation using the original fallback and unknown-state precedence. */
function getFavoriteButtonPresentation(state: FavoriteButtonQueryState) {
  const {
    queryClient,
    queryKey,
    initialFolderIdsValue,
    optimisticFolderIds,
    cachedFolderIds,
    checks,
    checksError,
    foldersError,
    isFavoriteStateUnavailable,
  } = state;
  const resolvedFolderIds = getResolvedFavoriteFolderIds(
    checks,
    optimisticFolderIds,
    cachedFolderIds,
    initialFolderIdsValue,
  );
  const isFavoriteUnknown =
    Boolean(checksError) ||
    (isFavoriteStateUnavailable && readFreshFavoriteCheck(queryClient, queryKey) === undefined);
  const isFav = !isFavoriteUnknown && resolvedFolderIds.length > 0;
  const favoriteLabel = isFavoriteUnknown ? '收藏状态未知' : isFav ? '已收藏' : '收藏';
  const lookupError = checksError ?? foldersError;
  const favFolderIds = new Set(resolvedFolderIds);

  return { isFavoriteUnknown, isFav, favoriteLabel, lookupError, favFolderIds };
}
/** Render the unchanged trigger, popover and captured confirmation as their original siblings. */
function renderFavoriteButton(state: FavoriteButtonViewState) {
  const {
    open,
    setOpen,
    triggerRef,
    folderToRemove,
    setFolderToRemove,
    removeMut,
    isFavoriteUnknown,
    isFav,
    favoriteLabel,
  } = state;
  return (
    <>
      <Popover open={open} onOpenChange={setOpen}>
        <PopoverTrigger asChild>
          <Button
            ref={triggerRef}
            variant="outline"
            size="sm"
            static
            className={cn(
              'size-11 p-0 md:h-10 md:w-auto md:px-3',
              isFav && 'text-warning-foreground hover:text-warning-foreground',
            )}
            aria-label={favoriteLabel}
            title={favoriteLabel}
            onClick={(event: MouseEvent<HTMLButtonElement>) => event.stopPropagation()}
          >
            <span
              data-favorite-state={isFavoriteUnknown ? 'unknown' : isFav ? 'active' : 'inactive'}
              className="flex items-center gap-1"
              aria-hidden="true"
            >
              <Star
                className={cn('motion-control size-4 transition-[fill]', isFav && 'fill-current')}
              />
              <span className="hidden md:grid">
                <span className="invisible col-start-1 row-start-1">已收藏</span>
                <span className="col-start-1 row-start-1">{favoriteLabel}</span>
              </span>
            </span>
          </Button>
        </PopoverTrigger>
        <PopoverContent
          className="w-56 rounded-xl border-0 p-2 shadow-vercel-card"
          align="start"
          onClick={(event: MouseEvent<HTMLDivElement>) => event.stopPropagation()}
        >
          <div className="space-y-1">
            <div className="px-2 py-1 text-xs text-muted-foreground font-medium">选择收藏夹</div>
            {renderFavoriteLookup(state)}
          </div>
        </PopoverContent>
      </Popover>
      <ConfirmDialog
        open={folderToRemove !== null}
        onOpenChange={(nextOpen) => {
          if (!nextOpen && !removeMut.isPending) {
            setFolderToRemove(null);
          }
        }}
        title="移除收藏？"
        description={`确认从“${folderToRemove?.name ?? ''}”移除收藏？`}
        actionLabel="确认移除"
        pendingLabel="移除中…"
        isPending={removeMut.isPending}
        error={removeMut.error instanceof Error ? removeMut.error.message : null}
        focusReturnRef={triggerRef}
        onConfirm={() => {
          if (folderToRemove) {
            removeMut.mutate(folderToRemove.id);
          }
        }}
      />
    </>
  );
}
/** Retain each folder button, pending guards and reset-before-target event order. */
function renderFavoriteFolderButton(state: FavoriteButtonViewState, folder: Folder) {
  const {
    setFolderToRemove,
    isCheckingFavorite,
    addMut,
    removeMut,
    isFavoriteUnknown,
    favFolderIds,
  } = state;

  const isInFolder = favFolderIds.has(folder.id);
  return (
    <button
      key={folder.id}
      type="button"
      aria-pressed={isInFolder}
      className={cn(
        'motion-control flex min-h-11 w-full items-center gap-2 rounded-sm px-2 py-1.5 text-sm outline-none transition-[background-color,color,box-shadow] focus-visible:ring-[3px] focus-visible:ring-ring/50 disabled:pointer-events-none disabled:opacity-50 md:min-h-10',
        isInFolder
          ? 'bg-warning text-warning-foreground hover:bg-warning/70 active:bg-warning/50'
          : 'hover:bg-accent active:bg-accent-pressed',
      )}
      disabled={addMut.isPending || removeMut.isPending || isCheckingFavorite || isFavoriteUnknown}
      onClick={() => {
        if (isInFolder) {
          removeMut.reset();
          setFolderToRemove(folder);
        } else {
          addMut.mutate(folder.id);
        }
      }}
    >
      <Star
        className={cn(
          'motion-control size-4 shrink-0 transition-[fill]',
          isInFolder && 'fill-current',
        )}
        strokeWidth={1.5}
        aria-hidden="true"
      />
      <span className="truncate">{folder.name}</span>
    </button>
  );
}

/** Retain error, loading, empty-folder and available-membership presentation priority. */
function renderFavoriteLookup(state: FavoriteButtonViewState) {
  const {
    checks,
    isCheckingFavorite,
    refetchChecks,
    folders,
    isFoldersPending,
    refetchFolders,
    lookupError,
  } = state;
  if (lookupError)
    return (
      <div role="alert" className="space-y-2 px-2 py-2 text-xs text-destructive">
        <p>{lookupError instanceof Error ? lookupError.message : '无法读取收藏状态'}</p>
        <Button
          type="button"
          variant="outline"
          size="sm"
          onClick={() => {
            void Promise.all([refetchChecks(), refetchFolders()]);
          }}
        >
          重试收藏状态
        </Button>
      </div>
    );
  if (isFoldersPending || (isCheckingFavorite && checks === undefined))
    return (
      <div role="status" className="px-2 py-2 text-xs text-muted-foreground">
        加载中…
      </div>
    );
  if (folders.length === 0)
    return (
      <div className="px-2 py-2 text-xs text-muted-foreground">
        暂无收藏夹，请先在「我的收藏」中创建
      </div>
    );
  return folders.map((folder) => renderFavoriteFolderButton(state, folder));
}

/** Resolve visible membership from checks before optimistic, cached and initial values. */
function getResolvedFavoriteFolderIds(
  checks: FavoriteCheck[] | undefined,
  optimistic: number[] | null,
  cached: number[] | null,
  initial: number[],
): number[] {
  return checks?.map((item) => item.folder_id) ?? optimistic ?? cached ?? initial;
}
