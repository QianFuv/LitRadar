'use client';

import { useState, type MouseEvent } from 'react';
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import { Star } from 'lucide-react';

import { useAuth } from '@/lib/auth-context';
import {
  checkFavorite,
  addFavorite,
  removeFavorite,
  getFolders,
  getCurrentDatabase,
  type ArticleId,
  type FavoriteCheck,
} from '@/lib/api';
import { Button } from '@/components/ui/button';
import { Popover, PopoverContent, PopoverTrigger } from '@/components/ui/popover';
import { cn } from '@/lib/utils';

export function FavoriteButton({
  articleId,
  dbName,
  initialFolderIds = [],
}: {
  articleId: ArticleId;
  dbName?: string;
  initialFolderIds?: number[];
}) {
  const { user, token } = useAuth();
  const queryClient = useQueryClient();
  const db = dbName || getCurrentDatabase();
  const [open, setOpen] = useState(false);
  const queryKey = ['fav-check', articleId, db] as const;
  const initialFolderIdsValue = Array.from(new Set(initialFolderIds)).sort((a, b) => a - b);
  const [optimisticFolderIds, setOptimisticFolderIds] = useState<number[] | null>(null);
  const cachedFolderIds =
    queryClient.getQueryData<FavoriteCheck[]>(queryKey)?.map((item) => item.folder_id) ?? null;

  const { data: checks } = useQuery({
    queryKey,
    queryFn: () => checkFavorite(token!, articleId, db),
    enabled: !!token && !!user && open,
    staleTime: 5 * 60 * 1000,
  });

  const { data: folders = [], isPending: isFoldersPending } = useQuery({
    queryKey: ['folders', user?.id],
    queryFn: () => getFolders(token!),
    enabled: !!token && !!user && open,
  });

  const addMut = useMutation({
    mutationFn: (folderId: number) => addFavorite(token!, folderId, articleId, db),
    onSuccess: (_, folderId) => {
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
      queryClient.invalidateQueries({ queryKey: ['fav-check-batch', user?.id, db] });
      queryClient.invalidateQueries({ queryKey: ['folders'] });
      queryClient.invalidateQueries({ queryKey: ['folder-articles', folderId] });
    },
  });

  const removeMut = useMutation({
    mutationFn: (folderId: number) => removeFavorite(token!, folderId, articleId, db),
    onSuccess: (_, folderId) => {
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
      queryClient.invalidateQueries({ queryKey: ['fav-check-batch', user?.id, db] });
      queryClient.invalidateQueries({ queryKey: ['folders'] });
      queryClient.invalidateQueries({ queryKey: ['folder-articles', folderId] });
    },
  });

  if (!user) return null;

  const resolvedFolderIds =
    checks?.map((item) => item.folder_id) ??
    optimisticFolderIds ??
    cachedFolderIds ??
    initialFolderIdsValue;
  const isFav = resolvedFolderIds.length > 0;
  const favFolderIds = new Set(resolvedFolderIds);

  return (
    <Popover open={open} onOpenChange={setOpen}>
      <PopoverTrigger asChild>
        <Button
          variant="outline"
          size="sm"
          className={cn(isFav && 'text-yellow-500 border-yellow-500/50')}
          onClick={(event: MouseEvent<HTMLButtonElement>) => event.stopPropagation()}
        >
          <Star className={cn('h-4 w-4 mr-1', isFav && 'fill-yellow-500')} />
          {isFav ? '已收藏' : '收藏'}
        </Button>
      </PopoverTrigger>
      <PopoverContent
        className="w-56 p-2"
        align="start"
        onClick={(event: MouseEvent<HTMLDivElement>) => event.stopPropagation()}
      >
        <div className="space-y-1">
          <div className="px-2 py-1 text-xs text-muted-foreground font-medium">选择收藏夹</div>
          {isFoldersPending ? (
            <div role="status" className="px-2 py-2 text-xs text-muted-foreground">
              加载中...
            </div>
          ) : folders.length === 0 ? (
            <div className="px-2 py-2 text-xs text-muted-foreground">
              暂无收藏夹，请先在「我的收藏」中创建
            </div>
          ) : (
            folders.map((folder) => {
              const isInFolder = favFolderIds.has(folder.id);
              return (
                <button
                  key={folder.id}
                  className={cn(
                    'w-full flex items-center gap-2 rounded-md px-2 py-1.5 text-sm transition-colors',
                    isInFolder
                      ? 'bg-yellow-500/10 text-yellow-600 dark:text-yellow-400'
                      : 'hover:bg-accent',
                  )}
                  onClick={() => {
                    if (isInFolder) {
                      removeMut.mutate(folder.id);
                    } else {
                      addMut.mutate(folder.id);
                    }
                  }}
                >
                  <Star
                    className={cn('h-3.5 w-3.5', isInFolder && 'fill-yellow-500 text-yellow-500')}
                  />
                  <span className="truncate">{folder.name}</span>
                </button>
              );
            })
          )}
        </div>
      </PopoverContent>
    </Popover>
  );
}
