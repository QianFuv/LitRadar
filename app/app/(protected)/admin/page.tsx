'use client';

import { useEffect } from 'react';
import Link from 'next/link';
import { useRouter } from 'next/navigation';

import { useAuth } from '@/lib/auth-context';
import { Button } from '@/components/ui/button';

/**
 * Preserve the legacy administrator route as a permission and compatibility boundary.
 *
 * @returns Administrator dashboard page.
 */
export default function AdminPage() {
  const { user } = useAuth();
  const router = useRouter();

  useEffect(() => {
    if (user?.is_admin) {
      router.replace('/?admin=overview');
    }
  }, [router, user?.is_admin]);

  if (!user?.is_admin) {
    return (
      <main
        id="main-content"
        className="flex flex-col items-center justify-center min-h-[60vh] gap-4"
      >
        <p className="text-muted-foreground">无管理员权限</p>
        <Button variant="outline" asChild>
          <Link href="/">返回首页</Link>
        </Button>
      </main>
    );
  }

  return (
    <main id="main-content" className="flex min-h-[60vh] items-center justify-center">
      <div role="status" className="animate-pulse text-muted-foreground">
        正在打开管理面板…
      </div>
    </main>
  );
}
