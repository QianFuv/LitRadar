'use client';

import Link from 'next/link';

import { TrackingPageContent } from '@/components/tracking/tracking-page-content';
import { Button } from '@/components/ui/button';
import { useAuth } from '@/lib/auth-context';

/**
 * Gate the tracking feature on authentication.
 *
 * @returns Tracking page or login prompt.
 */
export default function TrackingPage() {
  const { user } = useAuth();
  if (!user) {
    return (
      <main
        id="main-content"
        className="flex flex-col items-center justify-center min-h-[60vh] gap-4"
      >
        <p className="text-muted-foreground">请先登录</p>
        <Button asChild>
          <Link href="/login?next=/tracking">登录</Link>
        </Button>
      </main>
    );
  }
  return <TrackingPageContent userId={user.id} />;
}
