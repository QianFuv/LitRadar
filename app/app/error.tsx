'use client';

/**
 * Root route error boundary rendered inside the normal application layout.
 */

import Link from 'next/link';
import { useEffect } from 'react';

import { Button } from '@/components/ui/button';
import { RadarMark } from '@/components/feature/reader-placeholder';
import { reportClientError } from '@/lib/client-logger';

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
  useEffect(
    /**
     * Emit the route-boundary terminal event once for this error object.
     */
    function reportRouteError(): void {
      reportClientError('route_boundary', error, { digest: error.digest });
    },
    [error],
  );

  return (
    <main
      id="main-content"
      className="flex min-h-dvh flex-col items-center justify-center bg-background px-6 py-16 text-center"
    >
      <RadarMark className="size-40 text-muted-foreground" />
      <div role="alert" className="mt-8">
        <h1 className="text-2xl font-bold tracking-tight">页面加载失败</h1>
        <p className="mt-2 max-w-sm text-sm leading-6 text-muted-foreground">
          发生了意外错误。你可以重试，或返回首页继续使用。
        </p>
      </div>
      <div className="mt-8 flex flex-col gap-3 sm:flex-row">
        <Button type="button" className="h-10 rounded-xl px-5" onClick={reset}>
          重试
        </Button>
        <Button variant="outline" className="h-10 rounded-xl px-5" asChild>
          <Link href="/" prefetch={false}>
            返回首页
          </Link>
        </Button>
      </div>
    </main>
  );
}
