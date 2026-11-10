/**
 * Keyboard-accessible article rows that open in the docked reader or a detail sheet.
 */

import { useEffect, useState, type MouseEvent, type ReactNode } from 'react';
import { createPortal } from 'react-dom';
import type { BaseUIEvent } from '@base-ui/react/types';

import {
  ArticleDetailDialogContent,
  ArticleDetailPane,
} from '@/components/feature/article-detail-dialog-content';
import { ArticleListCard } from '@/components/feature/article-list-card';
import {
  READER_ITEM_ATTRIBUTE,
  useArticleReader,
} from '@/components/feature/article-reader-context';
import { Dialog, DialogTrigger } from '@/components/ui/dialog';
import { type Article } from '@/lib/api';
import { getArticleDisplayTitle, hasArticleTitle } from '@/lib/article-title';
import * as stylex from '@stylexjs/stylex';
import { colors } from '@/lib/theme.stylex';

type ArticleDialogCardProps = {
  article: Article;
  dbName: string;
  title?: ReactNode;
  preview?: ReactNode;
  initialFolderIds?: number[];
  isFavoriteStatePending?: boolean;
  isFavoriteStateUnavailable?: boolean;
  /** Folder whose membership the favorite button toggles directly. */
  favoriteToggleFolderId?: number;
  extraActions?: ReactNode;
  leading?: ReactNode;
  triggerRef?: (node?: Element | null) => void;
  xstyle?: stylex.StyleXStyles;
};

/**
 * Preserve text selection when a pointer gesture ends inside the article row.
 *
 * @param event - Pointer-generated click on the article trigger.
 */
function preserveArticleTextSelection(event: MouseEvent<HTMLElement>): void {
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
 * Render a selectable article row that fills the docked reader or opens a detail sheet.
 *
 * @param props - Article row and detail configuration.
 * @returns Article row with its detail surface.
 */
export function ArticleDialogCard({
  article,
  dbName,
  title,
  preview,
  initialFolderIds = [],
  isFavoriteStatePending = false,
  isFavoriteStateUnavailable = false,
  favoriteToggleFolderId,
  extraActions,
  leading,
  triggerRef,
  xstyle,
}: ArticleDialogCardProps) {
  const reader = useArticleReader();
  const [open, setOpen] = useState(false);
  const itemKey = `${dbName}::${article.article_id}`;
  const readerNode = reader?.isDocked ? reader.readerNode : null;
  const isDocked = readerNode !== null;
  const isSelected = isDocked && reader?.selectedKey === itemKey;
  const release = reader?.release;
  const displayTitle = getArticleDisplayTitle(article);
  const resolvedTitle = hasArticleTitle(article) ? (title ?? displayTitle) : displayTitle;
  const resolvedPreview = preview ?? article.abstract;
  const detailProps = {
    article,
    dbName,
    initialFolderIds,
    isFavoriteStatePending,
    isFavoriteStateUnavailable,
    favoriteToggleFolderId,
    extraActions,
  };

  useEffect(() => () => release?.(itemKey), [itemKey, release]);

  /**
   * Select the row in the docked reader, or let the dialog trigger open the sheet unless the
   * click only ends a text selection.
   *
   * @param event - Click on the row trigger.
   */
  function handleClick(event: BaseUIEvent<MouseEvent<HTMLElement>>): void {
    preserveArticleTextSelection(event);
    if (!event.defaultPrevented && isDocked && reader) {
      event.preventDefault();
      reader.select(itemKey);
    }
    if (event.defaultPrevented) {
      event.preventBaseUIHandler();
    }
  }

  return (
    <Dialog open={open && !isDocked} onOpenChange={setOpen}>
      <div
        data-selected={isSelected ? 'true' : 'false'}
        {...stylex.props(styles.articleDialogCardDiv, stylex.defaultMarker(), xstyle)}
      >
        {leading && <div {...stylex.props(styles.articleDialogCardDiv2)}>{leading}</div>}
        <DialogTrigger
          nativeButton={false}
          onClick={handleClick}
          render={
            <div
              ref={triggerRef}
              {...{ [READER_ITEM_ATTRIBUTE]: '' }}
              data-selected={isSelected ? 'true' : 'false'}
              aria-current={isSelected ? 'true' : undefined}
              aria-label={`查看文章详情：${displayTitle}`}
              {...stylex.props(styles.articleDialogCardDiv3)}
            />
          }
        >
          <ArticleListCard
            title={resolvedTitle}
            authors={article.authors}
            journalTitle={article.journal_title}
            volume={article.volume}
            number={article.number}
            date={article.date}
            preview={resolvedPreview}
            openAccess={article.open_access}
            inPress={article.in_press}
            xstyle={leading ? styles.articleDialogCardArticleListCardLeading : null}
          />
        </DialogTrigger>
      </div>
      {open && !isDocked && <ArticleDetailDialogContent {...detailProps} />}
      {isSelected && readerNode && createPortal(<ArticleDetailPane {...detailProps} />, readerNode)}
    </Dialog>
  );
}

const styles = stylex.create({
  articleDialogCardDiv: {
    position: 'relative',
    display: 'flex',
    alignItems: 'flex-start',
    borderBottomStyle: 'solid',
    borderBottomWidth: '1px',
    borderColor: `color-mix(in oklab, ${colors.border} 60%, transparent)`,
    transitionProperty: 'background-color',
    transitionTimingFunction: 'cubic-bezier(0.25, 0.1, 0.25, 1)',
    transitionDuration: '200ms',
    backgroundColor: {
      default: null,
      ':is([data-selected="true"])': colors.accent,
      ':hover': {
        default: null,
        '@media (hover: hover)': {
          default: `color-mix(in oklab, ${colors.accent} 50%, transparent)`,
          ':is([data-selected="true"])': colors.accent,
        },
      },
    },
  },
  articleDialogCardDiv2: {
    flexShrink: 0,
    paddingTop: '22px',
    paddingLeft: {
      default: '1.25rem',
      '@media (min-width: 40rem)': '1.75rem',
    },
  },
  articleDialogCardDiv3: {
    minWidth: '0px',
    flex: '1',
    cursor: 'pointer',
    outlineStyle: 'none',
    boxShadow: {
      default: null,
      ':focus-visible': `inset 0 0 0 2px ${colors.ring}`,
    },
  },
  articleDialogCardArticleListCardLeading: {
    paddingLeft: {
      default: '0.75rem',
      '@media (min-width: 40rem)': '0.75rem',
    },
  },
});
