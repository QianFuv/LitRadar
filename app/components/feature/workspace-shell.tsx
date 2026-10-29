'use client';

/**
 * Two-pane reading workspace: a list column beside a docked reading desk.
 */

import { X } from 'lucide-react';
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
import { cn } from '@/lib/utils';

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
  /** Optional classes for the list scroll body. */
  contentClassName?: string;
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
 * Check whether a keyboard event originates from an editable or interactive text field.
 *
 * @param target - Event target.
 * @returns True when single-key shortcuts must be ignored.
 */
function isEditableTarget(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) {
    return false;
  }
  return (
    target.isContentEditable ||
    ['INPUT', 'TEXTAREA', 'SELECT'].includes(target.tagName) ||
    target.closest('[role="dialog"], [role="menu"], [role="listbox"]') !== null
  );
}

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
  contentClassName,
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

  useEffect(() => {
    /**
     * Handle slash and J/K shortcuts outside editable fields and overlays.
     *
     * @param event - Document keyboard event.
     */
    function handleKeyDown(event: KeyboardEvent): void {
      if (event.defaultPrevented || event.metaKey || event.ctrlKey || event.altKey) {
        return;
      }
      if (isEditableTarget(event.target) || document.querySelector('[role="dialog"]')) {
        return;
      }
      const listElement = listRef.current;
      if (!listElement) {
        return;
      }
      if (event.key === '/') {
        const searchElement = listElement.querySelector<HTMLElement>(
          `[${WORKSPACE_SEARCH_ATTRIBUTE}]`,
        );
        if (searchElement) {
          event.preventDefault();
          searchElement.focus();
        }
        return;
      }
      const key = event.key.toLowerCase();
      if (key === 'j' || key === 'k') {
        event.preventDefault();
        moveReaderSelection(listElement, key === 'j' ? 1 : -1, isDocked);
      }
    }

    document.addEventListener('keydown', handleKeyDown);
    return () => document.removeEventListener('keydown', handleKeyDown);
  }, [isDocked]);

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
      <div className="flex h-full min-h-0 w-full">
        <section
          ref={listRef}
          aria-label={listLabel}
          data-slot="workspace-list"
          className={cn(
            'flex h-full min-h-0 w-full min-w-0 flex-col bg-background',
            isListEmphasized
              ? 'xl:w-auto xl:flex-[3_1_0%]'
              : 'xl:w-[460px] xl:shrink-0 xl:border-r 2xl:w-[520px]',
          )}
        >
          <header
            data-slot="workspace-list-header"
            className="z-20 shrink-0 border-b bg-background px-4 pt-4 pb-3 sm:px-5 md:pt-5"
          >
            {header}
          </header>
          {isListSelfScrolling ? (
            <div className={cn('flex min-h-0 flex-1 flex-col', contentClassName)}>{children}</div>
          ) : (
            <div
              id="results-scroll-container"
              className="workspace-bottom-space min-h-0 flex-1 overflow-y-auto overscroll-contain"
            >
              <div className={cn('min-w-0', contentClassName)}>{children}</div>
            </div>
          )}
        </section>

        {isDocked && (
          <section
            ref={deskRef}
            aria-label="阅读台"
            data-slot="reading-desk"
            data-emphasis={emphasis}
            className={cn(
              'relative hidden min-h-0 min-w-0 overflow-y-auto xl:block',
              isListEmphasized
                ? 'flex-[2_1_0%] border-l bg-background'
                : 'flex-1 border-l bg-background',
            )}
          >
            {reader ? (
              reader.content
            ) : (
              <>
                <div ref={setReaderNode} className="contents" />
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
            className="flex flex-col gap-0 overflow-hidden bg-canvas"
            onOpenAutoFocus={() => {
              sheetReturnFocusRef.current =
                document.activeElement instanceof HTMLElement ? document.activeElement : null;
            }}
            onCloseAutoFocus={(event) => {
              const returnTarget = sheetReturnFocusRef.current;
              sheetReturnFocusRef.current = null;
              if (returnTarget?.isConnected) {
                event.preventDefault();
                returnTarget.focus();
              }
            }}
          >
            <div className="flex h-14 shrink-0 items-center gap-2 border-b bg-background px-1.5">
              <DialogClose asChild>
                <Button
                  type="button"
                  variant="ghost"
                  size="icon"
                  static
                  aria-label="关闭"
                  className="size-11"
                >
                  <X className="size-5" aria-hidden="true" />
                </Button>
              </DialogClose>
              <DialogTitle className="min-w-0 truncate text-[15px]">{reader.title}</DialogTitle>
              <DialogDescription className="sr-only">{reader.description}</DialogDescription>
            </div>
            <div data-slot="reader-sheet-body" className="min-h-0 flex-1 overflow-y-auto">
              {reader.content}
            </div>
          </DialogContent>
        </Dialog>
      )}
    </ArticleReaderContext.Provider>
  );
}
