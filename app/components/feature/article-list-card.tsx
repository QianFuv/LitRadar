'use client';

import { type ReactNode } from 'react';

import { Badge } from '@/components/ui/badge';
import {
  Card,
  CardContent,
  CardDescription,
  CardFooter,
  CardHeader,
  CardTitle,
} from '@/components/ui/card';
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
  action?: ReactNode;
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
 * Render article metadata, selectable title/preview content, and an optional action.
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
  action,
  className,
}: ArticleListCardProps) {
  const hasPreview = hasPreviewContent(preview);
  const isOpenAccess = isEnabledFlag(openAccess);
  const isInPress = isEnabledFlag(inPress);
  const hasBadges = isOpenAccess || isInPress;

  return (
    <Card
      className={cn(
        'content-visibility-card transition-[background-color,box-shadow] duration-200 hover:bg-accent hover:shadow-md',
        className,
      )}
    >
      <CardHeader>
        <div className="flex justify-between items-start gap-4">
          <CardTitle className="text-lg text-foreground">{title}</CardTitle>
          {hasBadges && (
            <div className="flex gap-2 shrink-0">
              {isOpenAccess && (
                <Badge variant="secondary" className="text-xs">
                  开放获取
                </Badge>
              )}
              {isInPress && (
                <Badge variant="outline" className="text-xs">
                  预发表
                </Badge>
              )}
            </div>
          )}
        </div>
        <CardDescription>
          <span>{journalTitle || '未知期刊'}</span>
          {(volume || number) && (
            <span>
              {' '}
              •{' '}
              {[volume && `第 ${volume} 卷`, number && `第 ${number} 期`]
                .filter(Boolean)
                .join(', ')}
            </span>
          )}
          {date && <span> • {date}</span>}
        </CardDescription>
      </CardHeader>
      {hasPreview && (
        <CardContent>
          <div className="line-clamp-3 text-sm leading-relaxed text-muted-foreground">
            {preview}
          </div>
        </CardContent>
      )}
      {action && <CardFooter className="justify-end">{action}</CardFooter>}
    </Card>
  );
}
