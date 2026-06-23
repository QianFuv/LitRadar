'use client';

import { useState, type ReactNode } from 'react';

import { ArticleDetailDialogContent } from '@/components/feature/article-detail-dialog-content';
import { ArticleListCard } from '@/components/feature/article-list-card';
import { Dialog, DialogTrigger } from '@/components/ui/dialog';
import { type JournalId } from '@/lib/api';
import { cn } from '@/lib/utils';

type ArticleDialogCardArticle = {
  article_id: string;
  journal_id?: JournalId | null;
  title?: string | null;
  date?: string | null;
  authors?: string | null;
  abstract?: string | null;
  doi?: string | null;
  platform_id?: string | null;
  permalink?: string | null;
  full_text_file?: string | null;
  journal_title?: string | null;
  volume?: string | null;
  number?: string | null;
  open_access?: number | boolean | null;
  in_press?: number | boolean | null;
};

type ArticleDialogCardProps = {
  article: ArticleDialogCardArticle;
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
          <button
            ref={triggerRef}
            type="button"
            className={cn(
              'block flex-1 cursor-pointer appearance-none border-0 bg-transparent p-0 text-left group outline-none focus-visible:ring-ring/50 focus-visible:ring-[3px]',
            )}
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
          </button>
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
