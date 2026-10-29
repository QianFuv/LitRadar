'use client';

/**
 * Favorites workspace: folder tabs, selection bar, article rows, and the reading desk.
 */

import { useState, type Dispatch, type SetStateAction } from 'react';
import { Download, FolderPlus, MoreHorizontal, Pencil, Radar, Trash2 } from 'lucide-react';

import type { CitationFormat, FavoriteArticleItem } from '@/lib/api';
import { ArticleDialogCard } from '@/components/feature/article-dialog-card';
import { ArticleListSkeleton } from '@/components/feature/article-list-card';
import { ReaderPlaceholder } from '@/components/feature/reader-placeholder';
import { WorkspaceShell } from '@/components/feature/workspace-shell';
import {
  getFavoriteSelectionKey,
  useFavoritesPage,
} from '@/components/favorites/use-favorites-page';
import { Badge } from '@/components/ui/badge';
import { Button } from '@/components/ui/button';
import { Checkbox } from '@/components/ui/checkbox';
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
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from '@/components/ui/dropdown-menu';
import { Input } from '@/components/ui/input';
import {
  FADE_UP_VARIANTS,
  MOTION_DURATION_SECONDS,
  MotionDiv,
  MotionParagraph,
  MotionPresence,
  useMotionTransition,
} from '@/components/ui/motion';
import { Popover, PopoverContent, PopoverTrigger } from '@/components/ui/popover';
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
              'text-xs leading-5',
              feedback.tone === 'error' ? 'text-destructive' : 'text-success-foreground',
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
  const page = useFavoritesPage(userId);
  const [folderToDelete, setFolderToDelete] = useState<{ id: number; name: string } | null>(null);
  const stateTransition = useMotionTransition(MOTION_DURATION_SECONDS.base);
  const itemTransition = useMotionTransition(MOTION_DURATION_SECONDS.fast, 'exit');
  const favoritesState = getFavoritesState(page);
  const state = {
    ...page,
    folderToDelete,
    setFolderToDelete,
    stateTransition,
    itemTransition,
    favoritesState,
  };
  const favoritesAnnouncementRole = favoritesState === 'error' ? 'alert' : 'status';
  const favoritesAnnouncement = getFavoritesAnnouncement(state);
  return (
    <>
      <WorkspaceShell
        listLabel="收藏文章"
        emphasis="list"
        shouldAutoSelectFirst
        header={renderFavoritesHeader(state)}
        readerPlaceholder={
          <ReaderPlaceholder
            title="选择一篇收藏开始阅读"
            description="点击左侧收藏的文献查看摘要与访问入口；勾选多篇可批量移动或移除。"
          />
        }
      >
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
        {renderFavoritesWorkspace(state)}
      </WorkspaceShell>
      {renderCreateFavoriteFolderDialog(state)}
      {renderRenameFavoriteFolderDialog(state)}
      {renderDeleteFavoriteFolderDialog(state)}
      {renderBulkRemoveFavoriteDialog(state)}
    </>
  );
}

type FavoritesState = 'folders-loading' | 'no-folder' | 'loading' | 'error' | 'empty' | 'results';
type FavoriteFolderTarget = { id: number; name: string };
type FavoritesViewState = ReturnType<typeof useFavoritesPage> & {
  folderToDelete: FavoriteFolderTarget | null;
  setFolderToDelete: Dispatch<SetStateAction<FavoriteFolderTarget | null>>;
  stateTransition: ReturnType<typeof useMotionTransition>;
  itemTransition: ReturnType<typeof useMotionTransition>;
  favoritesState: FavoritesState;
};
/** Preserve folder loading, selection, article loading, failure and empty-result precedence. */
function getFavoritesState(page: ReturnType<typeof useFavoritesPage>): FavoritesState {
  if (page.isLoading) return 'folders-loading';
  if (!page.selectedFolder) return 'no-folder';
  if (page.isPendingFavorites) return 'loading';
  if (page.isFavoritesError) return 'error';
  if (page.favorites.length === 0) return 'empty';
  return 'results';
}

/** Render the page title, folder tabs, and current-folder toolbar. */
function renderFavoritesHeader(state: FavoritesViewState) {
  const { selectedFolder, setDialogOpen } = state;

  return (
    <div className="space-y-3">
      <div className="flex items-center justify-between gap-3">
        <div className="min-w-0">
          <h1 className="text-lg leading-7 font-bold">我的收藏</h1>
          <p className="text-xs text-muted-foreground">按收藏夹整理、批量管理并导出引用</p>
        </div>
        <Button
          variant="outline"
          size="sm"
          className="shrink-0 rounded-lg"
          aria-label="新建收藏夹"
          onClick={() => setDialogOpen(true)}
        >
          <FolderPlus aria-hidden="true" />
          <span className="hidden sm:inline">新建</span>
        </Button>
      </div>
      {renderFolderTabs(state)}
      {selectedFolder && renderFolderToolbar(state, selectedFolder)}
    </div>
  );
}

/** Render folders as a horizontally scrolling row of tabs. */
function renderFolderTabs(state: FavoritesViewState) {
  const { folders, isLoading, activeFolderId, handleSelectFolder } = state;

  if (isLoading) {
    return (
      <div aria-hidden="true" className="flex gap-2">
        <Skeleton className="h-8 w-24 rounded-full" />
        <Skeleton className="h-8 w-32 rounded-full" />
        <Skeleton className="h-8 w-20 rounded-full" />
      </div>
    );
  }
  if (folders.length === 0) {
    return <p className="text-xs text-muted-foreground">暂无收藏夹，点击「新建」创建第一个。</p>;
  }
  return (
    <div
      role="group"
      aria-label="收藏夹"
      data-slot="favorite-folder-tabs"
      className="scrollbar-none fade-x-edges -mx-4 -my-1 flex gap-1.5 overflow-x-auto px-4 py-1 sm:-mx-5 sm:px-5"
    >
      {folders.map((folder) => {
        const isActive = activeFolderId === folder.id;
        return (
          <button
            key={folder.id}
            type="button"
            data-motion-folder-key={folder.id}
            aria-pressed={isActive}
            aria-label={folder.name}
            title={folder.is_tracking ? `${folder.name}（追踪文件夹）` : folder.name}
            onClick={() => handleSelectFolder(folder.id)}
            className={cn(
              'motion-control inline-flex h-11 max-w-[13rem] shrink-0 items-center gap-1.5 rounded-full px-3.5 text-[13px] md:h-8 md:px-3 outline-none transition-[background-color,color,box-shadow] focus-visible:ring-2 focus-visible:ring-ring',
              isActive
                ? 'bg-foreground font-bold text-background'
                : 'bg-background text-foreground/80 shadow-[inset_0_0_0_1px_var(--border)] hover:bg-accent',
            )}
          >
            {folder.is_tracking && (
              <Radar
                className={cn(
                  'size-3.5 shrink-0',
                  isActive ? 'text-background' : 'text-primary-text',
                )}
                aria-hidden="true"
              />
            )}
            <span className="truncate">{folder.name}</span>
            <span
              aria-hidden="true"
              className={cn(
                'shrink-0 text-[11px] tabular-nums',
                isActive ? 'text-background/70' : 'text-muted-foreground',
              )}
            >
              {folder.article_count}
            </span>
          </button>
        );
      })}
    </div>
  );
}

/** Render current-folder actions: export, tracking, rename, and delete. */
function renderFolderToolbar(
  state: FavoritesViewState,
  selectedFolder: NonNullable<FavoritesViewState['selectedFolder']>,
) {
  const { deleteMut, trackMut, setEditingId, setEditName, setFolderToDelete } = state;

  return (
    <div className="flex items-center justify-between gap-2 rounded-xl bg-muted px-3 py-2">
      <div className="min-w-0">
        <p className="flex min-w-0 items-center gap-1.5 text-[13px] font-bold">
          <span className="truncate" title={selectedFolder.name}>
            {selectedFolder.name}
          </span>
          {selectedFolder.is_tracking && <Badge className="shrink-0">追踪</Badge>}
        </p>
        <p className="text-[11px] text-muted-foreground">
          {selectedFolder.article_count} 篇
          {selectedFolder.is_tracking && ' · 每周推送会自动收藏到此处'}
        </p>
      </div>
      <div className="flex shrink-0 items-center gap-1">
        {renderFavoriteExport(state, selectedFolder)}
        <DropdownMenu>
          <DropdownMenuTrigger asChild>
            <Button
              variant="ghost"
              size="icon-sm"
              aria-label={`管理收藏夹 ${selectedFolder.name}`}
              title="管理收藏夹"
            >
              <MoreHorizontal aria-hidden="true" />
            </Button>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="end" className="w-56">
            <DropdownMenuLabel className="truncate">{selectedFolder.name}</DropdownMenuLabel>
            <DropdownMenuItem
              aria-label={`设 ${selectedFolder.name} 为追踪文件夹`}
              disabled={selectedFolder.is_tracking}
              onSelect={() => trackMut.mutate(selectedFolder.id)}
            >
              <Radar />
              {selectedFolder.is_tracking ? '已是追踪文件夹' : '设为追踪文件夹'}
            </DropdownMenuItem>
            <DropdownMenuItem
              aria-label={`重命名收藏夹 ${selectedFolder.name}`}
              onSelect={() => {
                setEditingId(selectedFolder.id);
                setEditName(selectedFolder.name);
              }}
            >
              <Pencil />
              重命名
            </DropdownMenuItem>
            <DropdownMenuSeparator />
            <DropdownMenuItem
              isDestructive
              aria-label={`删除收藏夹 ${selectedFolder.name}`}
              disabled={deleteMut.isPending}
              onSelect={() => {
                deleteMut.reset();
                setFolderToDelete({ id: selectedFolder.id, name: selectedFolder.name });
              }}
            >
              <Trash2 />
              删除收藏夹
            </DropdownMenuItem>
          </DropdownMenuContent>
        </DropdownMenu>
      </div>
    </div>
  );
}

/** Return the keyed favorite row with its visible selection and ref. */
function renderFavoriteArticle(
  state: FavoritesViewState,
  favorite: FavoriteArticleItem,
  index: number,
) {
  const { prefetchIndex, prefetchRef, selectedKeySet, toggleFavoriteSelection, itemTransition } =
    state;

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
        favoriteToggleFolderId={favorite.folder_id}
        className={cn(isSelected && 'bg-accent/50')}
        preview={
          favorite.metadata_status === 'missing' ? (
            <span className="text-warning-foreground">
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
      />
    </MotionDiv>
  );
}

/** Retain the complete favorite-row presence subtree and propagation. */
function renderFavoriteArticles(state: FavoritesViewState) {
  const { favorites } = state;

  return (
    <MotionPresence propagate>
      {favorites.map((favorite, index) => renderFavoriteArticle(state, favorite, index))}
    </MotionPresence>
  );
}

/** Render citation export as a compact popover from the folder toolbar. */
function renderFavoriteExport(
  state: Pick<
    FavoritesViewState,
    'exportFeedback' | 'exportFormat' | 'exportMut' | 'setExportFormat'
  >,
  selectedFolder: NonNullable<FavoritesViewState['selectedFolder']>,
) {
  const { exportFeedback, exportFormat, exportMut, setExportFormat } = state;

  return (
    <Popover>
      <PopoverTrigger asChild>
        <Button variant="ghost" size="sm" className="h-8 px-2.5" aria-label="导出收藏夹">
          <Download aria-hidden="true" />
          <span className="hidden sm:inline">导出</span>
        </Button>
      </PopoverTrigger>
      <PopoverContent align="end" className="w-72 space-y-3" aria-label="导出引用">
        <div>
          <p className="text-sm font-bold">导出引用</p>
          <p className="mt-0.5 text-xs text-muted-foreground">
            导出当前收藏夹为 BibTeX、RIS 或 EndNote 格式
          </p>
        </div>
        <Select
          value={exportFormat}
          onValueChange={(value: string) => setExportFormat(value as CitationFormat)}
          disabled={exportMut.isPending}
        >
          <SelectTrigger className="w-full" aria-label="导出格式">
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
          className="w-full"
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
        <AnimatedFeedback feedback={exportFeedback} scope="export" />
      </PopoverContent>
    </Popover>
  );
}

/** Render the sticky selection bar with loaded-row selection and bulk actions. */
function renderFavoriteBulkActions(state: FavoritesViewState) {
  const {
    allLoadedSelected,
    batchFeedback,
    bulkMoveMut,
    bulkRemoveMut,
    effectiveMoveTargetFolderId,
    favorites,
    handleBulkMove,
    handleBulkRemove,
    handleSelectAllLoaded,
    hasNextPage,
    loadedPages,
    moveTargetFolders,
    selectedFavorites,
    setMoveTargetFolderId,
    visiblePageCount,
  } = state;
  const hasSelection = selectedFavorites.length > 0;

  return (
    <section
      aria-label="选择与批量操作"
      className={cn(
        'sticky top-0 z-10 border-b bg-background px-4 py-2.5 sm:px-5',
        hasSelection && 'bg-info/80',
      )}
    >
      <div className="flex flex-wrap items-center gap-x-3 gap-y-2">
        <div className="flex min-w-0 items-center gap-2.5">
          <Checkbox
            checked={allLoadedSelected || (hasSelection && 'indeterminate')}
            onCheckedChange={(checked: boolean | 'indeterminate') =>
              handleSelectAllLoaded(checked === true)
            }
            aria-label="选择当前已加载文章"
          />
          <div className="min-w-0 leading-tight">
            <p className="text-[13px] font-bold">已选 {selectedFavorites.length} 篇</p>
            <p className="text-[11px] text-muted-foreground">当前列表共 {favorites.length} 篇</p>
          </div>
        </div>
        <div className="flex items-center gap-1">
          <Button
            variant="ghost"
            size="sm"
            className="h-8 px-2"
            onClick={() => handleSelectAllLoaded(true)}
            disabled={favorites.length === 0 || allLoadedSelected}
          >
            全选当前列表
          </Button>
          <Button
            variant="ghost"
            size="sm"
            className="h-8 px-2"
            onClick={() => handleSelectAllLoaded(false)}
            disabled={!hasSelection}
          >
            清空选择
          </Button>
        </div>
        <div
          role="group"
          aria-label="批量操作"
          className={cn('flex w-full items-center gap-1.5', !hasSelection && 'hidden')}
        >
          <Select value={effectiveMoveTargetFolderId} onValueChange={setMoveTargetFolderId}>
            <SelectTrigger size="sm" className="min-w-0 flex-1" aria-label="选择目标收藏夹">
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
            size="sm"
            onClick={handleBulkMove}
            disabled={isFavoriteBulkMoveDisabled(state)}
          >
            {bulkMoveMut.isPending ? '移动中…' : '移动所选'}
          </Button>
          <Button
            variant="outline"
            size="sm"
            className="text-destructive"
            onClick={handleBulkRemove}
            disabled={isFavoriteBulkRemoveDisabled(state)}
          >
            {bulkRemoveMut.isPending ? '删除中…' : '删除所选'}
          </Button>
        </div>
      </div>
      {(hasNextPage || visiblePageCount < loadedPages) && (
        <p className="mt-1 text-[11px] text-muted-foreground">
          批量操作仅作用于当前列表中的 {favorites.length} 篇文章
        </p>
      )}
      <AnimatedFeedback feedback={batchFeedback} scope="batch" />
    </section>
  );
}

/** Retain the existing captured target and pending dismissal guard. */
function renderDeleteFavoriteFolderDialog(
  state: Pick<FavoritesViewState, 'deleteMut' | 'folderToDelete' | 'setFolderToDelete'>,
) {
  const { deleteMut, folderToDelete, setFolderToDelete } = state;

  return (
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
  );
}

/** Retain the existing captured target and pending dismissal guard. */
function renderBulkRemoveFavoriteDialog(
  state: Pick<
    FavoritesViewState,
    'bulkRemoveTarget' | 'bulkRemoveMut' | 'confirmBulkRemove' | 'setBulkRemoveTarget'
  >,
) {
  const { bulkRemoveTarget, bulkRemoveMut, confirmBulkRemove, setBulkRemoveTarget } = state;

  return (
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
  );
}

/** Retain folder creation and its original form submission. */
function renderCreateFavoriteFolderDialog(
  state: Pick<
    FavoritesViewState,
    'createMut' | 'dialogOpen' | 'newFolderName' | 'setDialogOpen' | 'setNewFolderName'
  >,
) {
  const { createMut, dialogOpen, newFolderName, setDialogOpen, setNewFolderName } = state;

  return (
    <Dialog open={dialogOpen} onOpenChange={setDialogOpen}>
      <DialogContent className="md:max-w-md">
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
          <DialogFooter>
            <Button type="submit" disabled={createMut.isPending}>
              创建
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}

/** Render folder renaming in a small dialog driven by the existing editing state. */
function renderRenameFavoriteFolderDialog(state: FavoritesViewState) {
  const { editingId, editName, editInputRef, folders, renameMut, setEditName, setEditingId } =
    state;
  const editingFolder = folders.find((folder) => folder.id === editingId) ?? null;

  return (
    <Dialog
      open={editingFolder !== null}
      onOpenChange={(nextOpen) => {
        if (!nextOpen && !renameMut.isPending) {
          setEditingId(null);
        }
      }}
    >
      <DialogContent className="md:max-w-md">
        <DialogHeader>
          <DialogTitle>重命名收藏夹</DialogTitle>
          <DialogDescription>为“{editingFolder?.name ?? ''}”输入新的名称</DialogDescription>
        </DialogHeader>
        <form
          className="space-y-4"
          data-motion-folder-mode="edit"
          onSubmit={(event) => {
            event.preventDefault();
            if (editingFolder && editName.trim()) {
              renameMut.mutate({ id: editingFolder.id, name: editName.trim() });
            }
          }}
        >
          <Input
            ref={editInputRef}
            aria-label={`重命名收藏夹 ${editingFolder?.name ?? ''}`}
            name="favorite_folder_rename"
            autoComplete="off"
            value={editName}
            onChange={(event) => setEditName(event.target.value)}
          />
          <DialogFooter>
            <Button type="submit" disabled={renameMut.isPending || !editName.trim()}>
              {renameMut.isPending ? '保存中…' : '保存'}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}

/** Select the existing direct keyed article panel in its original priority. */
function renderFavoritesArticlePanel(state: FavoritesViewState) {
  const {
    favorites,
    favoritesError,
    hasNextPage,
    isFavoritesError,
    isFetchingNextPage,
    isPendingFavorites,
    loadMoreRef,
    loadedPages,
    refetchFavorites,
    visiblePageCount,
    stateTransition,
  } = state;
  if (isPendingFavorites)
    return (
      <MotionDiv
        key="favorites-loading"
        data-favorites-state="loading"
        aria-hidden="true"
        variants={FADE_UP_VARIANTS}
        initial="hidden"
        animate="visible"
        exit={{ opacity: 0, pointerEvents: 'none', y: -4 }}
        transition={stateTransition}
      >
        <ArticleListSkeleton count={4} />
      </MotionDiv>
    );
  if (isFavoritesError)
    return (
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
          description={favoritesError instanceof Error ? favoritesError.message : '请稍后重试。'}
          action={
            <Button variant="outline" size="sm" onClick={() => void refetchFavorites()}>
              重试
            </Button>
          }
        />
      </MotionDiv>
    );
  if (favorites.length === 0)
    return (
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
    );
  return (
    <MotionDiv
      key="favorites-results"
      data-favorites-state="results"
      variants={FADE_UP_VARIANTS}
      initial="hidden"
      animate="visible"
      exit={{ opacity: 0, pointerEvents: 'none', y: -4 }}
      transition={stateTransition}
    >
      {renderFavoriteBulkActions(state)}
      {renderFavoriteArticles(state)}
      {(visiblePageCount < loadedPages || hasNextPage) && <div ref={loadMoreRef} className="h-1" />}
      <MotionPresence>
        {isFetchingNextPage && (
          <MotionDiv
            key="favorites-next-page"
            aria-hidden="true"
            className="space-y-2 px-4 py-4 sm:px-5"
            variants={FADE_UP_VARIANTS}
            initial="hidden"
            animate="visible"
            exit={{ opacity: 0, pointerEvents: 'none', y: -2 }}
            transition={stateTransition}
          >
            <Skeleton className="h-4 w-11/12" />
            <Skeleton className="h-3 w-1/2" />
          </MotionDiv>
        )}
      </MotionPresence>
    </MotionDiv>
  );
}

/** Retain the complete article-state animation owner. */
function renderFavoritesArticleStates(state: FavoritesViewState) {
  return <MotionPresence mode="wait">{renderFavoritesArticlePanel(state)}</MotionPresence>;
}

/** Retain selected-folder and no-folder panels with their original keys. */
function renderFavoritesWorkspace(state: FavoritesViewState) {
  const { isLoading, selectedFolder, stateTransition, favoritesState, setDialogOpen } = state;

  return (
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
            description={isLoading ? '收藏夹加载完成后即可查看文章。' : '从上方选择或新建收藏夹。'}
            action={
              isLoading ? undefined : (
                <Button variant="outline" size="sm" onClick={() => setDialogOpen(true)}>
                  <FolderPlus aria-hidden="true" />
                  新建收藏夹
                </Button>
              )
            }
          />
        </MotionDiv>
      ) : (
        <MotionDiv
          key={`favorites-folder-${selectedFolder.id}`}
          data-favorites-folder={selectedFolder.id}
          variants={FADE_UP_VARIANTS}
          initial="hidden"
          animate="visible"
          exit={{ opacity: 0, pointerEvents: 'none', y: -4 }}
          transition={stateTransition}
        >
          {renderFavoritesArticleStates(state)}
        </MotionDiv>
      )}
    </MotionPresence>
  );
}

/** Preserve the single immediate favorites announcement and its ordered messages. */
function getFavoritesAnnouncement(state: FavoritesViewState): string {
  const { favoritesState, selectedFolder, favoritesError, isFetchingNextPage, favorites } = state;
  if (favoritesState === 'folders-loading') return '正在加载收藏夹';
  if (favoritesState === 'no-folder') return '请选择一个收藏夹查看文章';
  if (favoritesState === 'loading')
    return `正在加载“${getFavoriteAnnouncementFolderName(selectedFolder)}”中的收藏文章`;
  if (favoritesState === 'error')
    return `加载收藏文章失败：${favoritesError instanceof Error ? favoritesError.message : '未知错误'}`;
  if (favoritesState === 'empty')
    return `“${getFavoriteAnnouncementFolderName(selectedFolder)}”中暂无收藏文章`;
  if (isFetchingNextPage) return '正在加载更多收藏文章';
  return `已加载 ${favorites.length} 篇收藏文章`;
}

/** Read the existing nullable folder name only when its announcement branch is selected. */
function getFavoriteAnnouncementFolderName(folder: FavoritesViewState['selectedFolder']): string {
  return folder?.name ?? '';
}
/** Preserve every existing move-action admission guard. */
function isFavoriteBulkMoveDisabled(state: FavoritesViewState): boolean {
  return (
    state.selectedFavorites.length === 0 ||
    !state.effectiveMoveTargetFolderId ||
    state.bulkMoveMut.isPending ||
    state.moveTargetFolders.length === 0
  );
}
/** Preserve every existing removal-action admission guard. */
function isFavoriteBulkRemoveDisabled(state: FavoritesViewState): boolean {
  return state.selectedFavorites.length === 0 || state.bulkRemoveMut.isPending;
}
