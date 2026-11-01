'use client';

/**
 * URL-backed switch between article favorites and saved relation graphs.
 */

import { FileText, Network } from 'lucide-react';
import { parseAsStringLiteral, useQueryState } from 'nuqs';

import { cn } from '@/lib/utils';

/** Favorite kinds shown on the favorites page. */
export const FAVORITE_KINDS = ['articles', 'graphs'] as const;

/** One favorite kind shown on the favorites page. */
export type FavoriteKind = (typeof FAVORITE_KINDS)[number];

const FAVORITE_KIND_PARSER = parseAsStringLiteral(FAVORITE_KINDS).withDefault('articles');

const FAVORITE_KIND_OPTIONS = [
  { kind: 'articles', label: '文章', icon: FileText },
  { kind: 'graphs', label: '关系图', icon: Network },
] as const;

/**
 * Read and change the favorite kind kept in the `kind` URL parameter.
 *
 * @returns Current kind and its setter.
 */
export function useFavoriteKind() {
  return useQueryState('kind', FAVORITE_KIND_PARSER);
}

/**
 * Render the article / relation graph switch of the favorites page.
 *
 * @returns Segmented radio group bound to the URL.
 */
export function FavoriteKindSwitch() {
  const [kind, setKind] = useFavoriteKind();

  return (
    <div role="radiogroup" aria-label="收藏类型" className="flex w-fit rounded-lg border p-0.5">
      {FAVORITE_KIND_OPTIONS.map(({ kind: option, label, icon: Icon }) => (
        <button
          key={option}
          type="button"
          role="radio"
          aria-checked={kind === option}
          className={cn(
            'motion-control flex h-10 items-center gap-1 rounded-md px-3 text-xs outline-none transition-colors focus-visible:ring-2 focus-visible:ring-ring md:h-7 md:px-2.5',
            kind === option
              ? 'bg-foreground text-background'
              : 'text-muted-foreground hover:text-foreground',
          )}
          onClick={() => void setKind(option === 'articles' ? null : option)}
        >
          <Icon className="size-3.5" aria-hidden="true" />
          {label}
        </button>
      ))}
    </div>
  );
}
