'use client';

/**
 * URL-backed switch between article favorites and saved relation graphs.
 */

import { FileText, Network } from 'lucide-react';
import { parseAsStringLiteral, useQueryState } from 'nuqs';

import { ToggleGroup, ToggleGroupItem } from '@/components/ui/toggle-group';

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
 * @returns Segmented toggle group bound to the URL.
 */
export function FavoriteKindSwitch() {
  const [kind, setKind] = useFavoriteKind();

  return (
    <ToggleGroup
      type="single"
      aria-label="收藏类型"
      value={kind}
      onValueChange={(option) => {
        if (option) void setKind(option === 'articles' ? null : (option as typeof kind));
      }}
    >
      {FAVORITE_KIND_OPTIONS.map(({ kind: option, label, icon: Icon }) => (
        <ToggleGroupItem key={option} value={option}>
          <Icon className="size-3.5" aria-hidden="true" />
          {label}
        </ToggleGroupItem>
      ))}
    </ToggleGroup>
  );
}
