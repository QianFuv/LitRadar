/**
 * Weekly updates route metadata boundary.
 */

import type { Metadata } from 'next';
import type { ReactNode } from 'react';

export const metadata: Metadata = {
  title: '每周更新',
  description: '按数据库和期刊浏览每周新增文献。',
};

/**
 * Preserve the weekly updates page while providing server-owned metadata.
 *
 * @param props - Layout children.
 * @returns Weekly updates route content.
 */
export default function WeeklyUpdatesLayout({ children }: Readonly<{ children: ReactNode }>) {
  return children;
}
