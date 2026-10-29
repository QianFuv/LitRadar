'use client';

/**
 * Keyboard-accessible article rows that open in the docked reader or a detail sheet.
 */

import { useEffect, useState, type KeyboardEvent, type MouseEvent, type ReactNode } from 'react';
import { createPortal } from 'react-dom';

import {
  ArticleDetailDialogContent,
  ArticleDetailPane,
} from '@/components/feature/article-detail-dialog-content';
import { ArticleListCard } from '@/components/feature/article-list-card';
import {
  READER_ITEM_ATTRIBUTE,
  useArticleReader,
} from '@/components/feature/article-reader-context';
import { Dialog, DialogTrigger } from '@/components/ui/dialog';
import { type Article } from '@/lib/api';
import { getArticleDisplayTitle, hasArticleTitle } from '@/lib/article-title';
import { cn } from '@/lib/utils';

type ArticleDialogCardProps = {
  article: Article;
  dbName: string;
  title?: ReactNode;
  preview?: ReactNode;
  initialFolderIds?: number[];
  isFavoriteStatePending?: boolean;
  isFavoriteStateUnavailable?: boolean;
  /** Folder whose membership the favorite button toggles directly. */
  favoriteToggleFolderId?: number;
  extraActions?: ReactNode;
  leading?: ReactNode;
  triggerRef?: (node?: Element | null) => void;
  className?: string;
};

/**
 * Open the row with Enter or Space without scrolling the workspace.
 *
 * @param event - Keyboard input on the article trigger.
 */
function handleArticleTriggerKeyDown(event: KeyboardEvent<HTMLDivElement>): void {
  if (event.target === event.currentTarget && (event.key === 'Enter' || event.key === ' ')) {
    event.preventDefault();
    event.currentTarget.click();
  }
}

/**
 * Preserve text selection when a pointer gesture ends inside the article row.
 *
 * @param event - Pointer-generated click on the article trigger.
 */
function preserveArticleTextSelection(event: MouseEvent<HTMLDivElement>): void {
  const selection = window.getSelection();
  if (
    event.detail > 0 &&
    selection &&
    !selection.isCollapsed &&
    (event.currentTarget.contains(selection.anchorNode) ||
      event.currentTarget.contains(selection.focusNode))
  ) {
    event.preventDefault();
  }
}

/**
 * Render a selectable article row that fills the docked reader or opens a detail sheet.
 *
 * @param props - Article row and detail configuration.
 * @returns Article row with its detail surface.
 */
export function ArticleDialogCard({
  article,
  dbName,
  title,
  preview,
  initialFolderIds = [],
  isFavoriteStatePending = false,
  isFavoriteStateUnavailable = false,
  favoriteToggleFolderId,
  extraActions,
  leading,
  triggerRef,
  className,
}: ArticleDialogCardProps) {
  const reader = useArticleReader();
  const [open, setOpen] = useState(false);
  const itemKey = `${dbName}::${article.article_id}`;
  const readerNode = reader?.isDocked ? reader.readerNode : null;
  const isDocked = readerNode !== null;
  const isSelected = isDocked && reader?.selectedKey === itemKey;
  const release = reader?.release;
  const displayTitle = getArticleDisplayTitle(article);
  const resolvedTitle = hasArticleTitle(article) ? (title ?? displayTitle) : displayTitle;
  const resolvedPreview = preview ?? article.abstract;
  const detailProps = {
    article,
    dbName,
    initialFolderIds,
    isFavoriteStatePending,
    isFavoriteStateUnavailable,
    favoriteToggleFolderId,
    extraActions,
  };

  useEffect(() => () => release?.(itemKey), [itemKey, release]);

  /**
   * Select the row in the docked reader, or let the dialog trigger open the sheet.
   *
   * @param event - Click on the row trigger.
   */
  function handleClick(event: MouseEvent<HTMLDivElement>): void {
    preserveArticleTextSelection(event);
    if (event.defaultPrevented || !isDocked || !reader) {
      return;
    }
    event.preventDefault();
    reader.select(itemKey);
  }

  return (
    <Dialog open={open && !isDocked} onOpenChange={setOpen}>
      <div
        data-selected={isSelected ? 'true' : 'false'}
        className={cn(
          'motion-control group/article relative flex items-start border-b border-border/60 transition-[background-color] hover:bg-accent/50 data-[selected=true]:bg-accent',
          className,
        )}
      >
        {leading && <div className="shrink-0 pt-[22px] pl-5 sm:pl-7">{leading}</div>}
        <DialogTrigger asChild>
          <div
            ref={triggerRef}
            role="button"
            tabIndex={0}
            {...{ [READER_ITEM_ATTRIBUTE]: '' }}
            data-selected={isSelected ? 'true' : 'false'}
            aria-current={isSelected ? 'true' : undefined}
            aria-label={`查看文章详情：${displayTitle}`}
            className="min-w-0 flex-1 cursor-pointer outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-inset"
            onClick={handleClick}
            onKeyDown={handleArticleTriggerKeyDown}
          >
            <ArticleListCard
              title={resolvedTitle}
              authors={article.authors}
              journalTitle={article.journal_title}
              volume={article.volume}
              number={article.number}
              date={article.date}
              preview={resolvedPreview}
              openAccess={article.open_access}
              inPress={article.in_press}
              className={leading ? 'pl-3 sm:pl-3' : undefined}
            />
          </div>
        </DialogTrigger>
      </div>
      {open && !isDocked && <ArticleDetailDialogContent {...detailProps} />}
      {isSelected && readerNode && createPortal(<ArticleDetailPane {...detailProps} />, readerNode)}
    </Dialog>
  );
}
