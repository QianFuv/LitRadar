'use client';

/**
 * Dense article row content for reading-workspace lists.
 */

import { type ReactNode } from 'react';

import { Skeleton } from '@/components/ui/skeleton';
import { cn } from '@/lib/utils';

type ArticleListCardProps = {
  title: ReactNode;
  authors?: string[] | null;
  journalTitle?: string | null;
  volume?: string | null;
  number?: string | null;
  date?: string | null;
  preview?: ReactNode;
  openAccess?: number | boolean | null;
  inPress?: number | boolean | null;
  className?: string;
};

/**
 * Check whether an API flag is explicitly enabled.
 *
 * @param value - API flag value.
 * @returns True when the flag is explicitly enabled.
 */
function isEnabledFlag(value: number | boolean | string | null | undefined): boolean {
  return value === true || value === 1 || value === '1';
}

/**
 * Check whether the preview prop contains visible content.
 *
 * @param preview - Preview node supplied by the article list.
 * @returns True when the row should render its preview line.
 */
function hasPreviewContent(preview: ReactNode): boolean {
  if (preview === null || preview === undefined || preview === false) {
    return false;
  }
  if (typeof preview === 'string') {
    return preview.trim().length > 0;
  }
  return true;
}

/**
 * Render an article row: title, authors, source line with access tags, and a short preview.
 *
 * @param props - Article row content.
 * @returns Article list row content.
 */
export function ArticleListCard({
  title,
  authors,
  journalTitle,
  volume,
  number,
  date,
  preview,
  openAccess,
  inPress,
  className,
}: ArticleListCardProps) {
  const hasPreview = hasPreviewContent(preview);
  const isOpenAccess = isEnabledFlag(openAccess);
  const isInPress = isEnabledFlag(inPress);
  const issueLabel = [volume && `第 ${volume} 卷`, number && `第 ${number} 期`]
    .filter(Boolean)
    .join(', ');
  const authorText = authors && authors.length > 0 ? authors.join('; ') : '';

  return (
    <div
      data-slot="article-list-card"
      className={cn('content-visibility-card min-w-0 py-5 pr-5 pl-5 sm:pr-7 sm:pl-7', className)}
    >
      <h3
        data-slot="article-list-title"
        className="line-clamp-3 text-[15px] leading-[1.5] font-bold tracking-[-0.005em] text-wrap break-words text-foreground"
      >
        {title}
      </h3>
      <div data-slot="article-list-meta" className="mt-2 space-y-1 text-xs leading-5">
        {authorText && (
          <p className="truncate text-foreground/65" title={authorText}>
            {authorText}
          </p>
        )}
        <p className="flex min-w-0 flex-wrap items-center gap-x-2 gap-y-1 text-[11.5px] text-muted-foreground">
          <span className="min-w-0 truncate font-bold tracking-[0.02em] text-foreground/80">
            {journalTitle || '未知期刊'}
          </span>
          {issueLabel && <span className="shrink-0">{issueLabel}</span>}
          {date && <time className="shrink-0 tabular-nums">{date}</time>}
          {isOpenAccess && (
            <span className="inline-flex shrink-0 items-center gap-1 font-bold text-success-foreground">
              <i aria-hidden="true" className="size-1.5 rounded-full bg-current" />
              开放获取
            </span>
          )}
          {isInPress && (
            <span className="inline-flex shrink-0 items-center gap-1 font-bold text-warning-foreground">
              <i aria-hidden="true" className="size-1.5 rounded-full bg-current" />
              预发表
            </span>
          )}
        </p>
      </div>
      {hasPreview && (
        <div className="mt-3 line-clamp-2 text-pretty text-[12.5px] leading-[1.75] text-muted-foreground/85">
          {preview}
        </div>
      )}
    </div>
  );
}

/**
 * Render decorative placeholders shaped like article rows.
 *
 * @param props - Number of placeholder rows.
 * @returns Hidden loading placeholders.
 */
export function ArticleListSkeleton({ count = 6 }: { count?: number }) {
  return (
    <div className="divide-y divide-border/70" aria-hidden="true">
      {Array.from({ length: count }, (_, index) => (
        <div key={index} className="space-y-2.5 px-5 py-5 sm:px-7">
          <Skeleton className="h-4 w-11/12" />
          <Skeleton className="h-4 w-2/3" />
          <Skeleton className="h-3 w-1/2" />
          <Skeleton className="h-3 w-full" />
        </div>
      ))}
    </div>
  );
}
