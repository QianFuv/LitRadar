'use client';

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

function hasPreviewContent(preview: ReactNode): boolean {
  if (preview === null || preview === undefined || preview === false) {
    return false;
  }
  if (typeof preview === 'string') {
    return preview.trim().length > 0;
  }
  return true;
}

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

  return (
    <Card
      className={cn(
        'transition-all duration-200 hover:bg-accent dark:hover:bg-[#1a1a1a] hover:shadow-[0px_0px_0px_1px_rgba(0,0,0,0.1),0px_4px_12px_rgba(0,0,0,0.08)] dark:hover:shadow-[0px_0px_0px_1px_rgba(255,255,255,0.3),0px_4px_12px_rgba(255,255,255,0.05)]',
        className,
      )}
    >
      <CardHeader>
        <div className="flex justify-between items-start gap-4">
          <CardTitle className="text-lg text-slate-900 dark:text-slate-100 group-hover:text-blue-600 dark:group-hover:text-blue-400 transition-colors">
            {title}
          </CardTitle>
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
          <div className="text-sm text-slate-600 dark:text-slate-400 line-clamp-3 leading-relaxed">
            {preview}
          </div>
        </CardContent>
      )}
    </Card>
  );
}
