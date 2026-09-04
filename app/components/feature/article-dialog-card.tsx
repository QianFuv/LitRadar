'use client';

/**
 * Keyboard-accessible article cards that open a shared detail dialog.
 */

import { useState, type KeyboardEvent, type MouseEvent, type ReactNode } from 'react';

import { ArticleDetailDialogContent } from '@/components/feature/article-detail-dialog-content';
import { ArticleListCard } from '@/components/feature/article-list-card';
import { Dialog, DialogTrigger } from '@/components/ui/dialog';
import { type Article } from '@/lib/api';
import { cn } from '@/lib/utils';

type ArticleDialogCardProps = {
  article: Article;
  dbName: string;
  title?: ReactNode;
  preview?: ReactNode;
  initialFolderIds?: number[];
  isFavoriteStatePending?: boolean;
  extraActions?: ReactNode;
  leading?: ReactNode;
  triggerRef?: (node?: Element | null) => void;
  className?: string;
};

/**
 * Open the card with Enter or Space without scrolling the workspace.
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
 * Preserve text selection when a pointer gesture ends inside the article card.
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
 * Render a selectable article card that opens its detail dialog from the whole surface.
 *
 * @param props - Article card and dialog configuration.
 * @returns Article card and lazily mounted detail dialog.
 */
export function ArticleDialogCard({
  article,
  dbName,
  title,
  preview,
  initialFolderIds = [],
  isFavoriteStatePending = false,
  extraActions,
  leading,
  triggerRef,
  className,
}: ArticleDialogCardProps) {
  const [open, setOpen] = useState(false);
  const resolvedTitle = title ?? article.title ?? `文章 #${article.article_id}`;
  const resolvedPreview = preview ?? article.abstract;

  return (
    <Dialog open={open} onOpenChange={setOpen}>
      <div className={cn('flex items-start gap-3', className)}>
        {leading && <div className="pt-4">{leading}</div>}
        <DialogTrigger asChild>
          <div
            ref={triggerRef}
            role="button"
            tabIndex={0}
            aria-label={`查看文章详情：${article.title || '未命名文章'}`}
            className="min-w-0 flex-1 cursor-pointer rounded-lg outline-none focus-visible:ring-[3px] focus-visible:ring-ring/50"
            onClick={preserveArticleTextSelection}
            onKeyDown={handleArticleTriggerKeyDown}
          >
            <ArticleListCard
              title={resolvedTitle}
              journalTitle={article.journal_title}
              volume={article.volume}
              number={article.number}
              date={article.date}
              preview={resolvedPreview}
              openAccess={article.open_access}
              inPress={article.in_press}
            />
          </div>
        </DialogTrigger>
      </div>
      {open && (
        <ArticleDetailDialogContent
          article={article}
          dbName={dbName}
          initialFolderIds={initialFolderIds}
          isFavoriteStatePending={isFavoriteStatePending}
          extraActions={extraActions}
        />
      )}
    </Dialog>
  );
}
