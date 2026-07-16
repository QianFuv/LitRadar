/**
 * Static custom not-found route and metadata.
 */

import type { Metadata } from 'next';
import Link from 'next/link';

import { Button } from '@/components/ui/button';
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/components/ui/card';

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
    <main id="main-content" className="flex min-h-dvh items-center justify-center p-6">
      <Card className="w-full max-w-md text-center">
        <CardHeader>
          <CardTitle>页面未找到</CardTitle>
          <CardDescription>你访问的页面不存在，可能已被移动或删除。</CardDescription>
        </CardHeader>
        <CardContent>
          <Button asChild>
            <Link href="/">返回首页</Link>
          </Button>
        </CardContent>
      </Card>
    </main>
  );
}
