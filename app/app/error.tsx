'use client';

/**
 * Root route error boundary rendered inside the normal application layout.
 */

import Link from 'next/link';
import { useEffect } from 'react';

import { Button } from '@/components/ui/button';
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/components/ui/card';

type RouteErrorProps = {
  error: Error & { digest?: string };
  reset: () => void;
};

/**
 * Report a route failure safely and offer retry and home navigation.
 *
 * @param props - Captured error and boundary reset callback.
 * @returns Accessible route failure UI.
 */
export default function RouteError({ error, reset }: RouteErrorProps) {
  useEffect(() => {
    console.error('LitRadar route error', {
      digest: error.digest,
      name: error.name,
    });
  }, [error]);

  return (
    <main id="main-content" className="flex min-h-dvh items-center justify-center p-6">
      <Card role="alert" className="w-full max-w-md">
        <CardHeader>
          <CardTitle>页面加载失败</CardTitle>
          <CardDescription>发生了意外错误。你可以重试，或返回首页继续使用。</CardDescription>
        </CardHeader>
        <CardContent className="flex flex-col gap-3 sm:flex-row">
          <Button type="button" onClick={reset}>
            重试
          </Button>
          <Button variant="outline" asChild>
            <Link href="/">返回首页</Link>
          </Button>
        </CardContent>
      </Card>
    </main>
  );
}
