'use client';

import { useState, type ReactNode } from 'react';

import { ArticleDetailDialogContent } from '@/components/feature/article-detail-dialog-content';
import { ArticleListCard } from '@/components/feature/article-list-card';
import { Button } from '@/components/ui/button';
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
 * Render a selectable article card with an explicit detail-dialog trigger.
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
        <ArticleListCard
          className="min-w-0 flex-1"
          title={resolvedTitle}
          journalTitle={article.journal_title}
          volume={article.volume}
          number={article.number}
          date={article.date}
          preview={resolvedPreview}
          openAccess={article.open_access}
          inPress={article.in_press}
          action={
            <DialogTrigger asChild>
              <Button ref={triggerRef} type="button" variant="outline" size="sm">
                查看详情
              </Button>
            </DialogTrigger>
          }
        />
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
