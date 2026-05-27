'use client';

import { usePathname, useRouter, useSearchParams } from 'next/navigation';
import { Suspense, useEffect } from 'react';
import { useAuthSession } from '@/lib/auth-session';
import { DesktopShell, ShellProvider } from '@/components/desktop/shell';

/**
 * Guard authenticated application routes.
 *
 * @param props - Protected layout props.
 * @returns Protected content or a desktop loading state.
 */
export default function ProtectedLayout({ children }: { children: React.ReactNode }) {
  return (
    <Suspense fallback={<ProtectedLoadingState />}>
      <ProtectedLayoutInner>{children}</ProtectedLayoutInner>
    </Suspense>
  );
}

/**
 * Render the authenticated route guard body that reads URL search params.
 *
 * @param props - Protected layout props.
 * @returns Protected content or null during redirect.
 */
function ProtectedLayoutInner({ children }: { children: React.ReactNode }) {
  const { user, loading } = useAuthSession();
  const pathname = usePathname();
  const router = useRouter();
  const searchParams = useSearchParams();

  useEffect(() => {
    if (!loading && !user) {
      const query = searchParams.toString();
      const nextPath = `${pathname}${query ? `?${query}` : ''}`;
      router.replace(`/login?next=${encodeURIComponent(nextPath)}`);
    }
  }, [loading, pathname, router, searchParams, user]);

  if (loading) {
    return <ProtectedLoadingState />;
  }

  if (!user) return null;

  return (
    <ShellProvider>
      <DesktopShell>{children}</DesktopShell>
    </ShellProvider>
  );
}

/**
 * Render the protected-route loading state.
 *
 * @returns Loading state.
 */
function ProtectedLoadingState() {
  return (
    <div className="desktop-loading">
      <div className="desktop-loading__mark" />
      <div className="desktop-loading__text">正在恢复工作台</div>
    </div>
  );
}
