/**
 * Dense article row content for reading-workspace lists.
 */

import { type ReactNode } from 'react';

import { Skeleton } from '@/components/ui/skeleton';
import * as stylex from '@stylexjs/stylex';
import { colors } from '@/lib/theme.stylex';
import { isString } from '@/lib/type-guards';

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
  xstyle?: stylex.StyleXStyles;
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
  if (isString(preview)) {
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
  xstyle,
}: ArticleListCardProps) {
  const hasPreview = hasPreviewContent(preview);
  const isOpenAccess = isEnabledFlag(openAccess);
  const isInPress = isEnabledFlag(inPress);
  const issueLabel = [volume && `第 ${volume} 卷`, number && `第 ${number} 期`]
    .filter(Boolean)
    .join(', ');
  const authorText = authors && authors.length > 0 ? authors.join('; ') : '';

  return (
    <div data-slot="article-list-card" {...stylex.props(styles.articleListCard, xstyle)}>
      <h3 data-slot="article-list-title" {...stylex.props(styles.articleListTitle)}>
        {title}
      </h3>
      <div data-slot="article-list-meta" {...stylex.props(styles.articleListMeta)}>
        {authorText && (
          <p
            {...stylex.props(styles.articleListMetaItem, styles.articleListCardP)}
            title={authorText}
          >
            {authorText}
          </p>
        )}
        <p {...stylex.props(styles.articleListMetaItem, styles.articleListCardP2)}>
          <span {...stylex.props(styles.articleListCardSpan)}>{journalTitle || '未知期刊'}</span>
          {issueLabel && <span {...stylex.props(styles.articleListCardSpan2)}>{issueLabel}</span>}
          {date && <time {...stylex.props(styles.articleListCardTime)}>{date}</time>}
          {isOpenAccess && (
            <span {...stylex.props(styles.articleListCardSpan3)}>
              <i aria-hidden="true" {...stylex.props(styles.articleListCardI)} />
              开放获取
            </span>
          )}
          {isInPress && (
            <span {...stylex.props(styles.articleListCardSpan4)}>
              <i aria-hidden="true" {...stylex.props(styles.articleListCardI)} />
              预发表
            </span>
          )}
        </p>
      </div>
      {hasPreview && <div {...stylex.props(styles.articleListCardDiv)}>{preview}</div>}
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
    <div aria-hidden="true">
      {Array.from({ length: count }, (_, index) => (
        <div
          key={index}
          {...stylex.props(styles.articleListSkeletonRow, styles.articleListSkeletonDiv)}
        >
          <Skeleton
            xstyle={[styles.articleListSkeletonDivItem, styles.articleListSkeletonSkeleton]}
          />
          <Skeleton
            xstyle={[styles.articleListSkeletonDivItem, styles.articleListSkeletonSkeleton2]}
          />
          <Skeleton
            xstyle={[styles.articleListSkeletonDivItem, styles.articleListSkeletonSkeleton3]}
          />
          <Skeleton
            xstyle={[styles.articleListSkeletonDivItem, styles.articleListSkeletonSkeleton4]}
          />
        </div>
      ))}
    </div>
  );
}

const styles = stylex.create({
  articleListSkeletonRow: {
    borderTopStyle: {
      default: null,
      ':not(:last-child)': 'solid',
    },
    borderTopWidth: {
      default: null,
      ':not(:last-child)': '0px',
    },
    borderBottomStyle: {
      default: null,
      ':not(:last-child)': 'solid',
    },
    borderBottomWidth: {
      default: null,
      ':not(:last-child)': '1px',
    },
    borderColor: {
      default: null,
      ':not(:last-child)': `color-mix(in oklab, ${colors.border} 70%, transparent)`,
    },
  },
  articleListCard: {
    minWidth: '0px',
    paddingBlock: '1.25rem',
    paddingRight: {
      default: '1.25rem',
      '@media (min-width: 40rem)': '1.75rem',
    },
    paddingLeft: {
      default: '1.25rem',
      '@media (min-width: 40rem)': '1.75rem',
    },
    contentVisibility: 'auto',
    containIntrinsicSize: '0 220px',
  },
  articleListTitle: {
    overflow: 'hidden',
    display: '-webkit-box',
    WebkitBoxOrient: 'vertical',
    WebkitLineClamp: 3,
    fontSize: '15px',
    lineHeight: 1.5,
    fontWeight: 700,
    letterSpacing: '-0.005em',
    textWrap: 'wrap',
    overflowWrap: 'break-word',
    color: colors.foreground,
  },
  articleListMetaItem: {
    marginBottom: {
      default: null,
      ':not(:last-child)': '0.25rem',
    },
  },
  articleListMeta: {
    marginTop: '0.5rem',
    fontSize: '0.75rem',
    lineHeight: '1.25rem',
  },
  articleListCardP: {
    overflow: 'hidden',
    textOverflow: 'ellipsis',
    whiteSpace: 'nowrap',
    color: `color-mix(in oklab, ${colors.foreground} 70%, transparent)`,
  },
  articleListCardP2: {
    display: 'flex',
    minWidth: '0px',
    flexWrap: 'wrap',
    alignItems: 'center',
    columnGap: '0.5rem',
    rowGap: '0.25rem',
    fontSize: '11.5px',
    color: colors.mutedForeground,
  },
  articleListCardSpan: {
    minWidth: '0px',
    overflow: 'hidden',
    textOverflow: 'ellipsis',
    whiteSpace: 'nowrap',
    fontWeight: 700,
    letterSpacing: '0.02em',
    color: `color-mix(in oklab, ${colors.foreground} 80%, transparent)`,
  },
  articleListCardSpan2: {
    flexShrink: 0,
  },
  articleListCardTime: {
    flexShrink: 0,
    fontVariantNumeric: 'tabular-nums',
  },
  articleListCardSpan3: {
    display: 'inline-flex',
    flexShrink: 0,
    alignItems: 'center',
    gap: '0.25rem',
    fontWeight: 700,
    color: colors.successForeground,
  },
  articleListCardI: {
    width: '0.375rem',
    height: '0.375rem',
    borderRadius: 'calc(infinity * 1px)',
    backgroundColor: 'currentcolor',
  },
  articleListCardSpan4: {
    display: 'inline-flex',
    flexShrink: 0,
    alignItems: 'center',
    gap: '0.25rem',
    fontWeight: 700,
    color: colors.warningForeground,
  },
  articleListCardDiv: {
    marginTop: '0.75rem',
    overflow: 'hidden',
    display: '-webkit-box',
    WebkitBoxOrient: 'vertical',
    WebkitLineClamp: 2,
    fontSize: '12.5px',
    lineHeight: 1.75,
    textWrap: 'pretty',
    color: colors.mutedForeground,
  },
  articleListSkeletonDivItem: {
    marginBottom: {
      default: null,
      ':not(:last-child)': '0.625rem',
    },
  },
  articleListSkeletonDiv: {
    paddingInline: {
      default: '1.25rem',
      '@media (min-width: 40rem)': '1.75rem',
    },
    paddingBlock: '1.25rem',
  },
  articleListSkeletonSkeleton: {
    height: '1rem',
    width: 'calc(11 / 12 * 100%)',
  },
  articleListSkeletonSkeleton2: {
    height: '1rem',
    width: 'calc(2 / 3 * 100%)',
  },
  articleListSkeletonSkeleton3: {
    height: '0.75rem',
    width: 'calc(1 / 2 * 100%)',
  },
  articleListSkeletonSkeleton4: {
    height: '0.75rem',
    width: '100%',
  },
});
