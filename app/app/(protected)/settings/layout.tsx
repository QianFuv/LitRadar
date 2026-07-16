/**
 * Account settings route metadata boundary.
 */

import type { Metadata } from 'next';
import type { ReactNode } from 'react';

export const metadata: Metadata = {
  title: '账号设置',
  description: '管理 LitRadar 账号、安全设置、访问令牌和知网会话。',
};

/**
 * Preserve the settings page while providing server-owned metadata.
 *
 * @param props - Layout children.
 * @returns Settings route content.
 */
export default function SettingsLayout({ children }: Readonly<{ children: ReactNode }>) {
  return children;
}
