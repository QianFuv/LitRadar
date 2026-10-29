'use client';

/**
 * Selection state shared between list items and the docked reader pane.
 */

import { createContext, useContext } from 'react';

/** Attribute that marks keyboard-navigable list items. */
export const READER_ITEM_ATTRIBUTE = 'data-reader-item';

/** Attribute that marks the element focused by the slash shortcut. */
export const WORKSPACE_SEARCH_ATTRIBUTE = 'data-workspace-search';

/** Reader coordination exposed by a workspace shell to its list items. */
export type ArticleReaderContextValue = {
  /** Whether the reader is docked beside the list instead of opening as a sheet. */
  isDocked: boolean;
  /** DOM node that receives the selected item's reader content while docked. */
  readerNode: HTMLElement | null;
  /** Stable key of the item currently shown in the docked reader. */
  selectedKey: string | null;
  /** Show one item in the docked reader. */
  select: (key: string) => void;
  /** Clear the docked reader when the given item is still selected. */
  release: (key: string) => void;
};

export const ArticleReaderContext = createContext<ArticleReaderContextValue | null>(null);

/**
 * Read the surrounding workspace reader, if any.
 *
 * @returns Reader coordination or null outside a workspace shell.
 */
export function useArticleReader(): ArticleReaderContextValue | null {
  return useContext(ArticleReaderContext);
}
