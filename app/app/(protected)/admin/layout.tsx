/**
 * Administrator route metadata boundary.
 */

import type { Metadata } from 'next';
import type { ReactNode } from 'react';

export const metadata: Metadata = {
  title: '管理面板',
  description: '管理 LitRadar 用户、邀请码、运行设置、计划任务和公告。',
};

/**
 * Preserve the administrator page while providing server-owned metadata.
 *
 * @param props - Layout children.
 * @returns Administrator route content.
 */
export default function AdminLayout({ children }: Readonly<{ children: ReactNode }>) {
  return children;
}
