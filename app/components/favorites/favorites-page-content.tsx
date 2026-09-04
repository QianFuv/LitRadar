'use client';

import { useState } from 'react';
import { Download, FolderPlus, Pencil, Radar, Star, Trash2 } from 'lucide-react';

import type { CitationFormat, FavoriteArticleItem } from '@/lib/api';
import { ArticleDialogCard } from '@/components/feature/article-dialog-card';
import { WorkspaceSidebar } from '@/components/feature/sidebar';
import { WorkspaceShell } from '@/components/feature/workspace-shell';
import {
  getFavoriteSelectionKey,
  useFavoritesPage,
} from '@/components/favorites/use-favorites-page';
import { Badge } from '@/components/ui/badge';
import { Button } from '@/components/ui/button';
import { Card, CardContent, CardHeader } from '@/components/ui/card';
import { Checkbox } from '@/components/ui/checkbox';
import { ConfirmDialog } from '@/components/ui/confirm-dialog';
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog';
import { Input } from '@/components/ui/input';
import {
  FADE_UP_VARIANTS,
  FADE_VARIANTS,
  MOTION_DURATION_SECONDS,
  MotionDiv,
  MotionParagraph,
  MotionPresence,
  useMotionTransition,
} from '@/components/ui/motion';
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@/components/ui/select';
import { Skeleton } from '@/components/ui/skeleton';
import { StateMessage } from '@/components/ui/state-message';
import { cn } from '@/lib/utils';

type FavoritesFeedback = {
  message: string;
  tone: 'error' | 'success';
};

type AnimatedFeedbackProps = {
  feedback: FavoritesFeedback | null;
  scope: 'batch' | 'export';
};

/**
 * Render one animated feedback line with an immediate, non-duplicated live announcement.
 *
 * @param props - Feedback content and stable scope identifier.
 * @returns Animated visual feedback and its current semantic announcement.
 */
function AnimatedFeedback({ feedback, scope }: AnimatedFeedbackProps) {
  const transition = useMotionTransition(MOTION_DURATION_SECONDS.fast);

  return (
    <>
      {feedback && (
        <p
          key={`${scope}-${feedback.tone}-${feedback.message}`}
          data-testid={`${scope}-feedback-announcement`}
          className="sr-only"
          role={feedback.tone === 'error' ? 'alert' : 'status'}
        >
          {feedback.message}
        </p>
      )}
      <MotionPresence>
        {feedback && (
          <MotionParagraph
            key={`${scope}-${feedback.tone}-${feedback.message}`}
            data-motion-feedback-key={`${scope}-${feedback.tone}`}
            aria-hidden="true"
            className={cn(
              'text-sm',
              feedback.tone === 'error' ? 'text-destructive' : 'text-emerald-700',
            )}
            variants={FADE_UP_VARIANTS}
            initial="hidden"
            animate="visible"
            exit={{ opacity: 0, pointerEvents: 'none', y: -2 }}
            transition={transition}
          >
            {feedback.message}
          </MotionParagraph>
        )}
      </MotionPresence>
    </>
  );
}

/**
 * Render favorite folders, article pages, exports, and batch controls.
 *
 * @param props - Authenticated user identifier.
 * @returns Favorites feature UI.
 */
export function FavoritesPageContent({ userId }: { userId: number }) {
  const {
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
    setBulkRemoveTarget,
    toggleFavoriteSelection,
    trackMut,
    visiblePageCount,
  } = useFavoritesPage(userId);
  const [folderToDelete, setFolderToDelete] = useState<{ id: number; name: string } | null>(null);
  const [favoriteToRemove, setFavoriteToRemove] = useState<FavoriteArticleItem | null>(null);
  const stateTransition = useMotionTransition(MOTION_DURATION_SECONDS.base);
  const itemTransition = useMotionTransition(MOTION_DURATION_SECONDS.fast, 'exit');
  const favoritesState = isLoading
    ? 'folders-loading'
    : !selectedFolder
      ? 'no-folder'
      : isPendingFavorites
        ? 'loading'
        : isFavoritesError
          ? 'error'
          : favorites.length === 0
            ? 'empty'
            : 'results';
  const favoritesAnnouncementRole = favoritesState === 'error' ? 'alert' : 'status';
  const favoritesAnnouncement =
    favoritesState === 'folders-loading'
      ? '正在加载收藏夹'
      : favoritesState === 'no-folder'
        ? '请选择一个收藏夹查看文章'
        : favoritesState === 'loading'
          ? `正在加载“${selectedFolder?.name ?? ''}”中的收藏文章`
          : favoritesState === 'error'
            ? `加载收藏文章失败：${favoritesError instanceof Error ? favoritesError.message : '未知错误'}`
            : favoritesState === 'empty'
              ? `“${selectedFolder?.name ?? ''}”中暂无收藏文章`
              : isFetchingNextPage
                ? '正在加载更多收藏文章'
                : `已加载 ${favorites.length} 篇收藏文章`;

  return (
    <>
      <WorkspaceShell
        sidebar={
          <WorkspaceSidebar
            headerContent={
              <div className="space-y-3 border-t border-sidebar-border pt-4">
                <div className="flex items-center justify-between">
                  <h2 className="text-sm font-semibold text-muted-foreground uppercase tracking-wider">
                    收藏夹
                  </h2>
                  <Button
                    variant="outline"
                    size="icon"
                    className="h-7 w-7"
                    aria-label="新建收藏夹"
                    onClick={() => setDialogOpen(true)}
                  >
                    <FolderPlus className="h-4 w-4" />
                  </Button>
                </div>

                <div className="space-y-1">
                  <MotionPresence>
                    {isLoading ? (
                      <MotionDiv
                        key="folder-list-loading"
                        aria-hidden="true"
                        className="text-sm text-muted-foreground"
                        variants={FADE_VARIANTS}
                        initial="hidden"
                        animate="visible"
                        exit={{ opacity: 0, pointerEvents: 'none' }}
                        transition={stateTransition}
                      >
                        加载中…
                      </MotionDiv>
                    ) : folders.length === 0 ? (
                      <MotionDiv
                        key="folder-list-empty"
                        className="text-sm text-muted-foreground"
                        variants={FADE_UP_VARIANTS}
                        initial="hidden"
                        animate="visible"
                        exit={{ opacity: 0, pointerEvents: 'none', y: -2 }}
                        transition={stateTransition}
                      >
                        暂无收藏夹，点击 + 创建
                      </MotionDiv>
                    ) : (
                      folders.map((folder) => (
                        <MotionDiv
                          key={folder.id}
                          data-motion-folder-key={folder.id}
                          className={cn(
                            'motion-control flex items-center gap-2 rounded-md px-3 py-2 text-sm transition-[background-color,color,box-shadow]',
                            activeFolderId === folder.id
                              ? 'bg-accent text-accent-foreground shadow-vercel-ring'
                              : 'hover:bg-accent/50',
                          )}
                          variants={FADE_UP_VARIANTS}
                          initial="hidden"
                          animate="visible"
                          exit={{ opacity: 0, pointerEvents: 'none', y: -3 }}
                          transition={itemTransition}
                        >
                          <div className="grid min-w-0 flex-1">
                            <MotionPresence>
                              {editingId === folder.id ? (
                                <MotionDiv
                                  key={`folder-${folder.id}-edit`}
                                  data-motion-folder-mode="edit"
                                  className="col-start-1 row-start-1"
                                  variants={FADE_VARIANTS}
                                  initial="hidden"
                                  animate="visible"
                                  exit={{ opacity: 0, pointerEvents: 'none' }}
                                  transition={itemTransition}
                                >
                                  <form
                                    className="flex gap-1"
                                    onSubmit={(event) => {
                                      event.preventDefault();
                                      if (editName.trim()) {
                                        renameMut.mutate({ id: folder.id, name: editName.trim() });
                                      }
                                    }}
                                    onClick={(event) => event.stopPropagation()}
                                  >
                                    <Input
                                      ref={editInputRef}
                                      aria-label={`重命名收藏夹 ${folder.name}`}
                                      name="favorite_folder_rename"
                                      autoComplete="off"
                                      value={editName}
                                      onChange={(event) => setEditName(event.target.value)}
                                      className="h-6 text-sm"
                                    />
                                  </form>
                                </MotionDiv>
                              ) : (
                                <MotionDiv
                                  key={`folder-${folder.id}-display`}
                                  data-motion-folder-mode="display"
                                  className="col-start-1 row-start-1 min-w-0"
                                  variants={FADE_VARIANTS}
                                  initial="hidden"
                                  animate="visible"
                                  exit={{ opacity: 0, pointerEvents: 'none' }}
                                  transition={itemTransition}
                                >
                                  <button
                                    type="button"
                                    className="flex w-full min-w-0 items-center gap-2 text-left outline-none focus-visible:ring-ring/50 focus-visible:ring-[3px]"
                                    aria-pressed={activeFolderId === folder.id}
                                    onClick={() => handleSelectFolder(folder.id)}
                                  >
                                    <Star className="h-4 w-4 shrink-0" aria-hidden="true" />
                                    <span className="min-w-0 flex-1 truncate">{folder.name}</span>
                                    {folder.is_tracking && (
                                      <Badge variant="secondary" className="px-1.5 text-[10px]">
                                        追踪
                                      </Badge>
                                    )}
                                    <span className="text-xs text-muted-foreground">
                                      {folder.article_count}
                                    </span>
                                  </button>
                                </MotionDiv>
                              )}
                            </MotionPresence>
                          </div>
                          <div className="flex gap-0.5">
                            <Button
                              variant="ghost"
                              size="icon"
                              className="h-6 w-6"
                              title="设为追踪文件夹"
                              aria-label={`设 ${folder.name} 为追踪文件夹`}
                              onClick={(event) => {
                                event.stopPropagation();
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
                              onClick={(event) => {
                                event.stopPropagation();
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
                              disabled={deleteMut.isPending}
                              onClick={(event) => {
                                event.stopPropagation();
                                deleteMut.reset();
                                setFolderToDelete({ id: folder.id, name: folder.name });
                              }}
                            >
                              <Trash2 className="h-3 w-3" />
                            </Button>
                          </div>
                        </MotionDiv>
                      ))
                    )}
                  </MotionPresence>
                </div>
              </div>
            }
          />
        }
        sidebarOpenLabel="打开收藏夹"
        sidebarDialogTitle="收藏夹"
        sidebarDialogDescription="选择和管理收藏夹。"
        toolbar={
          <div className="flex min-w-0 flex-1 items-center gap-3 md:mx-auto md:max-w-4xl">
            <Star className="size-5 shrink-0" aria-hidden="true" />
            <div className="min-w-0">
              <p className="text-xs text-muted-foreground">文献管理</p>
              <h1 className="truncate text-xl font-semibold tracking-tight">我的收藏</h1>
            </div>
          </div>
        }
      >
        <div>
          <p
            key={`${favoritesState}-${favoritesAnnouncement}`}
            data-testid="favorites-state-announcement"
            className="sr-only"
            role={favoritesAnnouncementRole}
            aria-label={favoritesAnnouncement}
            aria-live={favoritesAnnouncementRole === 'alert' ? 'assertive' : 'polite'}
            aria-atomic="true"
          >
            {favoritesAnnouncement}
          </p>
          <MotionPresence mode="wait">
            {!selectedFolder ? (
              <MotionDiv
                key="favorites-no-folder"
                data-favorites-state={favoritesState}
                variants={FADE_UP_VARIANTS}
                initial="hidden"
                animate="visible"
                exit={{ opacity: 0, pointerEvents: 'none', y: -4 }}
                transition={stateTransition}
              >
                <StateMessage
                  isLive={false}
                  title={isLoading ? '正在加载收藏夹' : '选择一个收藏夹'}
                  description={
                    isLoading ? '收藏夹加载完成后即可查看文章。' : '从侧栏选择或创建收藏夹。'
                  }
                />
              </MotionDiv>
            ) : (
              <MotionDiv
                key={`favorites-folder-${selectedFolder.id}`}
                data-favorites-folder={selectedFolder.id}
                className="space-y-3"
                variants={FADE_UP_VARIANTS}
                initial="hidden"
                animate="visible"
                exit={{ opacity: 0, pointerEvents: 'none', y: -4 }}
                transition={stateTransition}
              >
                <section className="space-y-3 rounded-lg bg-muted/30 px-4 py-4 shadow-vercel-ring">
                  <div className="flex flex-col gap-3 sm:flex-row sm:items-center sm:justify-between">
                    <div>
                      <h2 className="text-xl font-semibold tracking-tight">
                        {selectedFolder.name}
                        <span className="ml-2 text-sm font-normal text-muted-foreground">
                          {selectedFolder.article_count} 篇
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
                        disabled={exportMut.isPending}
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
                      <Button
                        type="button"
                        variant="outline"
                        disabled={exportMut.isPending}
                        onClick={() =>
                          exportMut.mutate({
                            folderId: selectedFolder.id,
                            format: exportFormat,
                          })
                        }
                      >
                        <Download className="h-4 w-4" aria-hidden="true" />
                        {exportMut.isPending ? '导出中…' : '导出引用'}
                      </Button>
                    </div>
                  </div>
                  <AnimatedFeedback feedback={exportFeedback} scope="export" />
                </section>

                <MotionPresence mode="wait">
                  {isPendingFavorites ? (
                    <MotionDiv
                      key="favorites-loading"
                      data-favorites-state="loading"
                      className="space-y-4"
                      aria-hidden="true"
                      variants={FADE_UP_VARIANTS}
                      initial="hidden"
                      animate="visible"
                      exit={{ opacity: 0, pointerEvents: 'none', y: -4 }}
                      transition={stateTransition}
                    >
                      {Array.from({ length: 3 }).map((_, index) => (
                        <Card key={index}>
                          <CardHeader>
                            <Skeleton className="h-6 w-3/4" />
                            <Skeleton className="mt-2 h-4 w-1/4" />
                          </CardHeader>
                          <CardContent>
                            <Skeleton className="h-4 w-full" />
                            <Skeleton className="mt-2 h-4 w-full" />
                          </CardContent>
                        </Card>
                      ))}
                    </MotionDiv>
                  ) : isFavoritesError ? (
                    <MotionDiv
                      key="favorites-error"
                      data-favorites-state="error"
                      variants={FADE_UP_VARIANTS}
                      initial="hidden"
                      animate="visible"
                      exit={{ opacity: 0, pointerEvents: 'none', y: -4 }}
                      transition={stateTransition}
                    >
                      <StateMessage
                        isLive={false}
                        tone="danger"
                        title="加载收藏文章失败"
                        description={
                          favoritesError instanceof Error ? favoritesError.message : '请稍后重试。'
                        }
                        action={
                          <Button
                            variant="outline"
                            size="sm"
                            onClick={() => void refetchFavorites()}
                          >
                            重试
                          </Button>
                        }
                      />
                    </MotionDiv>
                  ) : favorites.length === 0 ? (
                    <MotionDiv
                      key="favorites-empty"
                      data-favorites-state="empty"
                      variants={FADE_UP_VARIANTS}
                      initial="hidden"
                      animate="visible"
                      exit={{ opacity: 0, pointerEvents: 'none', y: -4 }}
                      transition={stateTransition}
                    >
                      <StateMessage
                        isLive={false}
                        title="此收藏夹为空"
                        description="收藏文章后，它们会按加入顺序显示在这里。"
                      />
                    </MotionDiv>
                  ) : (
                    <MotionDiv
                      key="favorites-results"
                      data-favorites-state="results"
                      className="space-y-3"
                      variants={FADE_UP_VARIANTS}
                      initial="hidden"
                      animate="visible"
                      exit={{ opacity: 0, pointerEvents: 'none', y: -4 }}
                      transition={stateTransition}
                    >
                      <section className="grid gap-3 rounded-lg bg-muted/35 p-3 shadow-vercel-ring xl:grid-cols-[minmax(0,1fr)_auto] xl:items-center">
                        <div className="flex flex-wrap items-center gap-2">
                          <Checkbox
                            checked={
                              allLoadedSelected || (selectedFavorites.length > 0 && 'indeterminate')
                            }
                            onCheckedChange={(checked: boolean | 'indeterminate') =>
                              handleSelectAllLoaded(checked === true)
                            }
                            aria-label="选择当前已加载文章"
                          />
                          <div className="mr-1 min-w-28">
                            <p className="text-sm font-semibold">
                              已选 {selectedFavorites.length} 篇
                            </p>
                            <p className="text-xs text-muted-foreground">
                              当前列表共 {favorites.length} 篇
                            </p>
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
                            <span className="basis-full text-xs text-muted-foreground">
                              批量操作仅作用于当前列表中的 {favorites.length} 篇文章
                            </span>
                          )}
                        </div>
                        <div
                          role="group"
                          aria-label="批量操作"
                          className="flex flex-col gap-2 sm:flex-row sm:items-center"
                        >
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
                            className="border-destructive/30 text-destructive"
                            onClick={handleBulkRemove}
                            disabled={selectedFavorites.length === 0 || bulkRemoveMut.isPending}
                          >
                            {bulkRemoveMut.isPending ? '删除中…' : '删除所选'}
                          </Button>
                        </div>
                        <div className="xl:col-span-2">
                          <AnimatedFeedback feedback={batchFeedback} scope="batch" />
                        </div>
                      </section>
                      <MotionPresence propagate>
                        {favorites.map((favorite, index) => {
                          const selectionKey = getFavoriteSelectionKey(
                            favorite.folder_id,
                            favorite.article_id,
                            favorite.db_name,
                          );
                          const isSelected = selectedKeySet.has(selectionKey);

                          return (
                            <MotionDiv
                              key={favorite.id}
                              data-motion-favorite-key={favorite.id}
                              className="content-visibility-card"
                              initial={false}
                              animate={{ opacity: 1, scale: 1 }}
                              exit={{ opacity: 0, pointerEvents: 'none', scale: 0.985 }}
                              transition={itemTransition}
                            >
                              <ArticleDialogCard
                                triggerRef={index === prefetchIndex ? prefetchRef : undefined}
                                article={favorite}
                                dbName={favorite.db_name}
                                initialFolderIds={[favorite.folder_id]}
                                preview={
                                  favorite.metadata_status === 'missing' ? (
                                    <span className="text-amber-700">
                                      来源数据库或文章已不存在。收藏仍保留；可移动、移除，导出时会保留空元数据条目。
                                    </span>
                                  ) : favorite.metadata_status === 'unavailable' ? (
                                    <span className="text-destructive">
                                      文章元数据暂时无法读取，请稍后重试。收藏仍保留，可移动或移除；当前导出会明确报错。
                                    </span>
                                  ) : undefined
                                }
                                leading={
                                  <Checkbox
                                    checked={isSelected}
                                    onCheckedChange={(checked: boolean | 'indeterminate') =>
                                      toggleFavoriteSelection(favorite, checked === true)
                                    }
                                    aria-label={`选择文章 ${favorite.title || favorite.article_id}`}
                                  />
                                }
                                extraActions={
                                  <Button
                                    variant="outline"
                                    size="sm"
                                    className="size-11 border-destructive/30 p-0 text-destructive md:h-10 md:w-auto md:px-3"
                                    aria-label="移除收藏"
                                    title="移除收藏"
                                    disabled={removeMut.isPending}
                                    onClick={(event) => {
                                      event.stopPropagation();
                                      removeMut.reset();
                                      setFavoriteToRemove(favorite);
                                    }}
                                  >
                                    <Trash2 className="h-4 w-4" aria-hidden="true" />
                                    <span className="hidden md:inline">移除收藏</span>
                                  </Button>
                                }
                              />
                            </MotionDiv>
                          );
                        })}
                      </MotionPresence>
                      {(visiblePageCount < loadedPages || hasNextPage) && (
                        <div ref={loadMoreRef} className="h-1" />
                      )}
                      <MotionPresence>
                        {isFetchingNextPage && (
                          <MotionDiv
                            key="favorites-next-page"
                            aria-hidden="true"
                            className="flex justify-center py-4"
                            variants={FADE_UP_VARIANTS}
                            initial="hidden"
                            animate="visible"
                            exit={{ opacity: 0, pointerEvents: 'none', y: -2 }}
                            transition={stateTransition}
                          >
                            <Skeleton className="h-8 w-48" />
                          </MotionDiv>
                        )}
                      </MotionPresence>
                    </MotionDiv>
                  )}
                </MotionPresence>
              </MotionDiv>
            )}
          </MotionPresence>
        </div>
      </WorkspaceShell>
      <Dialog open={dialogOpen} onOpenChange={setDialogOpen}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>新建收藏夹</DialogTitle>
            <DialogDescription>输入收藏夹名称</DialogDescription>
          </DialogHeader>
          <form
            onSubmit={(event) => {
              event.preventDefault();
              if (newFolderName.trim()) createMut.mutate(newFolderName.trim());
            }}
            className="space-y-4"
          >
            <Input
              aria-label="收藏夹名称"
              name="favorite_folder_name"
              autoComplete="off"
              value={newFolderName}
              onChange={(event) => setNewFolderName(event.target.value)}
              placeholder="收藏夹名称"
            />
            <Button type="submit" disabled={createMut.isPending}>
              创建
            </Button>
          </form>
        </DialogContent>
      </Dialog>
      <ConfirmDialog
        open={folderToDelete !== null}
        onOpenChange={(nextOpen) => {
          if (!nextOpen && !deleteMut.isPending) {
            setFolderToDelete(null);
          }
        }}
        title="删除收藏夹？"
        description={`确认删除收藏夹“${folderToDelete?.name ?? ''}”？收藏夹内的收藏关系也会被删除。`}
        actionLabel="确认删除"
        pendingLabel="删除中…"
        isPending={deleteMut.isPending}
        error={deleteMut.error instanceof Error ? deleteMut.error.message : null}
        onConfirm={() => {
          if (folderToDelete) {
            deleteMut.mutate(folderToDelete.id, {
              onSuccess: (_data, deletedFolderId) => {
                setFolderToDelete((current) => (current?.id === deletedFolderId ? null : current));
              },
            });
          }
        }}
      />
      <ConfirmDialog
        open={favoriteToRemove !== null}
        onOpenChange={(nextOpen) => {
          if (!nextOpen && !removeMut.isPending) {
            setFavoriteToRemove(null);
          }
        }}
        title="移除收藏？"
        description={`确认从当前收藏夹移除“${favoriteToRemove?.title || favoriteToRemove?.article_id || ''}”？`}
        actionLabel="确认移除"
        pendingLabel="移除中…"
        isPending={removeMut.isPending}
        error={removeMut.error instanceof Error ? removeMut.error.message : null}
        onConfirm={() => {
          if (favoriteToRemove) {
            removeMut.mutate(favoriteToRemove, {
              onSuccess: (_data, removedFavorite) => {
                setFavoriteToRemove((current) =>
                  current?.folder_id === removedFavorite.folder_id &&
                  current.article_id === removedFavorite.article_id &&
                  current.db_name === removedFavorite.db_name
                    ? null
                    : current,
                );
              },
            });
          }
        }}
      />
      <ConfirmDialog
        open={bulkRemoveTarget !== null}
        onOpenChange={(nextOpen) => {
          if (!nextOpen && !bulkRemoveMut.isPending) {
            setBulkRemoveTarget(null);
          }
        }}
        title="移除所选收藏？"
        description={`确认从当前收藏夹移除 ${bulkRemoveTarget?.length ?? 0} 篇文章？`}
        actionLabel="确认移除"
        pendingLabel="移除中…"
        isPending={bulkRemoveMut.isPending}
        error={bulkRemoveMut.error instanceof Error ? bulkRemoveMut.error.message : null}
        onConfirm={confirmBulkRemove}
      />
    </>
  );
}
