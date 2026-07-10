'use client';

import { useEffect, useRef, useState } from 'react';
import { useInfiniteQuery, useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { parseAsInteger, useQueryState } from 'nuqs';

import {
  bulkMoveFavorites,
  bulkRemoveFavorites,
  createFolder,
  deleteFolder,
  getFolderArticles,
  getFolders,
  removeFavorite,
  renameFolder,
  setTrackingFolder,
  type ArticleId,
  type CitationFormat,
  type FavoriteArticleItem,
  type FavoriteArticleRef,
  type FavoriteItem,
} from '@/lib/api';
import { useVisiblePageList } from '@/components/feature/use-visible-page-list';

export function getFavoriteSelectionKey(
  folderId: number,
  articleId: ArticleId,
  dbName: string,
): string {
  return `${folderId}:${articleId}:${dbName}`;
}

function toFavoriteArticleRef(favorite: FavoriteArticleItem): FavoriteArticleRef {
  return {
    article_id: favorite.article_id,
    db_name: favorite.db_name,
  };
}

/**
 * Own favorite-folder URL state, infinite pages, selection, and mutations.
 *
 * @param userId - Authenticated user identifier used by folder query keys.
 * @returns Favorites page view model and actions.
 */
export function useFavoritesPage(userId: number) {
  const queryClient = useQueryClient();
  const [selectedFolderId, setSelectedFolderId] = useQueryState('folder', parseAsInteger);
  const [newFolderName, setNewFolderName] = useState('');
  const [editingId, setEditingId] = useState<number | null>(null);
  const [editName, setEditName] = useState('');
  const [dialogOpen, setDialogOpen] = useState(false);
  const [exportFormat, setExportFormat] = useState<CitationFormat>('bibtex');
  const [selectedArticleKeys, setSelectedArticleKeys] = useState<string[]>([]);
  const [moveTargetFolderId, setMoveTargetFolderId] = useState<string>('');
  const [batchFeedback, setBatchFeedback] = useState<{
    tone: 'success' | 'error';
    message: string;
  } | null>(null);
  const editInputRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    if (editingId === null || typeof window === 'undefined') {
      return;
    }
    if (window.matchMedia('(pointer: fine)').matches) {
      editInputRef.current?.focus();
    }
  }, [editingId]);

  const { data: folders = [], isLoading } = useQuery({
    queryKey: ['folders', userId],
    queryFn: () => getFolders(),
    enabled: true,
  });
  const activeFolderId =
    selectedFolderId !== null && folders.some((folder) => folder.id === selectedFolderId)
      ? selectedFolderId
      : (folders.find((folder) => folder.is_tracking)?.id ?? folders[0]?.id ?? null);
  const selectedFolder = folders.find((folder) => folder.id === activeFolderId) || null;

  const PAGE_SIZE = 50;

  const {
    data: favPages,
    fetchNextPage,
    hasNextPage,
    isFetchingNextPage,
    isPending: isPendingFavorites,
    isError: isFavoritesError,
    error: favoritesError,
    refetch: refetchFavorites,
  } = useInfiniteQuery({
    queryKey: ['folder-articles', activeFolderId],
    queryFn: ({ pageParam = 0 }) => getFolderArticles(activeFolderId!, PAGE_SIZE, pageParam),
    getNextPageParam: (lastPage, allPages) =>
      lastPage.length === PAGE_SIZE ? allPages.flat().length : undefined,
    initialPageParam: 0,
    enabled: true && !!activeFolderId && !!selectedFolder,
  });

  const favoritePages = favPages?.pages ?? [];
  const loadedPages = favoritePages.length;
  const listKey = String(activeFolderId ?? 'none');
  const { visiblePages, prefetchRef, loadMoreRef } = useVisiblePageList({
    listKey,
    loadedPages,
    hasNextPage,
    isFetchingNextPage,
    onFetchNextPage: () => void fetchNextPage(),
  });
  const visiblePageCount = Math.min(visiblePages, loadedPages);
  const favorites = favoritePages.slice(0, visiblePageCount).flat();
  const prefetchIndex = Math.max(0, favorites.length - 25);
  const selectedKeySet = new Set(selectedArticleKeys);
  const selectedFavorites = favorites.filter((favorite) =>
    selectedKeySet.has(
      getFavoriteSelectionKey(favorite.folder_id, favorite.article_id, favorite.db_name),
    ),
  );
  const allLoadedSelected = favorites.length > 0 && selectedFavorites.length === favorites.length;
  const moveTargetFolders = folders.filter((folder) => folder.id !== activeFolderId);
  const effectiveMoveTargetFolderId = moveTargetFolders.some(
    (folder) => String(folder.id) === moveTargetFolderId,
  )
    ? moveTargetFolderId
    : '';

  const createMut = useMutation({
    mutationFn: (name: string) => createFolder(name),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['folders'] });
      setNewFolderName('');
      setDialogOpen(false);
    },
  });

  const deleteMut = useMutation({
    mutationFn: (id: number) => deleteFolder(id),
    onSuccess: (_data, deletedFolderId) => {
      setSelectedArticleKeys([]);
      setMoveTargetFolderId('');
      setBatchFeedback(null);
      queryClient.invalidateQueries({ queryKey: ['folders'] });
      if (selectedFolderId === deletedFolderId) {
        void setSelectedFolderId(null);
      }
    },
  });

  const renameMut = useMutation({
    mutationFn: ({ id, name }: { id: number; name: string }) => renameFolder(id, name),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['folders'] });
      setEditingId(null);
    },
  });

  const trackMut = useMutation({
    mutationFn: (folderId: number) => setTrackingFolder(folderId),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ['folders'] }),
  });

  const removeMut = useMutation({
    mutationFn: (item: FavoriteItem) =>
      removeFavorite(item.folder_id, item.article_id, item.db_name),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['folder-articles'] });
      queryClient.invalidateQueries({ queryKey: ['folders'] });
    },
  });

  const bulkRemoveMut = useMutation({
    mutationFn: (articles: FavoriteArticleRef[]) => bulkRemoveFavorites(activeFolderId!, articles),
    onSuccess: (count) => {
      setSelectedArticleKeys([]);
      setBatchFeedback({
        tone: 'success',
        message: `已从当前收藏夹移除 ${count} 篇文章。`,
      });
      queryClient.invalidateQueries({ queryKey: ['folder-articles'] });
      queryClient.invalidateQueries({ queryKey: ['folders'] });
    },
    onError: (error) => {
      setBatchFeedback({
        tone: 'error',
        message: error instanceof Error ? error.message : '批量移除收藏失败',
      });
    },
  });

  const bulkMoveMut = useMutation({
    mutationFn: ({
      targetFolderId,
      articles,
    }: {
      targetFolderId: number;
      articles: FavoriteArticleRef[];
    }) => bulkMoveFavorites(activeFolderId!, targetFolderId, articles),
    onSuccess: (count) => {
      const targetFolderName = moveTargetFolders.find(
        (folder) => folder.id === Number(moveTargetFolderId),
      )?.name;
      setSelectedArticleKeys([]);
      setMoveTargetFolderId('');
      setBatchFeedback({
        tone: 'success',
        message: targetFolderName
          ? `已将 ${count} 篇文章移动到“${targetFolderName}”。`
          : `已移动 ${count} 篇文章。`,
      });
      queryClient.invalidateQueries({ queryKey: ['folder-articles'] });
      queryClient.invalidateQueries({ queryKey: ['folders'] });
    },
    onError: (error) => {
      setBatchFeedback({
        tone: 'error',
        message: error instanceof Error ? error.message : '批量移动收藏失败',
      });
    },
  });

  const toggleFavoriteSelection = (favorite: FavoriteArticleItem, checked: boolean) => {
    const key = getFavoriteSelectionKey(favorite.folder_id, favorite.article_id, favorite.db_name);
    setSelectedArticleKeys((previous) => {
      if (checked) {
        return previous.includes(key) ? previous : [...previous, key];
      }
      return previous.filter((item) => item !== key);
    });
    setBatchFeedback(null);
  };

  const handleSelectAllLoaded = (checked: boolean) => {
    setSelectedArticleKeys(
      checked
        ? favorites.map((favorite) =>
            getFavoriteSelectionKey(favorite.folder_id, favorite.article_id, favorite.db_name),
          )
        : [],
    );
    setBatchFeedback(null);
  };

  const handleSelectFolder = (folderId: number) => {
    void setSelectedFolderId(folderId);
    setSelectedArticleKeys([]);
    setMoveTargetFolderId('');
    setBatchFeedback(null);
  };

  const handleBulkRemove = () => {
    if (selectedFavorites.length === 0) {
      return;
    }
    if (!window.confirm(`确认从当前收藏夹移除 ${selectedFavorites.length} 篇文章？`)) {
      return;
    }
    bulkRemoveMut.mutate(selectedFavorites.map(toFavoriteArticleRef));
  };

  const handleBulkMove = () => {
    const targetFolderId = Number(effectiveMoveTargetFolderId);
    if (
      selectedFavorites.length === 0 ||
      !Number.isInteger(targetFolderId) ||
      targetFolderId <= 0
    ) {
      return;
    }
    bulkMoveMut.mutate({
      targetFolderId,
      articles: selectedFavorites.map(toFavoriteArticleRef),
    });
  };

  return {
    activeFolderId,
    allLoadedSelected,
    batchFeedback,
    bulkMoveMut,
    bulkRemoveMut,
    createMut,
    deleteMut,
    dialogOpen,
    editInputRef,
    editName,
    editingId,
    effectiveMoveTargetFolderId,
    exportFormat,
    favorites,
    favoritesError,
    folders,
    handleBulkMove,
    handleBulkRemove,
    handleSelectAllLoaded,
    handleSelectFolder,
    hasNextPage,
    isFavoritesError,
    isFetchingNextPage,
    isLoading,
    isPendingFavorites,
    loadMoreRef,
    loadedPages,
    moveTargetFolders,
    newFolderName,
    prefetchIndex,
    prefetchRef,
    refetchFavorites,
    removeMut,
    renameMut,
    selectedFavorites,
    selectedFolder,
    selectedKeySet,
    setDialogOpen,
    setEditName,
    setEditingId,
    setExportFormat,
    setMoveTargetFolderId,
    setNewFolderName,
    toggleFavoriteSelection,
    trackMut,
    visiblePageCount,
  };
}
