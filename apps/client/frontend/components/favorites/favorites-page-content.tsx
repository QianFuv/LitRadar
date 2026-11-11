/**
 * Favorites workspace: folder tabs, selection bar, article rows, and the reading desk.
 */

import { useState, type Dispatch, type SetStateAction } from 'react';
import {
  Download,
  MoreHorizontal,
  Pencil,
  Plus,
  Trash2,
  Radar,
} from '@/components/ui/animated-icons';

import type { CitationFormat, FavoriteArticleItem } from '@/lib/api';
import { ArticleDialogCard } from '@/components/feature/article-dialog-card';
import { ArticleListSkeleton } from '@/components/feature/article-list-card';
import { ReaderPlaceholder } from '@/components/feature/reader-placeholder';
import { WorkspaceShell } from '@/components/feature/workspace-shell';
import { FavoriteKindSwitch } from '@/components/favorites/favorite-kind-switch';
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
  LIST_ITEM_VARIANTS,
  MotionDiv,
  MotionParagraph,
  MotionPresence,
  PANEL_VARIANTS,
  REVEAL_VARIANTS,
} from '@/components/ui/motion';
import { Popover, PopoverContent, PopoverTrigger } from '@/components/ui/popover';
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@/components/ui/select';
import { RevealItem } from '@/components/ui/reveal';
import { attachHorizontalWheelScroll } from '@/lib/horizontal-wheel-scroll';
import { Highlight, HighlightItem } from '@animate-ui/react/primitives/effects/highlight';
import { ScrollFade } from '@animate-ui/react/primitives/effects/scroll-fade';
import { Skeleton } from '@/components/ui/skeleton';
import { StateMessage } from '@/components/ui/state-message';
import * as stylex from '@stylexjs/stylex';
import { colors, lineHeights } from '@/lib/theme.stylex';

/** Citation export formats offered in the export popover. */
const CITATION_FORMAT_ITEMS = [
  { value: 'bibtex', label: 'BibTeX' },
  { value: 'ris', label: 'RIS' },
  { value: 'endnote', label: 'EndNote XML' },
] as const satisfies readonly { value: CitationFormat; label: string }[];

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
  return (
    <>
      {feedback && (
        <p
          key={`${scope}-${feedback.tone}-${feedback.message}`}
          data-testid={`${scope}-feedback-announcement`}
          {...stylex.props(styles.animatedFeedbackP)}
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
            {...stylex.props(
              styles.animatedFeedbackMotionParagraph,
              feedback.tone === 'error'
                ? styles.animatedFeedbackMotionParagraphError
                : styles.animatedFeedbackMotionParagraphNotError,
            )}
            variants={REVEAL_VARIANTS}
            initial="hidden"
            animate="visible"
            exit="exit"
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
 * @returns Favorites feature UI.
 */
export function FavoritesPageContent() {
  const page = useFavoritesPage();
  const [folderToDelete, setFolderToDelete] = useState<{ id: number; name: string } | null>(null);
  const favoritesState = getFavoritesState(page);
  const state = {
    ...page,
    folderToDelete,
    setFolderToDelete,
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
          {...stylex.props(styles.animatedFeedbackP)}
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
    <div {...stylex.props(styles.renderFavoritesHeaderDiv)}>
      <div {...stylex.props(styles.renderFavoritesHeaderDiv2)}>
        <div {...stylex.props(styles.renderFavoritesHeaderDiv3)}>
          <h1 {...stylex.props(styles.renderFavoritesHeaderH1)}>我的收藏</h1>
          <p {...stylex.props(styles.renderFavoritesHeaderP)}>按收藏夹整理、批量管理并导出引用</p>
        </div>
        <Button
          variant="outline"
          size="sm"
          xstyle={styles.renderFavoritesHeaderButton}
          aria-label="新建收藏夹"
          onClick={() => setDialogOpen(true)}
        >
          <Plus aria-hidden="true" />
          <span {...stylex.props(styles.renderFavoritesHeaderSpan)}>新建</span>
        </Button>
      </div>
      <FavoriteKindSwitch />
      {renderFolderTabs(state)}
      <RevealItem isReady={!state.isLoading} placeholder={null}>
        {selectedFolder && renderFolderToolbar(state, selectedFolder)}
      </RevealItem>
    </div>
  );
}

/** Render folders as a horizontally scrolling row of tabs. */
function renderFolderTabs(state: FavoritesViewState) {
  const { folders, isLoading, activeFolderId, handleSelectFolder } = state;

  return (
    <RevealItem
      isReady={!isLoading}
      placeholder={
        <div aria-hidden="true" {...stylex.props(styles.renderFolderTabsDiv)}>
          <Skeleton xstyle={styles.renderFolderTabsSkeleton} />
          <Skeleton xstyle={styles.renderFolderTabsSkeleton2} />
          <Skeleton xstyle={styles.renderFolderTabsSkeleton3} />
        </div>
      }
    >
      {folders.length === 0 ? (
        <p {...stylex.props(styles.renderFavoritesHeaderP)}>暂无收藏夹，点击「新建」创建第一个。</p>
      ) : (
        <fieldset aria-label="收藏夹" {...stylex.props(styles.layoutFreeGroup)}>
          <ScrollFade orientation="horizontal" shrink={0} asChild>
            <div
              ref={attachHorizontalWheelScroll}
              data-slot="favorite-folder-tabs"
              {...stylex.props(styles.favoriteFolderTabs)}
            >
              <Highlight
                controlledItems
                value={activeFolderId === null ? null : String(activeFolderId)}
                click={false}
                {...stylex.props(styles.renderFolderTabsHighlight)}
              >
                {folders.map((folder) => {
                  const isActive = activeFolderId === folder.id;
                  return (
                    <HighlightItem
                      key={folder.id}
                      value={String(folder.id)}
                      {...stylex.props(styles.renderFolderTabsHighlightItem)}
                    >
                      <button
                        type="button"
                        data-motion-folder-key={folder.id}
                        aria-pressed={isActive}
                        title={folder.is_tracking ? `${folder.name}（追踪文件夹）` : folder.name}
                        onClick={() => handleSelectFolder(folder.id)}
                        {...stylex.props(
                          styles.renderFolderTabsButton,
                          isActive
                            ? styles.renderFolderTabsButtonActive
                            : styles.renderFolderTabsButtonNotActive,
                        )}
                      >
                        {folder.is_tracking && (
                          <Radar
                            xstyle={[
                              styles.renderFolderTabsRadar,
                              isActive
                                ? styles.renderFolderTabsRadarActive
                                : styles.renderFolderTabsRadarNotActive,
                            ]}
                            aria-hidden="true"
                          />
                        )}
                        <span {...stylex.props(styles.renderFolderTabsSpan)}>{folder.name}</span>
                        <span
                          aria-hidden="true"
                          {...stylex.props(
                            styles.renderFolderTabsSpan2,
                            isActive
                              ? styles.renderFolderTabsSpanActive
                              : styles.renderFolderTabsSpanNotActive,
                          )}
                        >
                          {folder.article_count}
                        </span>
                      </button>
                    </HighlightItem>
                  );
                })}
              </Highlight>
            </div>
          </ScrollFade>
        </fieldset>
      )}
    </RevealItem>
  );
}

/** Render current-folder actions: export, tracking, rename, and delete. */
function renderFolderToolbar(
  state: FavoritesViewState,
  selectedFolder: NonNullable<FavoritesViewState['selectedFolder']>,
) {
  const { deleteMut, trackMut, setEditingId, setEditName, setFolderToDelete } = state;

  return (
    <div {...stylex.props(styles.renderFolderToolbarDiv)}>
      <div {...stylex.props(styles.renderFavoritesHeaderDiv3)}>
        <p {...stylex.props(styles.renderFolderToolbarP)}>
          <span {...stylex.props(styles.renderFolderTabsSpan)} title={selectedFolder.name}>
            {selectedFolder.name}
          </span>
          {selectedFolder.is_tracking && (
            <Badge xstyle={styles.renderFolderTabsHighlightItem}>追踪</Badge>
          )}
        </p>
        <p {...stylex.props(styles.renderFolderToolbarP2)}>
          {selectedFolder.article_count} 篇
          {selectedFolder.is_tracking && ' · 每周推送会自动收藏到此处'}
        </p>
      </div>
      <div {...stylex.props(styles.renderFolderToolbarDiv2)}>
        {renderFavoriteExport(state, selectedFolder)}
        <DropdownMenu>
          <DropdownMenuTrigger
            render={
              <Button
                variant="ghost"
                size="icon-sm"
                aria-label={`管理收藏夹 ${selectedFolder.name}`}
                title="管理收藏夹"
              />
            }
          >
            <MoreHorizontal aria-hidden="true" />
          </DropdownMenuTrigger>
          <DropdownMenuContent align="end" xstyle={styles.renderFolderToolbarDropdownMenuContent}>
            <DropdownMenuLabel xstyle={styles.renderFolderTabsSpan}>
              {selectedFolder.name}
            </DropdownMenuLabel>
            <DropdownMenuItem
              aria-label={`设 ${selectedFolder.name} 为追踪文件夹`}
              disabled={selectedFolder.is_tracking}
              onClick={() => trackMut.mutate(selectedFolder.id)}
            >
              <Radar />
              {selectedFolder.is_tracking ? '已是追踪文件夹' : '设为追踪文件夹'}
            </DropdownMenuItem>
            <DropdownMenuItem
              aria-label={`重命名收藏夹 ${selectedFolder.name}`}
              onClick={() => {
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
              onClick={() => {
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
  const { prefetchIndex, prefetchRef, selectedKeySet, toggleFavoriteSelection } = state;

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
      variants={LIST_ITEM_VARIANTS}
      initial="hidden"
      animate="visible"
      exit="exit"
    >
      <ArticleDialogCard
        triggerRef={index === prefetchIndex ? prefetchRef : undefined}
        article={favorite}
        dbName={favorite.db_name}
        initialFolderIds={[favorite.folder_id]}
        favoriteToggleFolderId={favorite.folder_id}
        xstyle={isSelected && styles.renderFavoriteArticleArticleDialogCardSelected}
        preview={
          favorite.metadata_status === 'missing' ? (
            <span {...stylex.props(styles.renderFavoriteArticleSpan)}>
              来源数据库或文章已不存在。收藏仍保留；可移动、移除，导出时会保留空元数据条目。
            </span>
          ) : favorite.metadata_status === 'unavailable' ? (
            <span {...stylex.props(styles.animatedFeedbackMotionParagraphError)}>
              文章元数据暂时无法读取，请稍后重试。收藏仍保留，可移动或移除；当前导出会明确报错。
            </span>
          ) : undefined
        }
        leading={
          <Checkbox
            checked={isSelected}
            onCheckedChange={(checked) => toggleFavoriteSelection(favorite, checked)}
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
      <PopoverTrigger
        render={
          <Button
            variant="ghost"
            size="sm"
            xstyle={styles.renderFavoriteExportButton}
            aria-label="导出收藏夹"
          />
        }
      >
        <Download aria-hidden="true" />
        <span {...stylex.props(styles.renderFavoritesHeaderSpan)}>导出</span>
      </PopoverTrigger>
      <PopoverContent
        align="end"
        xstyle={styles.renderFavoriteExportPopoverContent}
        aria-label="导出引用"
      >
        <div>
          <p {...stylex.props(styles.renderFavoriteExportP)}>导出引用</p>
          <p {...stylex.props(styles.renderFavoriteExportP2)}>
            导出当前收藏夹为 BibTeX、RIS 或 EndNote 格式
          </p>
        </div>
        <Select
          value={exportFormat}
          items={CITATION_FORMAT_ITEMS}
          onValueChange={setExportFormat}
          disabled={exportMut.isPending}
        >
          <SelectTrigger xstyle={styles.renderFavoriteExportSelectTrigger} aria-label="导出格式">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            {CITATION_FORMAT_ITEMS.map((item) => (
              <SelectItem key={item.value} value={item.value}>
                {item.label}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
        <Button
          type="button"
          xstyle={styles.renderFavoriteExportSelectTrigger}
          disabled={exportMut.isPending}
          onClick={() =>
            exportMut.mutate({
              folderId: selectedFolder.id,
              format: exportFormat,
            })
          }
        >
          <Download xstyle={styles.renderFavoriteExportDownload} aria-hidden="true" />
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
      {...stylex.props(
        styles.renderFavoriteBulkActionsSection,
        hasSelection && styles.renderFavoriteBulkActionsSectionSelection,
      )}
    >
      <div {...stylex.props(styles.renderFavoriteBulkActionsDiv)}>
        <div {...stylex.props(styles.renderFavoriteBulkActionsDiv2)}>
          <Checkbox
            checked={allLoadedSelected}
            indeterminate={!allLoadedSelected && hasSelection}
            onCheckedChange={handleSelectAllLoaded}
            aria-label="选择当前已加载文章"
          />
          <div {...stylex.props(styles.renderFavoriteBulkActionsDiv3)}>
            <p {...stylex.props(styles.renderFavoriteBulkActionsP)}>
              已选 {selectedFavorites.length} 篇
            </p>
            <p {...stylex.props(styles.renderFolderToolbarP2)}>当前列表共 {favorites.length} 篇</p>
          </div>
        </div>
        <div {...stylex.props(styles.renderFavoriteBulkActionsDiv4)}>
          <Button
            variant="ghost"
            size="sm"
            xstyle={styles.renderFavoriteBulkActionsButton}
            onClick={() => handleSelectAllLoaded(true)}
            disabled={favorites.length === 0 || allLoadedSelected}
          >
            全选当前列表
          </Button>
          <Button
            variant="ghost"
            size="sm"
            xstyle={styles.renderFavoriteBulkActionsButton}
            onClick={() => handleSelectAllLoaded(false)}
            disabled={!hasSelection}
          >
            清空选择
          </Button>
        </div>
        <fieldset
          aria-label="批量操作"
          {...stylex.props(
            styles.groupFieldset,
            styles.renderFavoriteBulkActionsDiv5,
            !hasSelection && styles.renderFavoriteBulkActionsDivNoSelection,
          )}
        >
          <Select
            value={effectiveMoveTargetFolderId}
            items={moveTargetFolders.map((folder) => ({
              value: String(folder.id),
              label: folder.name,
            }))}
            onValueChange={setMoveTargetFolderId}
          >
            <SelectTrigger
              size="sm"
              xstyle={styles.renderFavoriteBulkActionsSelectTrigger}
              aria-label="选择目标收藏夹"
            >
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
            xstyle={styles.bulkRemoveButton}
            onClick={handleBulkRemove}
            disabled={isFavoriteBulkRemoveDisabled(state)}
          >
            {bulkRemoveMut.isPending ? '删除中…' : '删除所选'}
          </Button>
        </fieldset>
      </div>
      {(hasNextPage || visiblePageCount < loadedPages) && (
        <p {...stylex.props(styles.renderFavoriteBulkActionsP2)}>
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
      <DialogContent xstyle={styles.renderCreateFavoriteFolderDialogDialogContent}>
        <DialogHeader>
          <DialogTitle>新建收藏夹</DialogTitle>
          <DialogDescription>输入收藏夹名称</DialogDescription>
        </DialogHeader>
        <form
          onSubmit={(event) => {
            event.preventDefault();
            if (newFolderName.trim()) createMut.mutate(newFolderName.trim());
          }}
        >
          <Input
            xstyle={styles.renderCreateFavoriteFolderDialogFormItem}
            aria-label="收藏夹名称"
            name="favorite_folder_name"
            autoComplete="off"
            value={newFolderName}
            onChange={(event) => setNewFolderName(event.target.value)}
            placeholder="收藏夹名称"
          />
          <DialogFooter xstyle={styles.renderCreateFavoriteFolderDialogFormItem}>
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
      <DialogContent xstyle={styles.renderCreateFavoriteFolderDialogDialogContent}>
        <DialogHeader>
          <DialogTitle>重命名收藏夹</DialogTitle>
          <DialogDescription>为“{editingFolder?.name ?? ''}”输入新的名称</DialogDescription>
        </DialogHeader>
        <form
          data-motion-folder-mode="edit"
          onSubmit={(event) => {
            event.preventDefault();
            if (editingFolder && editName.trim()) {
              renameMut.mutate({ id: editingFolder.id, name: editName.trim() });
            }
          }}
        >
          <Input
            xstyle={styles.renderCreateFavoriteFolderDialogFormItem}
            ref={editInputRef}
            aria-label={`重命名收藏夹 ${editingFolder?.name ?? ''}`}
            name="favorite_folder_rename"
            autoComplete="off"
            value={editName}
            onChange={(event) => setEditName(event.target.value)}
          />
          <DialogFooter xstyle={styles.renderCreateFavoriteFolderDialogFormItem}>
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
  } = state;
  if (isPendingFavorites) return null;
  if (isFavoritesError)
    return (
      <MotionDiv
        key="favorites-error"
        data-favorites-state="error"
        variants={PANEL_VARIANTS}
        initial="hidden"
        animate="visible"
        exit="exit"
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
        variants={PANEL_VARIANTS}
        initial="hidden"
        animate="visible"
        exit="exit"
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
      variants={PANEL_VARIANTS}
      initial="hidden"
      animate="visible"
      exit="exit"
    >
      {renderFavoriteBulkActions(state)}
      {renderFavoriteArticles(state)}
      {(visiblePageCount < loadedPages || hasNextPage) && (
        <div ref={loadMoreRef} {...stylex.props(styles.renderFavoritesArticlePanelDiv)} />
      )}
      <MotionPresence>
        {isFetchingNextPage && (
          <MotionDiv
            key="favorites-next-page"
            aria-hidden="true"
            {...stylex.props(styles.renderFavoritesArticlePanelMotionDiv)}
            variants={REVEAL_VARIANTS}
            initial="hidden"
            animate="visible"
            exit="exit"
          >
            <Skeleton
              xstyle={[
                styles.renderFavoritesArticlePanelMotionDivItem,
                styles.renderFavoritesArticlePanelSkeleton,
              ]}
            />
            <Skeleton
              xstyle={[
                styles.renderFavoritesArticlePanelMotionDivItem,
                styles.renderFavoritesArticlePanelSkeleton2,
              ]}
            />
          </MotionDiv>
        )}
      </MotionPresence>
    </MotionDiv>
  );
}

/** Retain the complete article-state animation owner. */
function renderFavoritesArticleStates(state: FavoritesViewState) {
  return (
    <RevealItem
      isReady={!state.isPendingFavorites}
      placeholder={
        <div data-favorites-state="loading" aria-hidden="true">
          <ArticleListSkeleton count={4} />
        </div>
      }
    >
      <MotionPresence mode="wait">{renderFavoritesArticlePanel(state)}</MotionPresence>
    </RevealItem>
  );
}

/** Retain selected-folder and no-folder panels with their original keys. */
function renderFavoritesWorkspace(state: FavoritesViewState) {
  const { isLoading, selectedFolder, favoritesState, setDialogOpen } = state;

  return (
    <RevealItem
      isReady={!isLoading}
      placeholder={
        <div data-favorites-state="folders-loading">
          <StateMessage
            isLive={false}
            title="正在加载收藏夹"
            description="收藏夹加载完成后即可查看文章。"
          />
        </div>
      }
    >
      <MotionPresence mode="wait">
        {!selectedFolder ? (
          <MotionDiv
            key="favorites-no-folder"
            data-favorites-state={favoritesState}
            variants={PANEL_VARIANTS}
            initial="hidden"
            animate="visible"
            exit="exit"
          >
            <StateMessage
              isLive={false}
              title="选择一个收藏夹"
              description="从上方选择或新建收藏夹。"
              action={
                <Button variant="outline" size="sm" onClick={() => setDialogOpen(true)}>
                  <Plus aria-hidden="true" />
                  新建收藏夹
                </Button>
              }
            />
          </MotionDiv>
        ) : (
          <MotionDiv
            key={`favorites-folder-${selectedFolder.id}`}
            data-favorites-folder={selectedFolder.id}
            variants={PANEL_VARIANTS}
            initial="hidden"
            animate="visible"
            exit="exit"
          >
            {renderFavoritesArticleStates(state)}
          </MotionDiv>
        )}
      </MotionPresence>
    </RevealItem>
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

const styles = stylex.create({
  layoutFreeGroup: {
    display: 'contents',
  },
  groupFieldset: {
    minInlineSize: 0,
  },
  bulkRemoveButton: {
    color: {
      default: colors.destructive,
      ':hover': {
        default: null,
        '@media (hover: hover)': colors.accentForeground,
      },
    },
  },
  animatedFeedbackP: {
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
  animatedFeedbackMotionParagraph: {
    fontSize: '0.75rem',
    lineHeight: '1.25rem',
  },
  animatedFeedbackMotionParagraphError: {
    color: colors.destructive,
  },
  animatedFeedbackMotionParagraphNotError: {
    color: colors.successForeground,
  },
  renderFavoritesHeaderDiv: {
    display: 'flex',
    flexDirection: 'column',
    rowGap: '0.75rem',
  },
  renderFavoritesHeaderDiv2: {
    display: 'flex',
    alignItems: 'center',
    justifyContent: 'space-between',
    gap: '0.75rem',
  },
  renderFavoritesHeaderDiv3: {
    minWidth: '0px',
  },
  renderFavoritesHeaderH1: {
    fontSize: '1.125rem',
    lineHeight: '1.75rem',
    fontWeight: 700,
  },
  renderFavoritesHeaderP: {
    fontSize: '0.75rem',
    lineHeight: lineHeights.textXs,
    color: colors.mutedForeground,
  },
  renderFavoritesHeaderButton: {
    flexShrink: 0,
    borderRadius: '8px',
  },
  renderFavoritesHeaderSpan: {
    display: {
      default: 'none',
      '@media (min-width: 40rem)': 'inline',
    },
  },
  renderFolderTabsDiv: {
    display: 'flex',
    gap: '0.5rem',
  },
  renderFolderTabsSkeleton: {
    height: '2rem',
    width: '6rem',
    borderRadius: 'calc(infinity * 1px)',
  },
  renderFolderTabsSkeleton2: {
    height: '2rem',
    width: '8rem',
    borderRadius: 'calc(infinity * 1px)',
  },
  renderFolderTabsSkeleton3: {
    height: '2rem',
    width: '5rem',
    borderRadius: 'calc(infinity * 1px)',
  },
  favoriteFolderTabs: {
    marginInline: {
      default: '-1rem',
      '@media (min-width: 40rem)': '-1.25rem',
    },
    marginBlock: '-0.25rem',
    display: 'flex',
    gap: '0.375rem',
    overflowX: 'auto',
    paddingInline: {
      default: '1rem',
      '@media (min-width: 40rem)': '1.25rem',
    },
    paddingBlock: '0.25rem',
    scrollbarWidth: 'none',
    '::-webkit-scrollbar': {
      display: 'none',
    },
  },
  renderFolderTabsHighlight: {
    borderRadius: 'calc(infinity * 1px)',
    backgroundColor: colors.foreground,
  },
  renderFolderTabsHighlightItem: {
    flexShrink: 0,
  },
  renderFolderTabsButton: {
    display: 'inline-flex',
    height: {
      default: '2.75rem',
      '@media (min-width: 48rem)': '2rem',
    },
    maxWidth: '13rem',
    flexShrink: 0,
    alignItems: 'center',
    gap: '0.375rem',
    borderRadius: 'calc(infinity * 1px)',
    paddingInline: {
      default: '0.875rem',
      '@media (min-width: 48rem)': '0.75rem',
    },
    fontSize: '13px',
    transitionProperty: 'background-color, color, box-shadow',
    transitionTimingFunction: 'cubic-bezier(0.25, 0.1, 0.25, 1)',
    transitionDuration: '200ms',
    outlineStyle: 'none',
    boxShadow: {
      default: null,
      ':focus-visible': `0 0 0 2px ${colors.ring}`,
    },
  },
  renderFolderTabsButtonActive: {
    fontWeight: 700,
    color: colors.background,
  },
  renderFolderTabsButtonNotActive: {
    backgroundColor: {
      default: colors.background,
      ':hover': {
        default: null,
        '@media (hover: hover)': colors.accent,
      },
    },
    color: `color-mix(in oklab, ${colors.foreground} 80%, transparent)`,
    boxShadow: {
      default: `inset 0 0 0 1px ${colors.border}`,
      ':focus-visible': `0 0 0 2px ${colors.ring}, inset 0 0 0 1px ${colors.border}`,
    },
  },
  renderFolderTabsRadar: {
    width: '0.875rem',
    height: '0.875rem',
    flexShrink: 0,
  },
  renderFolderTabsRadarActive: {
    color: colors.background,
  },
  renderFolderTabsRadarNotActive: {
    color: colors.primaryText,
  },
  renderFolderTabsSpan: {
    overflow: 'hidden',
    textOverflow: 'ellipsis',
    whiteSpace: 'nowrap',
  },
  renderFolderTabsSpan2: {
    flexShrink: 0,
    fontSize: '11px',
    fontVariantNumeric: 'tabular-nums',
  },
  renderFolderTabsSpanActive: {
    color: `color-mix(in oklab, ${colors.background} 70%, transparent)`,
  },
  renderFolderTabsSpanNotActive: {
    color: colors.mutedForeground,
  },
  renderFolderToolbarDiv: {
    display: 'flex',
    alignItems: 'center',
    justifyContent: 'space-between',
    gap: '0.5rem',
    borderRadius: '12px',
    backgroundColor: colors.muted,
    paddingInline: '0.75rem',
    paddingBlock: '0.5rem',
  },
  renderFolderToolbarP: {
    display: 'flex',
    minWidth: '0px',
    alignItems: 'center',
    gap: '0.375rem',
    fontSize: '13px',
    fontWeight: 700,
  },
  renderFolderToolbarP2: {
    fontSize: '11px',
    color: colors.mutedForeground,
  },
  renderFolderToolbarDiv2: {
    display: 'flex',
    flexShrink: 0,
    alignItems: 'center',
    gap: '0.25rem',
  },
  renderFolderToolbarDropdownMenuContent: {
    width: '14rem',
  },
  renderFavoriteArticleArticleDialogCardSelected: {
    backgroundColor: `color-mix(in oklab, ${colors.accent} 50%, transparent)`,
  },
  renderFavoriteArticleSpan: {
    color: colors.warningForeground,
  },
  renderFavoriteExportButton: {
    height: '2rem',
    paddingInline: '0.625rem',
  },
  renderFavoriteExportPopoverContent: {
    display: 'flex',
    width: '18rem',
    flexDirection: 'column',
    rowGap: '0.75rem',
  },
  renderFavoriteExportP: {
    fontSize: '0.875rem',
    lineHeight: lineHeights.textSm,
    fontWeight: 700,
  },
  renderFavoriteExportP2: {
    marginTop: '0.125rem',
    fontSize: '0.75rem',
    lineHeight: lineHeights.textXs,
    color: colors.mutedForeground,
  },
  renderFavoriteExportSelectTrigger: {
    width: '100%',
  },
  renderFavoriteExportDownload: {
    height: '1rem',
    width: '1rem',
  },
  renderFavoriteBulkActionsSection: {
    position: 'sticky',
    top: '0px',
    zIndex: 10,
    borderBottomStyle: 'solid',
    borderBottomWidth: '1px',
    backgroundColor: colors.background,
    paddingInline: {
      default: '1rem',
      '@media (min-width: 40rem)': '1.25rem',
    },
    paddingBlock: '0.625rem',
  },
  renderFavoriteBulkActionsSectionSelection: {
    backgroundColor: `color-mix(in oklab, ${colors.info} 80%, transparent)`,
  },
  renderFavoriteBulkActionsDiv: {
    display: 'flex',
    flexWrap: 'wrap',
    alignItems: 'center',
    columnGap: '0.75rem',
    rowGap: '0.5rem',
  },
  renderFavoriteBulkActionsDiv2: {
    display: 'flex',
    minWidth: '0px',
    alignItems: 'center',
    gap: '0.625rem',
  },
  renderFavoriteBulkActionsDiv3: {
    minWidth: '0px',
    lineHeight: 1.25,
  },
  renderFavoriteBulkActionsP: {
    fontSize: '13px',
    fontWeight: 700,
  },
  renderFavoriteBulkActionsDiv4: {
    display: 'flex',
    alignItems: 'center',
    gap: '0.25rem',
  },
  renderFavoriteBulkActionsButton: {
    height: '2rem',
    paddingInline: {
      default: '0.5rem',
      ':has( > svg)': '0.625rem',
    },
  },
  renderFavoriteBulkActionsDiv5: {
    display: 'flex',
    width: '100%',
    alignItems: 'center',
    gap: '0.375rem',
  },
  renderFavoriteBulkActionsDivNoSelection: {
    display: 'none',
  },
  renderFavoriteBulkActionsSelectTrigger: {
    minWidth: '0px',
    flex: '1',
  },
  renderFavoriteBulkActionsP2: {
    marginTop: '0.25rem',
    fontSize: '11px',
    color: colors.mutedForeground,
  },
  renderCreateFavoriteFolderDialogDialogContent: {
    maxWidth: {
      default: 'none',
      '@media (min-width: 48rem)': '28rem',
    },
  },
  renderCreateFavoriteFolderDialogFormItem: {
    marginBottom: {
      default: null,
      ':not(:last-child)': '1rem',
    },
  },
  renderFavoritesArticlePanelDiv: {
    height: '0.25rem',
  },
  renderFavoritesArticlePanelMotionDivItem: {
    marginBottom: {
      default: null,
      ':not(:last-child)': '0.5rem',
    },
  },
  renderFavoritesArticlePanelMotionDiv: {
    paddingInline: {
      default: '1rem',
      '@media (min-width: 40rem)': '1.25rem',
    },
    paddingBlock: '1rem',
  },
  renderFavoritesArticlePanelSkeleton: {
    height: '1rem',
    width: 'calc(11 / 12 * 100%)',
  },
  renderFavoritesArticlePanelSkeleton2: {
    height: '0.75rem',
    width: 'calc(1 / 2 * 100%)',
  },
});
