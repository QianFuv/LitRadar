/**
 * Literature tracking route metadata boundary.
 */

import type { Metadata } from 'next';
import type { ReactNode } from 'react';

export const metadata: Metadata = {
  title: '文献追踪',
  description: '配置文献推荐、通知和每周追踪推送。',
};

/**
 * Preserve the tracking page while providing server-owned metadata.
 *
 * @param props - Layout children.
 * @returns Tracking route content.
 */
export default function TrackingLayout({ children }: Readonly<{ children: ReactNode }>) {
  return children;
}
