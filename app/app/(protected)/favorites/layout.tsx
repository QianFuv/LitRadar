/**
 * Favorites route metadata boundary.
 */

import type { Metadata } from 'next';
import type { ReactNode } from 'react';

export const metadata: Metadata = {
  title: '我的收藏',
  description: '整理、移动和导出已收藏的文献。',
};

/**
 * Preserve the favorites page while providing server-owned metadata.
 *
 * @param props - Layout children.
 * @returns Favorites route content.
 */
export default function FavoritesLayout({ children }: Readonly<{ children: ReactNode }>) {
  return children;
}
