'use client';

import { type ReactNode } from 'react';

import { Badge } from '@/components/ui/badge';
import {
  Card,
  CardContent,
  CardDescription,
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
  className?: string;
};

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
  const hasBadges = Boolean(openAccess) || Boolean(inPress);

  return (
    <Card
      className={cn(
        'hover:shadow-md transition-all duration-200 border-transparent hover:border-slate-200 dark:hover:border-slate-800',
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
              {Boolean(openAccess) && (
                <Badge variant="secondary" className="text-xs">
                  开放获取
                </Badge>
              )}
              {Boolean(inPress) && (
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
