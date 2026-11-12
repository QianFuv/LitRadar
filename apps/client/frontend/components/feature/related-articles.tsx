/**
 * Related-article list shown at the end of article details.
 */

import { useQuery } from '@tanstack/react-query';

import { HoverHighlightGroup, hoverHighlightItemProps } from '@/components/ui/hover-highlight';
import { RevealItem } from '@/components/ui/reveal';
import { Skeleton } from '@/components/ui/skeleton';
import { relatedArticlesQuery, type Article } from '@/lib/api';
import { getArticleDisplayTitle } from '@/lib/article-title';
import { useInView } from '@/lib/use-in-view';
import { LinkIcon } from '@/components/ui/animated-icons';
import * as stylex from '@stylexjs/stylex';
import { colors, lineHeights } from '@/lib/theme.stylex';

/** Number of related articles requested from the backend. */
const RELATED_ARTICLE_LIMIT = 6;

type RelatedArticlesProps = {
  article: Article;
  dbName: string;
  /** Open a related article in the current reader. */
  onOpen: (article: Article) => void;
};

/**
 * Render related articles, ranked by the backend, once the section scrolls near the viewport.
 *
 * @param props - Source article, database, and open callback.
 * @returns Related-article section.
 */
export function RelatedArticles({ article, dbName, onOpen }: RelatedArticlesProps) {
  const { ref, isInView } = useInView({ isOnce: true, rootMargin: '300px' });
  const relatedQuery = useQuery({
    ...relatedArticlesQuery(article.article_id, dbName, RELATED_ARTICLE_LIMIT),
    enabled: isInView && Boolean(dbName),
    staleTime: 10 * 60 * 1000,
    retry: false,
  });
  const related = relatedQuery.data ?? [];

  return (
    <section ref={ref} aria-label="相关文章" {...stylex.props(styles.relatedArticlesSection)}>
      <h3 {...stylex.props(styles.relatedArticlesH3)}>
        <span {...stylex.props(styles.relatedArticlesSpan)}>
          <LinkIcon animateOnView xstyle={styles.relatedArticlesLinkIcon} aria-hidden="true" />
          相关文章
        </span>
      </h3>
      <RevealItem
        isReady={!relatedQuery.isLoading}
        placeholder={
          <div {...stylex.props(styles.relatedArticlesDiv)}>
            <output {...stylex.props(styles.relatedArticlesDivItem, styles.relatedArticlesSpan2)}>
              正在查找相关文章
            </output>
            {[0, 1, 2].map((index) => (
              <div key={index} {...stylex.props(styles.relatedArticlesDivItem)}>
                <Skeleton
                  xstyle={[styles.relatedArticlesDivItem2, styles.relatedArticlesSkeleton]}
                />
                <Skeleton
                  xstyle={[styles.relatedArticlesDivItem2, styles.relatedArticlesSkeleton2]}
                />
              </div>
            ))}
          </div>
        }
      >
        {relatedQuery.isError ? (
          <p {...stylex.props(styles.relatedArticlesP)}>暂时无法加载相关文章。</p>
        ) : !relatedQuery.isSuccess ? (
          <div aria-hidden="true" {...stylex.props(styles.relatedArticlesDiv2)} />
        ) : related.length === 0 ? (
          <p {...stylex.props(styles.relatedArticlesP)}>暂未找到相关文章。</p>
        ) : (
          <HoverHighlightGroup>
            <ul {...stylex.props(styles.relatedArticlesUl)}>
              {related.map((candidate) => (
                <li
                  {...stylex.props(styles.relatedArticlesUlItem)}
                  key={String(candidate.article_id)}
                >
                  <button
                    type="button"
                    {...hoverHighlightItemProps}
                    {...stylex.props(styles.relatedArticlesButton)}
                    onClick={() => onOpen(candidate)}
                  >
                    <span {...stylex.props(styles.relatedArticlesSpan3)}>
                      {getArticleDisplayTitle(candidate)}
                    </span>
                    <span {...stylex.props(styles.relatedArticlesSpan4)}>
                      {[candidate.journal_title, candidate.date].filter(Boolean).join(' · ')}
                    </span>
                  </button>
                </li>
              ))}
            </ul>
          </HoverHighlightGroup>
        )}
      </RevealItem>
    </section>
  );
}

const styles = stylex.create({
  relatedArticlesSection: {
    marginTop: '2rem',
  },
  relatedArticlesH3: {
    display: 'flex',
    alignItems: 'center',
    gap: '0.75rem',
    fontSize: '11px',
    fontWeight: 700,
    letterSpacing: '0.16em',
    color: colors.mutedForeground,
    '::after': {
      content: '""',
      height: '1px',
      flex: '1',
      backgroundColor: colors.border,
    },
  },
  relatedArticlesSpan: {
    display: 'flex',
    alignItems: 'center',
    gap: '0.375rem',
  },
  relatedArticlesLinkIcon: {
    width: '0.875rem',
    height: '0.875rem',
    flexShrink: 0,
  },
  relatedArticlesDivItem: {
    marginBottom: {
      default: null,
      ':not(:last-child)': '0.75rem',
    },
  },
  relatedArticlesDiv: {
    marginTop: '0.75rem',
  },
  relatedArticlesSpan2: {
    position: 'absolute',
    width: '1px',
    height: '1px',
    padding: 0,
    margin: '-1px',
    overflow: 'hidden',
    clipPath: 'inset(50%)',
    whiteSpace: 'nowrap',
    borderWidth: 0,
  },
  relatedArticlesDivItem2: {
    marginBottom: {
      default: null,
      ':not(:last-child)': '0.375rem',
    },
  },
  relatedArticlesSkeleton: {
    height: '1rem',
    width: 'calc(11 / 12 * 100%)',
  },
  relatedArticlesSkeleton2: {
    height: '0.75rem',
    width: 'calc(1 / 2 * 100%)',
  },
  relatedArticlesP: {
    marginTop: '0.75rem',
    fontSize: '13px',
    color: colors.mutedForeground,
  },
  relatedArticlesDiv2: {
    height: '4rem',
  },
  relatedArticlesUlItem: {
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
  relatedArticlesUl: {
    marginTop: '0.25rem',
  },
  relatedArticlesButton: {
    marginInline: '-0.5rem',
    display: 'block',
    width: 'calc(100% + 1rem)',
    borderRadius: '6px',
    paddingInline: '0.5rem',
    paddingBlock: '0.75rem',
    textAlign: 'left',
    transitionProperty: 'color, border-color, outline-color, text-decoration-color, fill, stroke',
    transitionTimingFunction: 'cubic-bezier(0.25, 0.1, 0.25, 1)',
    transitionDuration: '200ms',
    outlineStyle: 'none',
    backgroundColor: {
      default: null,
      ':hover': {
        default: null,
        '@media (hover: hover)': colors.accent,
      },
    },
    boxShadow: {
      default: null,
      ':focus-visible': `0 0 0 2px ${colors.ring}`,
    },
  },
  relatedArticlesSpan3: {
    overflow: 'hidden',
    display: '-webkit-box',
    WebkitBoxOrient: 'vertical',
    WebkitLineClamp: 2,
    fontSize: '13.5px',
    lineHeight: 1.5,
    fontWeight: 700,
  },
  relatedArticlesSpan4: {
    marginTop: '0.25rem',
    display: 'block',
    overflow: 'hidden',
    textOverflow: 'ellipsis',
    whiteSpace: 'nowrap',
    fontSize: '0.75rem',
    lineHeight: lineHeights.textXs,
    color: colors.mutedForeground,
  },
});
