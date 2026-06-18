'use client';

import { useEffect, useRef, useState } from 'react';
import { useQuery, useMutation, useQueryClient, useInfiniteQuery } from '@tanstack/react-query';
import Link from 'next/link';
import { ArrowLeft, Download, FolderPlus, Pencil, Radar, Star, Trash2 } from 'lucide-react';

import { useAuth } from '@/lib/auth-context';
import {
  getFolders,
  createFolder,
  deleteFolder,
  renameFolder,
  getFolderArticles,
  bulkMoveFavorites,
  bulkRemoveFavorites,
  removeFavorite,
  setTrackingFolder,
  getExportUrl,
  type ArticleId,
  type CitationFormat,
  type FavoriteArticleItem,
  type FavoriteArticleRef,
  type FavoriteItem,
} from '@/lib/api';
import { ArticleDialogCard } from '@/components/feature/article-dialog-card';
import { useVisiblePageList } from '@/components/feature/use-visible-page-list';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import { Card, CardContent, CardHeader } from '@/components/ui/card';
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
  DialogTrigger,
} from '@/components/ui/dialog';
import { Badge } from '@/components/ui/badge';
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@/components/ui/select';
import { Skeleton } from '@/components/ui/skeleton';
import { Checkbox } from '@/components/ui/checkbox';
import { cn } from '@/lib/utils';

function getFavoriteSelectionKey(folderId: number, articleId: ArticleId, dbName: string): string {
  return `${folderId}:${articleId}:${dbName}`;
}

function toFavoriteArticleRef(favorite: FavoriteArticleItem): FavoriteArticleRef {
  return {
    article_id: favorite.article_id,
    db_name: favorite.db_name,
  };
}

export default function FavoritesPage() {
  const { user, token } = useAuth();
  const queryClient = useQueryClient();
  const [selectedFolderId, setSelectedFolderId] = useState<number | null>(null);
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
    queryKey: ['folders', user?.id],
    queryFn: () => getFolders(token!),
    enabled: !!token,
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
    queryFn: ({ pageParam = 0 }) =>
      getFolderArticles(token!, activeFolderId!, PAGE_SIZE, pageParam),
    getNextPageParam: (lastPage, allPages) =>
      lastPage.length === PAGE_SIZE ? allPages.flat().length : undefined,
    initialPageParam: 0,
    enabled: !!token && !!activeFolderId && !!selectedFolder,
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
    mutationFn: (name: string) => createFolder(token!, name),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['folders'] });
      setNewFolderName('');
      setDialogOpen(false);
    },
  });

  const deleteMut = useMutation({
    mutationFn: (id: number) => deleteFolder(token!, id),
    onSuccess: () => {
      setSelectedArticleKeys([]);
      setMoveTargetFolderId('');
      setBatchFeedback(null);
      queryClient.invalidateQueries({ queryKey: ['folders'] });
      if (selectedFolderId === deleteMut.variables) {
        setSelectedFolderId(null);
      }
    },
  });

  const renameMut = useMutation({
    mutationFn: ({ id, name }: { id: number; name: string }) => renameFolder(token!, id, name),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['folders'] });
      setEditingId(null);
    },
  });

  const trackMut = useMutation({
    mutationFn: (folderId: number) => setTrackingFolder(token!, folderId),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ['folders'] }),
  });

  const removeMut = useMutation({
    mutationFn: (item: FavoriteItem) =>
      removeFavorite(token!, item.folder_id, item.article_id, item.db_name),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['folder-articles'] });
      queryClient.invalidateQueries({ queryKey: ['folders'] });
    },
  });

  const bulkRemoveMut = useMutation({
    mutationFn: (articles: FavoriteArticleRef[]) =>
      bulkRemoveFavorites(token!, activeFolderId!, articles),
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
    }) => bulkMoveFavorites(token!, activeFolderId!, targetFolderId, articles),
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
    setSelectedFolderId(folderId);
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

  if (!user) {
    return (
      <main
        id="main-content"
        className="flex flex-col items-center justify-center min-h-[60vh] gap-4"
      >
        <p className="text-muted-foreground">请先登录</p>
        <Button asChild>
          <Link href="/login?next=/favorites">登录</Link>
        </Button>
      </main>
    );
  }

  return (
    <main id="main-content" className="max-w-5xl mx-auto p-6 space-y-6">
      <div className="flex items-center gap-3">
        <Button variant="ghost" size="icon" aria-label="返回首页" asChild>
          <Link href="/">
            <ArrowLeft className="h-5 w-5" />
          </Link>
        </Button>
        <h1 className="text-2xl font-bold">我的收藏</h1>
      </div>

      <div className="grid md:grid-cols-[280px_1fr] gap-6">
        {/* Folder list */}
        <div className="space-y-3">
          <div className="flex items-center justify-between">
            <h2 className="text-sm font-semibold text-muted-foreground uppercase tracking-wider">
              收藏夹
            </h2>
            <Dialog open={dialogOpen} onOpenChange={setDialogOpen}>
              <DialogTrigger asChild>
                <Button variant="outline" size="icon" className="h-7 w-7" aria-label="新建收藏夹">
                  <FolderPlus className="h-4 w-4" />
                </Button>
              </DialogTrigger>
              <DialogContent>
                <DialogHeader>
                  <DialogTitle>新建收藏夹</DialogTitle>
                  <DialogDescription>输入收藏夹名称</DialogDescription>
                </DialogHeader>
                <form
                  onSubmit={(e) => {
                    e.preventDefault();
                    if (newFolderName.trim()) createMut.mutate(newFolderName.trim());
                  }}
                  className="space-y-4"
                >
                  <Input
                    aria-label="收藏夹名称"
                    value={newFolderName}
                    onChange={(e) => setNewFolderName(e.target.value)}
                    placeholder="收藏夹名称"
                  />
                  <Button type="submit" disabled={createMut.isPending}>
                    创建
                  </Button>
                </form>
              </DialogContent>
            </Dialog>
          </div>

          {isLoading ? (
            <div role="status" className="text-sm text-muted-foreground">
              加载中…
            </div>
          ) : folders.length === 0 ? (
            <div className="text-sm text-muted-foreground">暂无收藏夹，点击 + 创建</div>
          ) : (
            <div className="space-y-1">
              {folders.map((folder) => (
                <div
                  key={folder.id}
                  className={cn(
                    'flex items-center gap-2 rounded-md px-3 py-2 text-sm transition-colors',
                    activeFolderId === folder.id
                      ? 'bg-accent text-accent-foreground'
                      : 'hover:bg-accent/50',
                  )}
                >
                  {editingId === folder.id ? (
                    <form
                      className="flex-1 flex gap-1"
                      onSubmit={(e) => {
                        e.preventDefault();
                        if (editName.trim()) {
                          renameMut.mutate({ id: folder.id, name: editName.trim() });
                        }
                      }}
                      onClick={(e) => e.stopPropagation()}
                    >
                      <Input
                        ref={editInputRef}
                        aria-label={`重命名收藏夹 ${folder.name}`}
                        value={editName}
                        onChange={(e) => setEditName(e.target.value)}
                        className="h-6 text-sm"
                      />
                    </form>
                  ) : (
                    <button
                      type="button"
                      className="flex min-w-0 flex-1 items-center gap-2 text-left outline-none focus-visible:ring-ring/50 focus-visible:ring-[3px]"
                      aria-pressed={activeFolderId === folder.id}
                      onClick={() => handleSelectFolder(folder.id)}
                    >
                      <Star className="h-4 w-4 shrink-0" />
                      <span className="truncate flex-1">{folder.name}</span>
                      {folder.is_tracking && (
                        <Badge variant="secondary" className="text-[10px] px-1.5">
                          追踪
                        </Badge>
                      )}
                      <span className="text-xs text-muted-foreground">{folder.article_count}</span>
                    </button>
                  )}
                  <div className="flex gap-0.5">
                    <Button
                      variant="ghost"
                      size="icon"
                      className="h-6 w-6"
                      title="设为追踪文件夹"
                      aria-label={`设 ${folder.name} 为追踪文件夹`}
                      onClick={(e) => {
                        e.stopPropagation();
                        trackMut.mutate(folder.id);
                      }}
                    >
                      <Radar className="h-3 w-3" />
                    </Button>
                    <Button
                      variant="ghost"
                      size="icon"
                      className="h-6 w-6"
                      aria-label={`重命名收藏夹 ${folder.name}`}
                      onClick={(e) => {
                        e.stopPropagation();
                        setEditingId(folder.id);
                        setEditName(folder.name);
                      }}
                    >
                      <Pencil className="h-3 w-3" />
                    </Button>
                    <Button
                      variant="ghost"
                      size="icon"
                      className="h-6 w-6 text-destructive"
                      aria-label={`删除收藏夹 ${folder.name}`}
                      onClick={(e) => {
                        e.stopPropagation();
                        if (window.confirm(`确认删除收藏夹“${folder.name}”？`)) {
                          deleteMut.mutate(folder.id);
                        }
                      }}
                    >
                      <Trash2 className="h-3 w-3" />
                    </Button>
                  </div>
                </div>
              ))}
            </div>
          )}
        </div>

        {/* Article list */}
        <div>
          {!selectedFolder ? (
            <div className="flex items-center justify-center h-40 text-muted-foreground">
              选择一个收藏夹查看文章
            </div>
          ) : (
            <div className="space-y-4">
              <div className="flex flex-col gap-3 rounded-lg border bg-card px-4 py-4 sm:flex-row sm:items-center sm:justify-between">
                <div>
                  <h2 className="text-lg font-semibold">
                    {selectedFolder.name}
                    <span className="text-sm text-muted-foreground ml-2">
                      ({selectedFolder.article_count} 篇)
                    </span>
                  </h2>
                  <p className="text-sm text-muted-foreground">
                    导出当前收藏夹为 BibTeX、RIS 或 EndNote 格式
                  </p>
                </div>
                <div className="flex flex-col gap-2 sm:flex-row sm:items-center">
                  <Select
                    value={exportFormat}
                    onValueChange={(value: string) => setExportFormat(value as CitationFormat)}
                  >
                    <SelectTrigger className="w-full sm:w-40">
                      <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                      <SelectItem value="bibtex">BibTeX</SelectItem>
                      <SelectItem value="ris">RIS</SelectItem>
                      <SelectItem value="endnote">EndNote XML</SelectItem>
                    </SelectContent>
                  </Select>
                  <Button asChild variant="outline">
                    <a href={getExportUrl(token!, selectedFolder.id, exportFormat)} download>
                      <Download className="mr-2 h-4 w-4" />
                      导出引用
                    </a>
                  </Button>
                </div>
              </div>

              {isPendingFavorites ? (
                <div className="space-y-4">
                  {Array.from({ length: 3 }).map((_, idx) => (
                    <Card key={idx}>
                      <CardHeader>
                        <Skeleton className="h-6 w-3/4" />
                        <Skeleton className="h-4 w-1/4 mt-2" />
                      </CardHeader>
                      <CardContent>
                        <Skeleton className="h-4 w-full" />
                        <Skeleton className="h-4 w-full mt-2" />
                      </CardContent>
                    </Card>
                  ))}
                </div>
              ) : isFavoritesError ? (
                <div className="flex h-40 flex-col items-center justify-center gap-3 text-center">
                  <p role="alert" className="text-sm text-muted-foreground">
                    {favoritesError instanceof Error ? favoritesError.message : '加载收藏文章失败'}
                  </p>
                  <Button variant="outline" size="sm" onClick={() => void refetchFavorites()}>
                    重试
                  </Button>
                </div>
              ) : favorites.length === 0 ? (
                <div className="flex items-center justify-center h-40 text-muted-foreground">
                  此收藏夹为空
                </div>
              ) : (
                <>
                  <div className="rounded-lg border border-dashed bg-muted/30 px-4 py-3 space-y-3">
                    <div className="flex flex-col gap-3 xl:flex-row xl:items-center xl:justify-between">
                      <div className="flex flex-wrap items-center gap-3">
                        <div className="flex items-center gap-2">
                          <Checkbox
                            checked={
                              allLoadedSelected || (selectedFavorites.length > 0 && 'indeterminate')
                            }
                            onCheckedChange={(checked: boolean | 'indeterminate') =>
                              handleSelectAllLoaded(checked === true)
                            }
                            aria-label="选择当前已加载文章"
                          />
                          <span className="text-sm font-medium">
                            已选 {selectedFavorites.length} 篇
                          </span>
                        </div>
                        <Button
                          variant="ghost"
                          size="sm"
                          onClick={() => handleSelectAllLoaded(true)}
                          disabled={favorites.length === 0 || allLoadedSelected}
                        >
                          全选当前列表
                        </Button>
                        <Button
                          variant="ghost"
                          size="sm"
                          onClick={() => handleSelectAllLoaded(false)}
                          disabled={selectedFavorites.length === 0}
                        >
                          清空选择
                        </Button>
                        {(hasNextPage || visiblePageCount < loadedPages) && (
                          <span className="text-xs text-muted-foreground">
                            批量操作仅作用于当前列表中的 {favorites.length} 篇文章
                          </span>
                        )}
                      </div>
                      <div className="flex flex-col gap-2 sm:flex-row sm:items-center">
                        <Select
                          value={effectiveMoveTargetFolderId}
                          onValueChange={setMoveTargetFolderId}
                        >
                          <SelectTrigger className="w-full sm:w-48" aria-label="选择目标收藏夹">
                            <SelectValue placeholder="选择目标收藏夹" />
                          </SelectTrigger>
                          <SelectContent>
                            {moveTargetFolders.map((folder) => (
                              <SelectItem key={folder.id} value={String(folder.id)}>
                                {folder.name}
                              </SelectItem>
                            ))}
                          </SelectContent>
                        </Select>
                        <Button
                          variant="outline"
                          onClick={handleBulkMove}
                          disabled={
                            selectedFavorites.length === 0 ||
                            !effectiveMoveTargetFolderId ||
                            bulkMoveMut.isPending ||
                            moveTargetFolders.length === 0
                          }
                        >
                          {bulkMoveMut.isPending ? '移动中…' : '移动所选'}
                        </Button>
                        <Button
                          variant="outline"
                          className="text-destructive border-destructive/30"
                          onClick={handleBulkRemove}
                          disabled={selectedFavorites.length === 0 || bulkRemoveMut.isPending}
                        >
                          {bulkRemoveMut.isPending ? '删除中…' : '删除所选'}
                        </Button>
                      </div>
                    </div>
                    {batchFeedback && (
                      <p
                        role={batchFeedback.tone === 'error' ? 'alert' : 'status'}
                        className={`text-sm ${
                          batchFeedback.tone === 'error' ? 'text-destructive' : 'text-emerald-700'
                        }`}
                      >
                        {batchFeedback.message}
                      </p>
                    )}
                  </div>
                  {favorites.map((fav, index) => {
                    const selectionKey = getFavoriteSelectionKey(
                      fav.folder_id,
                      fav.article_id,
                      fav.db_name,
                    );
                    const isSelected = selectedKeySet.has(selectionKey);

                    return (
                      <ArticleDialogCard
                        key={fav.id}
                        triggerRef={index === prefetchIndex ? prefetchRef : undefined}
                        article={fav}
                        dbName={fav.db_name}
                        token={token!}
                        initialFolderIds={[fav.folder_id]}
                        leading={
                          <Checkbox
                            checked={isSelected}
                            onCheckedChange={(checked: boolean | 'indeterminate') =>
                              toggleFavoriteSelection(fav, checked === true)
                            }
                            aria-label={`选择文章 ${fav.title || fav.article_id}`}
                          />
                        }
                        extraActions={
                          <Button
                            variant="outline"
                            size="sm"
                            className="text-destructive border-destructive/30"
                            onClick={(e) => {
                              e.stopPropagation();
                              if (window.confirm('确认移除这篇收藏文章？')) {
                                removeMut.mutate(fav);
                              }
                            }}
                          >
                            <Trash2 className="mr-2 h-4 w-4" />
                            移除收藏
                          </Button>
                        }
                      />
                    );
                  })}
                  {(visiblePageCount < loadedPages || hasNextPage) && (
                    <div ref={loadMoreRef} className="h-1" />
                  )}
                  {isFetchingNextPage && (
                    <div className="py-4 flex justify-center">
                      <Skeleton className="h-8 w-48" />
                    </div>
                  )}
                </>
              )}
            </div>
          )}
        </div>
      </div>
    </main>
  );
}
