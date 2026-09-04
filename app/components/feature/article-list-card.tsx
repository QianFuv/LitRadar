'use client';

/**
 * Compact article summary card with selectable content.
 */

import { type ReactNode } from 'react';

import { Badge } from '@/components/ui/badge';
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/components/ui/card';
import { cn } from '@/lib/utils';

type ArticleListCardProps = {
  title: ReactNode;
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
 * @returns True when the card should render its preview section.
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
 * Render article metadata and selectable title/preview content.
 *
 * @param props - Article list card content.
 * @returns Article list card.
 */
export function ArticleListCard({
  title,
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
  const hasBadges = isOpenAccess || isInPress;
  const issueLabel = [volume && `第 ${volume} 卷`, number && `第 ${number} 期`]
    .filter(Boolean)
    .join(', ');

  return (
    <Card
      className={cn(
        'motion-control content-visibility-card gap-0 overflow-hidden py-0 transition-[background-color] hover:bg-accent/30',
        className,
      )}
    >
      <CardHeader className="gap-2 px-4 py-4 sm:px-5 sm:py-5">
        <div className="flex flex-col items-start gap-2 sm:flex-row sm:justify-between sm:gap-3">
          <CardTitle className="min-w-0 text-balance break-words text-base leading-snug text-foreground sm:text-lg">
            {title}
          </CardTitle>
          {hasBadges && (
            <div className="flex shrink-0 flex-wrap gap-1.5 sm:justify-end">
              {isOpenAccess && (
                <Badge variant="secondary" className="px-2 py-0 text-[11px]">
                  开放获取
                </Badge>
              )}
              {isInPress && (
                <Badge variant="outline" className="px-2 py-0 text-[11px]">
                  预发表
                </Badge>
              )}
            </div>
          )}
        </div>
        <CardDescription className="flex flex-wrap items-center gap-x-2 gap-y-1 text-xs leading-relaxed">
          <span className="font-medium text-foreground/70">{journalTitle || '未知期刊'}</span>
          {issueLabel && <span>{issueLabel}</span>}
          {date && <time>{date}</time>}
        </CardDescription>
      </CardHeader>
      {hasPreview && (
        <CardContent className="px-4 pb-4 sm:px-5 sm:pb-5">
          <div className="line-clamp-3 text-pretty text-sm leading-6 text-muted-foreground">
            {preview}
          </div>
        </CardContent>
      )}
    </Card>
  );
}
