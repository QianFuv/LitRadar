'use client';

import { usePathname, useRouter, useSearchParams } from 'next/navigation';
import { Suspense, useEffect } from 'react';
import { UserMenu } from '@/components/feature/user-menu';
import { SettingsCenterDialog } from '@/components/settings/settings-center-dialog';
import { useAuth } from '@/lib/auth-context';

/**
 * Render the protected layout loading state.
 *
 * @returns Protected route loading state.
 */
function ProtectedLayoutFallback() {
  return (
    <main id="main-content" className="flex h-dvh items-center justify-center">
      <div role="status" className="animate-pulse text-muted-foreground">
        加载中…
      </div>
    </main>
  );
}

/**
 * Render authenticated route content and redirect unauthenticated users to login.
 *
 * @param props - Layout props.
 * @returns Protected route content.
 */
function ProtectedLayoutContent({ children }: { children: React.ReactNode }) {
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
    return <ProtectedLayoutFallback />;
  }

  if (!user) return null;

  return (
    <>
      {children}
      <SettingsCenterDialog />
      <UserMenu />
    </>
  );
}

/**
 * Render authenticated routes inside the Suspense boundary required by search params.
 *
 * @param props - Layout props.
 * @returns Protected route layout.
 */
export default function ProtectedLayout({ children }: { children: React.ReactNode }) {
  return (
    <Suspense fallback={<ProtectedLayoutFallback />}>
      <ProtectedLayoutContent>{children}</ProtectedLayoutContent>
    </Suspense>
  );
}
