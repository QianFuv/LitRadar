import { useEffect, useMemo, useRef, useState } from 'react';
import { useInfiniteQuery, useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { useSearch } from '@tanstack/react-router';

import {
  bulkMoveFavorites,
  bulkRemoveFavorites,
  createFolder,
  deleteFolder,
  downloadFavoriteExport,
  folderArticlePagesQuery,
  foldersQuery,
  renameFolder,
  setTrackingFolder,
  type ArticleId,
  type CitationFormat,
  type FavoriteArticleItem,
  type FavoriteArticleRef,
  type Folder,
  FOLDER_ARTICLES_QUERY_PREFIX,
} from '@/lib/api';
import { useVisiblePageList } from '@/components/feature/use-visible-page-list';
import { invalidateFavoriteMemberships } from '@/components/feature/use-favorite-checks';
import { CLIENT_PLATFORM } from '@/lib/platform';
import { useUpdateWorkspaceSearch, WORKSPACE_ROUTE_ID } from '@/lib/search-params';

/**
 * Build a stable identity for one favorite row in selection state.
 *
 * @param folderId - Owning folder identifier.
 * @param articleId - Article identifier.
 * @param dbName - Source database name.
 * @returns Stable selection key.
 */
export function getFavoriteSelectionKey(
  folderId: number,
  articleId: ArticleId,
  dbName: string,
): string {
  return `${folderId}:${articleId}:${dbName}`;
}

/**
 * Reduce a loaded favorite to the identity required by bulk mutation endpoints.
 *
 * @param favorite - Loaded favorite article.
 * @returns Bulk mutation article reference.
 */
function toFavoriteArticleRef(favorite: FavoriteArticleItem): FavoriteArticleRef {
  return {
    article_id: favorite.article_id,
    db_name: favorite.db_name,
  };
}

/**
 * Own favorite-folder URL state, infinite pages, selection, and mutations.
 *
 * @returns Favorites page view model and actions.
 */
export function useFavoritesPage() {
  const queryClient = useQueryClient();
  const selectedFolderId = useSearch({
    from: WORKSPACE_ROUTE_ID,
    select: (search) => search.folder ?? null,
  });
  const updateSearch = useUpdateWorkspaceSearch();

  /**
   * Select a favorite folder in the URL, or return to every favorite.
   *
   * @param folderId - Folder to show, or null for all favorites.
   */
  const setSelectedFolderId = (folderId: number | null) => {
    void updateSearch((search) => ({ ...search, folder: folderId ?? undefined }));
  };
  const [newFolderName, setNewFolderName] = useState('');
  const [editingId, setEditingId] = useState<number | null>(null);
  const [editName, setEditName] = useState('');
  const [dialogOpen, setDialogOpen] = useState(false);
  const [exportFormat, setExportFormat] = useState<CitationFormat>('bibtex');
  const [selectedArticleKeys, setSelectedArticleKeys] = useState<string[]>([]);
  const [bulkRemoveTarget, setBulkRemoveTarget] = useState<FavoriteArticleRef[] | null>(null);
  const [moveTargetFolderId, setMoveTargetFolderId] = useState<string>('');
  const [batchFeedback, setBatchFeedback] = useState<{
    tone: 'success' | 'error';
    message: string;
  } | null>(null);
  const [exportFeedback, setExportFeedback] = useState<{
    tone: 'success' | 'error';
    message: string;
  } | null>(null);
  const editInputRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    if (editingId === null) {
      return;
    }
    if (window.matchMedia('(pointer: fine)').matches) {
      editInputRef.current?.focus();
    }
  }, [editingId]);

  const { data: folders = [], isLoading } = useQuery(foldersQuery());
  const activeFolderId = getActiveFavoriteFolderId(folders, selectedFolderId);
  const selectedFolder = folders.find((folder) => folder.id === activeFolderId) || null;

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
    ...folderArticlePagesQuery(activeFolderId ?? 0),
    enabled: !!activeFolderId && !!selectedFolder,
  });

  const favoritePages = useMemo(() => favPages?.pages ?? [], [favPages?.pages]);
  const loadedPages = favoritePages.length;
  const listKey = String(activeFolderId ?? 'none');
  const { visiblePages, prefetchRef, loadMoreRef } = useVisiblePageList({
    listKey,
    loadedPages,
    hasNextPage,
    isFetchingNextPage,
    onFetchNextPage: () => void fetchNextPage(),
    scrollContainerId: 'results-scroll-container',
  });
  const visiblePageCount = Math.min(visiblePages, loadedPages);
  const favorites = useMemo(
    () => favoritePages.slice(0, visiblePageCount).flatMap((page) => page.items),
    [favoritePages, visiblePageCount],
  );
  const prefetchIndex = Math.max(0, favorites.length - 25);
  const selectedKeySet = useMemo(() => new Set(selectedArticleKeys), [selectedArticleKeys]);
  const selectedFavorites = useMemo(
    () =>
      favorites.filter((favorite) =>
        selectedKeySet.has(
          getFavoriteSelectionKey(favorite.folder_id, favorite.article_id, favorite.db_name),
        ),
      ),
    [favorites, selectedKeySet],
  );
  const allLoadedSelected = favorites.length > 0 && selectedFavorites.length === favorites.length;
  const moveTargetFolders = useMemo(
    () => folders.filter((folder) => folder.id !== activeFolderId),
    [folders, activeFolderId],
  );
  const effectiveMoveTargetFolderId = getEffectiveMoveTargetFolderId(
    moveTargetFolders,
    moveTargetFolderId,
  );

  const createMut = useMutation({
    mutationFn: (name: string) => createFolder(name),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: foldersQuery().queryKey });
      setNewFolderName('');
      setDialogOpen(false);
    },
  });

  const deleteMut = useMutation({
    mutationFn: (id: number) => deleteFolder(id),
    onSuccess: async (_data, deletedFolderId) => {
      await invalidateFavoriteMemberships(queryClient);
      setSelectedArticleKeys([]);
      setMoveTargetFolderId('');
      setBatchFeedback(null);
      setExportFeedback(null);
      void queryClient.invalidateQueries({ queryKey: foldersQuery().queryKey });
      if (selectedFolderId === deletedFolderId) {
        setSelectedFolderId(null);
      }
    },
  });

  const renameMut = useMutation({
    mutationFn: ({ id, name }: { id: number; name: string }) => renameFolder(id, name),
    onSuccess: async () => {
      await invalidateFavoriteMemberships(queryClient);
      void queryClient.invalidateQueries({ queryKey: foldersQuery().queryKey });
      setEditingId(null);
    },
  });

  const trackMut = useMutation({
    mutationFn: (folderId: number) => setTrackingFolder(folderId),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: foldersQuery().queryKey }),
  });

  const exportMut = useMutation({
    mutationFn: async ({ folderId, format }: { folderId: number; format: CitationFormat }) => {
      const { blob, filename } = await downloadFavoriteExport(folderId, format);
      const isSaved = await CLIENT_PLATFORM.saveDownload(blob, filename);
      return { filename, isSaved };
    },
    onMutate: () => setExportFeedback(null),
    onSuccess: ({ filename, isSaved }) => {
      if (isSaved) {
        setExportFeedback({ tone: 'success', message: `已导出 ${filename}。` });
      }
    },
    onError: (error) => {
      setExportFeedback({
        tone: 'error',
        message: error instanceof Error ? error.message : '导出引用失败',
      });
    },
  });

  const bulkRemoveMut = useMutation({
    mutationFn: (articles: FavoriteArticleRef[]) => bulkRemoveFavorites(activeFolderId!, articles),
    onSuccess: async (count) => {
      await invalidateFavoriteMemberships(queryClient);
      setBulkRemoveTarget(null);
      setSelectedArticleKeys([]);
      setBatchFeedback({
        tone: 'success',
        message: `已从当前收藏夹移除 ${count} 篇文章。`,
      });
      void queryClient.invalidateQueries({ queryKey: FOLDER_ARTICLES_QUERY_PREFIX });
      void queryClient.invalidateQueries({ queryKey: foldersQuery().queryKey });
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
    onSuccess: async (count) => {
      await invalidateFavoriteMemberships(queryClient);
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
      void queryClient.invalidateQueries({ queryKey: FOLDER_ARTICLES_QUERY_PREFIX });
      void queryClient.invalidateQueries({ queryKey: foldersQuery().queryKey });
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
    setSelectedFolderId(folderId);
    setSelectedArticleKeys([]);
    setMoveTargetFolderId('');
    setBatchFeedback(null);
    setExportFeedback(null);
  };

  /**
   * Snapshot the current selection and open its confirmation dialog.
   */
  const handleBulkRemove = () => {
    if (selectedFavorites.length === 0) {
      return;
    }
    bulkRemoveMut.reset();
    setBatchFeedback(null);
    setBulkRemoveTarget(selectedFavorites.map(toFavoriteArticleRef));
  };

  /**
   * Submit the previously confirmed bulk-removal snapshot.
   */
  const confirmBulkRemove = () => {
    if (bulkRemoveTarget?.length) {
      bulkRemoveMut.mutate(bulkRemoveTarget);
    }
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
    bulkRemoveTarget,
    bulkMoveMut,
    bulkRemoveMut,
    confirmBulkRemove,
    createMut,
    deleteMut,
    dialogOpen,
    editInputRef,
    editName,
    editingId,
    effectiveMoveTargetFolderId,
    exportFeedback,
    exportFormat,
    exportMut,
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
    setBulkRemoveTarget,
    toggleFavoriteSelection,
    trackMut,
    visiblePageCount,
  };
}

/** Select a valid URL folder before the tracking, first-folder and empty fallbacks. */
function getActiveFavoriteFolderId(folders: Folder[], selectedId: number | null): number | null {
  if (selectedId !== null && folders.some((folder) => folder.id === selectedId)) return selectedId;
  return folders.find((folder) => folder.is_tracking)?.id ?? folders[0]?.id ?? null;
}
/** Retain an available move target using the original string identity comparison. */
function getEffectiveMoveTargetFolderId(folders: Folder[], value: string): string {
  return folders.some((folder) => String(folder.id) === value) ? value : '';
}
