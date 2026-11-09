/**
 * Authenticated application frame rendered around protected child routes.
 */

import { Suspense } from 'react';
import { Outlet } from 'react-router';

import { AppFrame } from '@/components/shell/app-frame';
import {
  LazyAdminCenterDialog,
  LazySettingsCenterDialog,
  WorkspaceViewFallback,
} from '@/components/shell/lazy-workspaces';

/**
 * Render the navigation frame, the routed workspace, and the query-driven centers.
 *
 * @returns Protected application layout.
 */
export default function ProtectedLayout() {
  return (
    <>
      <AppFrame>
        <Suspense fallback={<WorkspaceViewFallback />}>
          <Outlet />
        </Suspense>
      </AppFrame>
      <LazySettingsCenterDialog />
      <LazyAdminCenterDialog />
    </>
  );
}
