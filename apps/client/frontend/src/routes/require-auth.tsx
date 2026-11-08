/**
 * Authenticated route guard that renders the protected application frame.
 */

import { Navigate, Outlet, useLocation } from 'react-router';

import { AdminCenterDialog } from '@/components/admin/admin-center-dialog';
import { SettingsCenterDialog } from '@/components/settings/settings-center-dialog';
import { AppFrame } from '@/components/shell/app-frame';
import { LoadingText } from '@/components/ui/loading-text';
import { useAuth } from '@/lib/auth-context';

/**
 * Render the protected layout loading state.
 *
 * @returns Protected route loading state.
 */
function ProtectedLayoutFallback() {
  return (
    <main id="main-content" className="flex h-dvh items-center justify-center">
      <div role="status" className="text-muted-foreground">
        <LoadingText text="加载中…" />
      </div>
    </main>
  );
}

/**
 * Render authenticated child routes and redirect unauthenticated users to login.
 *
 * @returns Protected route content, loading state, or login redirect.
 */
export default function RequireAuth() {
  const { user, loading } = useAuth();
  const location = useLocation();

  if (loading) {
    return <ProtectedLayoutFallback />;
  }

  if (!user) {
    const nextPath = `${location.pathname}${location.search}`;
    return <Navigate to={`/login?next=${encodeURIComponent(nextPath)}`} replace />;
  }

  return (
    <>
      <AppFrame>
        <Outlet />
      </AppFrame>
      <SettingsCenterDialog />
      <AdminCenterDialog />
    </>
  );
}
