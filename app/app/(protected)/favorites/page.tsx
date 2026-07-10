'use client';

import Link from 'next/link';

import { FavoritesPageContent } from '@/components/favorites/favorites-page-content';
import { Button } from '@/components/ui/button';
import { useAuth } from '@/lib/auth-context';

/**
 * Gate the favorites feature on authentication.
 *
 * @returns Favorites page or login prompt.
 */
export default function FavoritesPage() {
  const { user } = useAuth();
  if (!user) {
    return (
      <main
        id="main-content"
        className="flex flex-col items-center justify-center min-h-[60vh] gap-4"
      >
        <p className="text-muted-foreground">请先登录</p>
        <Button asChild>
          <Link href="/login?next=/favorites">登录</Link>
        </Button>
      </main>
    );
  }
  return <FavoritesPageContent userId={user.id} />;
}
