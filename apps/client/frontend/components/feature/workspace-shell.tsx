/**
 * Two-pane reading workspace: a list column beside a docked reading desk.
 */

import { useHotkey } from '@tanstack/react-hotkeys';
import { X } from '@/components/ui/animated-icons';
import { useCallback, useEffect, useMemo, useRef, useState, type ReactNode } from 'react';

import {
  ArticleReaderContext,
  READER_ITEM_ATTRIBUTE,
  WORKSPACE_SEARCH_ATTRIBUTE,
  type ArticleReaderContextValue,
} from '@/components/feature/article-reader-context';
import { Button } from '@/components/ui/button';
import {
  Dialog,
  DialogClose,
  DialogContent,
  DialogDescription,
  DialogTitle,
} from '@/components/ui/dialog';
import { READER_DOCK_MEDIA_QUERY, useMediaQuery } from '@/lib/use-media-query';
import * as stylex from '@stylexjs/stylex';
import { colors } from '@/lib/theme.stylex';

type ExplicitReader = {
  /** Reader content for the current selection. */
  content: ReactNode;
  /** Whether the narrow-screen reader sheet is open. */
  isSheetOpen: boolean;
  /** Open or close the narrow-screen reader sheet. */
  onSheetOpenChange: (open: boolean) => void;
  /** Accessible sheet title. */
  title: string;
  /** Accessible sheet description. */
  description: string;
};

type WorkspaceShellProps = {
  children: ReactNode;
  /** Sticky list header with page title, search, and filters. */
  header: ReactNode;
  /** Accessible list-column name. */
  listLabel: string;
  /** Let the list body manage its own scrolling instead of the shared scroll container. */
  isListSelfScrolling?: boolean;
  /** Reader content supplied by the view; omit to let article items portal into the desk. */
  reader?: ExplicitReader;
  /**
   * Empty reading-desk content shown when nothing is selected. List-emphasized workspaces leave
   * the side panel blank instead, because auto-selection only leaves it empty while the list
   * itself is loading or empty and already explains why.
   */
  readerPlaceholder: ReactNode;
  /**
   * Column balance while docked: `list` gives the list three fifths of the width and shows
   * details in a plain side panel; `reader` keeps a narrow directory beside a wide desk.
   */
  emphasis?: 'list' | 'reader';
  /** Select the first list item automatically whenever the docked reader is empty. */
  shouldAutoSelectFirst?: boolean;
};

/**
 * Single-key workspace shortcut options: skip keys a focused field or widget uses, keys another
 * handler already claimed, and leave default prevention to the callback.
 */
const WORKSPACE_HOTKEY_OPTIONS = {
  ignoreDefaultPrevented: true,
  ignoreInputs: true,
  preventDefault: false,
  stopPropagation: false,
} as const;

/**
 * Move keyboard focus to the next or previous list item and select it when docked.
 *
 * @param container - List container.
 * @param direction - 1 for next, -1 for previous.
 * @param isDocked - Whether selection should immediately update the docked reader.
 */
function moveReaderSelection(container: HTMLElement, direction: 1 | -1, isDocked: boolean): void {
  const items = Array.from(container.querySelectorAll<HTMLElement>(`[${READER_ITEM_ATTRIBUTE}]`));
  if (items.length === 0) {
    return;
  }
  const activeElement = document.activeElement;
  const focusedIndex = items.findIndex((item) => item === activeElement);
  const selectedIndex = items.findIndex((item) => item.dataset.selected === 'true');
  const currentIndex = focusedIndex >= 0 ? focusedIndex : selectedIndex;
  const nextIndex =
    currentIndex < 0
      ? direction === 1
        ? 0
        : items.length - 1
      : Math.min(items.length - 1, Math.max(0, currentIndex + direction));
  const nextItem = items[nextIndex];
  nextItem.focus({ preventScroll: true });
  nextItem.scrollIntoView?.({ block: 'nearest' });
  if (isDocked) {
    nextItem.click();
  }
}

/**
 * Render the list column, docked reading desk, keyboard shortcuts, and narrow reader sheet.
 *
 * @param props - Header, list content, reader mode, and placeholder.
 * @returns Shared reading workspace.
 */
export function WorkspaceShell({
  children,
  header,
  listLabel,
  isListSelfScrolling = false,
  reader,
  readerPlaceholder,
  emphasis = 'reader',
  shouldAutoSelectFirst = false,
}: WorkspaceShellProps) {
  const isDocked = useMediaQuery(READER_DOCK_MEDIA_QUERY);
  const [readerNode, setReaderNode] = useState<HTMLElement | null>(null);
  const [selectedKey, setSelectedKey] = useState<string | null>(null);
  const listRef = useRef<HTMLElement>(null);
  const deskRef = useRef<HTMLElement>(null);
  const sheetReturnFocusRef = useRef<HTMLElement | null>(null);

  const select = useCallback((key: string) => {
    setSelectedKey(key);
    deskRef.current?.scrollTo?.({ top: 0 });
  }, []);
  const release = useCallback((key: string) => {
    setSelectedKey((current) => (current === key ? null : current));
  }, []);
  const readerContext = useMemo<ArticleReaderContextValue>(
    () => ({ isDocked, readerNode, selectedKey, select, release }),
    [isDocked, readerNode, selectedKey, select, release],
  );

  /**
   * Run a list shortcut unless an overlay is open, claiming the key only when it acts.
   *
   * @param event - Keyboard event that matched the shortcut.
   * @param action - Shortcut action; returns whether it handled the key.
   */
  function runListShortcut(
    event: KeyboardEvent,
    action: (listElement: HTMLElement) => boolean,
  ): void {
    const listElement = listRef.current;
    if (!listElement || document.querySelector('[role="dialog"]')) {
      return;
    }
    if (action(listElement)) {
      event.preventDefault();
    }
  }

  useHotkey(
    '/',
    (event) =>
      runListShortcut(event, (listElement) => {
        const searchElement = listElement.querySelector<HTMLElement>(
          `[${WORKSPACE_SEARCH_ATTRIBUTE}]`,
        );
        searchElement?.focus();
        return Boolean(searchElement);
      }),
    WORKSPACE_HOTKEY_OPTIONS,
  );
  useHotkey(
    'J',
    (event) =>
      runListShortcut(event, (listElement) => {
        moveReaderSelection(listElement, 1, isDocked);
        return true;
      }),
    WORKSPACE_HOTKEY_OPTIONS,
  );
  useHotkey(
    'K',
    (event) =>
      runListShortcut(event, (listElement) => {
        moveReaderSelection(listElement, -1, isDocked);
        return true;
      }),
    WORKSPACE_HOTKEY_OPTIONS,
  );

  useEffect(() => {
    const listElement = listRef.current;
    if (!shouldAutoSelectFirst || reader || !isDocked || !readerNode || selectedKey !== null) {
      return;
    }
    if (!listElement) {
      return;
    }
    /**
     * Show the first available list item in the docked reader.
     *
     * @returns Whether an item was selected.
     */
    function selectFirstItem(): boolean {
      const firstItem = listElement?.querySelector<HTMLElement>(`[${READER_ITEM_ATTRIBUTE}]`);
      firstItem?.click();
      return firstItem !== null && firstItem !== undefined;
    }
    if (selectFirstItem()) {
      return;
    }
    const observer = new MutationObserver(() => {
      if (selectFirstItem()) {
        observer.disconnect();
      }
    });
    observer.observe(listElement, { childList: true, subtree: true });
    return () => observer.disconnect();
  }, [isDocked, reader, readerNode, selectedKey, shouldAutoSelectFirst]);

  const hasArticleSelection = !reader && selectedKey !== null;
  const isListEmphasized = emphasis === 'list';

  return (
    <ArticleReaderContext.Provider value={readerContext}>
      <div {...stylex.props(styles.workspaceShellDiv)}>
        <section
          ref={listRef}
          aria-label={listLabel}
          data-slot="workspace-list"
          {...stylex.props(
            styles.workspaceList,
            isListEmphasized
              ? styles.workspaceListListEmphasized
              : styles.workspaceListNotListEmphasized,
          )}
        >
          <header data-slot="workspace-list-header" {...stylex.props(styles.workspaceListHeader)}>
            {header}
          </header>
          {isListSelfScrolling ? (
            <div {...stylex.props(styles.listContent)}>{children}</div>
          ) : (
            <div id="results-scroll-container" {...stylex.props(styles.workspaceShellDiv3)}>
              <div {...stylex.props(styles.listContentFlow)}>{children}</div>
            </div>
          )}
        </section>

        {isDocked && (
          <section
            ref={deskRef}
            aria-label="阅读台"
            data-slot="reading-desk"
            data-emphasis={emphasis}
            {...stylex.props(
              styles.readingDesk,
              isListEmphasized
                ? styles.readingDeskListEmphasized
                : styles.readingDeskNotListEmphasized,
            )}
          >
            {reader ? (
              reader.content
            ) : (
              <>
                <div ref={setReaderNode} {...stylex.props(styles.workspaceShellDiv5)} />
                {!hasArticleSelection && !isListEmphasized && readerPlaceholder}
              </>
            )}
          </section>
        )}
      </div>

      {reader && !isDocked && (
        <Dialog open={reader.isSheetOpen} onOpenChange={reader.onSheetOpenChange}>
          <DialogContent
            placement="right"
            xstyle={[
              styles.workspaceShellDialogContent,
              isListEmphasized ? readerSheetWidths.list : readerSheetWidths.reader,
            ]}
            initialFocus={() => {
              sheetReturnFocusRef.current =
                document.activeElement instanceof HTMLElement ? document.activeElement : null;
              return true;
            }}
            finalFocus={() => {
              const returnTarget = sheetReturnFocusRef.current;
              sheetReturnFocusRef.current = null;
              return returnTarget?.isConnected ? returnTarget : true;
            }}
          >
            <div {...stylex.props(styles.workspaceShellDiv6)}>
              <DialogClose
                render={
                  <Button
                    type="button"
                    variant="ghost"
                    size="icon"
                    static
                    aria-label="关闭"
                    xstyle={styles.workspaceShellButton}
                  />
                }
              >
                <X xstyle={styles.workspaceShellX} aria-hidden="true" />
              </DialogClose>
              <DialogTitle xstyle={styles.workspaceShellDialogTitle}>{reader.title}</DialogTitle>
              <DialogDescription xstyle={styles.workspaceShellDialogDescription}>
                {reader.description}
              </DialogDescription>
            </div>
            <div data-slot="reader-sheet-body" {...stylex.props(styles.readerSheetBody)}>
              {reader.content}
            </div>
          </DialogContent>
        </Dialog>
      )}
    </ArticleReaderContext.Provider>
  );
}

const readerSheetWidths = stylex.create({
  list: {
    width: {
      default: '100%',
      '@media (min-width: 48rem)': 'min(calc((80rem - 4rem) * 2 / 5), calc(100vw - 5rem))',
    },
  },
  reader: {
    width: {
      default: '100%',
      '@media (min-width: 48rem)': 'min(calc(80rem - 4rem - 460px), calc(100vw - 5rem))',
    },
  },
});

/**
 * Width of the narrow-screen article detail sheet: the list-emphasized reading desk's width at
 * the 80rem dock breakpoint, where the 4rem navigation rail leaves 76rem that the desk shares
 * with the list 2 : 3. Workspace sheets use the same rule per emphasis (`reader` sits beside a
 * 460px directory), so details keep one width whether they slide in or dock; phones keep the
 * full-width sheet, and a 5rem strip of the page always stays visible beside it.
 */
export const listReaderSheetWidthStyle = readerSheetWidths.list;

const styles = stylex.create({
  listContent: {
    display: 'flex',
    minHeight: '0px',
    flex: '1',
    flexDirection: 'column',
  },
  listContentFlow: {
    minWidth: '0px',
  },
  workspaceShellDiv: {
    display: 'flex',
    height: '100%',
    minHeight: '0px',
    width: '100%',
  },
  workspaceList: {
    display: 'flex',
    height: '100%',
    minHeight: '0px',
    width: '100%',
    minWidth: '0px',
    flexDirection: 'column',
    backgroundColor: colors.background,
  },
  workspaceListListEmphasized: {
    width: {
      default: '100%',
      '@media (min-width: 80rem)': 'auto',
    },
    flexGrow: {
      default: null,
      '@media (min-width: 80rem)': 3,
    },
    flexShrink: {
      default: null,
      '@media (min-width: 80rem)': 1,
    },
    flexBasis: {
      default: null,
      '@media (min-width: 80rem)': '0%',
    },
  },
  workspaceListNotListEmphasized: {
    width: {
      default: '100%',
      '@media (min-width: 80rem)': '460px',
      '@media (min-width: 96rem)': '520px',
    },
    flexShrink: {
      default: null,
      '@media (min-width: 80rem)': 0,
    },
    borderRightStyle: {
      default: null,
      '@media (min-width: 80rem)': 'solid',
    },
    borderRightWidth: {
      default: null,
      '@media (min-width: 80rem)': '1px',
    },
  },
  workspaceListHeader: {
    zIndex: 20,
    flexShrink: 0,
    borderBottomStyle: 'solid',
    borderBottomWidth: '1px',
    backgroundColor: colors.background,
    paddingInline: {
      default: '1rem',
      '@media (min-width: 40rem)': '1.25rem',
    },
    paddingTop: {
      default: '1rem',
      '@media (min-width: 48rem)': '1.25rem',
    },
    paddingBottom: '0.75rem',
  },
  workspaceShellDiv3: {
    minHeight: '0px',
    flex: '1',
    overflowY: 'auto',
    overscrollBehavior: 'contain',
    paddingBottom: {
      default: 'calc(5.5rem + var(--safe-area-inset-bottom, env(safe-area-inset-bottom, 0px)))',
      '@media (min-width: 48rem)': '2rem',
    },
  },
  readingDesk: {
    position: 'relative',
    display: {
      default: 'none',
      '@media (min-width: 80rem)': 'block',
    },
    minHeight: '0px',
    minWidth: '0px',
    overflowY: 'auto',
  },
  readingDeskListEmphasized: {
    flexGrow: 2,
    flexShrink: 1,
    flexBasis: '0%',
    borderLeftStyle: 'solid',
    borderLeftWidth: '1px',
    backgroundColor: colors.background,
  },
  readingDeskNotListEmphasized: {
    flex: '1',
    borderLeftStyle: 'solid',
    borderLeftWidth: '1px',
    backgroundColor: colors.background,
  },
  workspaceShellDiv5: {
    display: 'contents',
  },
  workspaceShellDialogContent: {
    display: 'flex',
    flexDirection: 'column',
    gap: '0px',
    overflow: 'hidden',
    backgroundColor: colors.canvas,
  },
  workspaceShellDiv6: {
    display: 'flex',
    height: '3.5rem',
    flexShrink: 0,
    alignItems: 'center',
    gap: '0.5rem',
    borderBottomStyle: 'solid',
    borderBottomWidth: '1px',
    backgroundColor: colors.background,
    paddingInline: '0.375rem',
  },
  workspaceShellButton: {
    width: '2.75rem',
    height: '2.75rem',
  },
  workspaceShellX: {
    width: '1.25rem',
    height: '1.25rem',
  },
  workspaceShellDialogTitle: {
    minWidth: '0px',
    overflow: 'hidden',
    textOverflow: 'ellipsis',
    fontSize: '15px',
    lineHeight: 'inherit',
  },
  workspaceShellDialogDescription: {
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
  readerSheetBody: {
    minHeight: '0px',
    flex: '1',
    overflowY: 'auto',
  },
});
