'use client';

import { usePathname, useRouter, useSearchParams } from 'next/navigation';
import { useEffect } from 'react';
import { useAuth } from '@/lib/auth-context';

/**
 * Render authenticated routes and redirect unauthenticated users to login.
 *
 * @param props - Layout props.
 * @returns Protected route layout.
 */
export default function ProtectedLayout({ children }: { children: React.ReactNode }) {
  const { user, loading } = useAuth();
  const router = useRouter();
  const pathname = usePathname();
  const searchParams = useSearchParams();
  const search = searchParams.toString();

  useEffect(() => {
    if (!loading && !user) {
      const nextPath = search ? `${pathname}?${search}` : pathname;
      router.replace(`/login?next=${encodeURIComponent(nextPath)}`);
    }
  }, [loading, pathname, router, search, user]);

  if (loading) {
    return (
      <main id="main-content" className="flex h-screen items-center justify-center">
        <div role="status" className="animate-pulse text-muted-foreground">
          加载中...
        </div>
      </main>
    );
  }

  if (!user) return null;

  return <>{children}</>;
}
