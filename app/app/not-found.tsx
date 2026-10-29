/**
 * Static custom not-found route and metadata.
 */

import type { Metadata } from 'next';
import Link from 'next/link';

import { Button } from '@/components/ui/button';
import { RadarMark } from '@/components/feature/reader-placeholder';

export const metadata: Metadata = {
  title: '页面未找到',
  description: '所请求的 LitRadar 页面不存在。',
};

/**
 * Render the custom application not-found page.
 *
 * @returns Accessible not-found content with home navigation.
 */
export default function NotFound() {
  return (
    <main
      id="main-content"
      className="flex min-h-dvh flex-col items-center justify-center bg-background px-6 py-16 text-center"
    >
      <RadarMark className="size-40 text-muted-foreground" />
      <p className="mt-8 text-xs font-bold tracking-[0.2em] text-primary-text">404 · 信号丢失</p>
      <h1 className="mt-3 text-2xl font-bold tracking-tight">页面未找到</h1>
      <p className="mt-2 max-w-sm text-sm leading-6 text-muted-foreground">
        你访问的页面不存在，可能已被移动或删除。
      </p>
      <Button asChild className="mt-8 h-10 rounded-xl px-5">
        <Link href="/" prefetch={false}>
          返回首页
        </Link>
      </Button>
    </main>
  );
}
