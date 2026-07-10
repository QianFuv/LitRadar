'use client';

import Link from 'next/link';
import { ArrowLeft, Shield } from 'lucide-react';

import { useAuth } from '@/lib/auth-context';
import { AdminInviteCodesCard } from '@/components/admin/invite-codes-card';
import { AdminOverviewCard } from '@/components/admin/overview-card';
import { AdminUsersCard } from '@/components/admin/users-card';
import { AnnouncementsCard } from '@/components/admin/announcements-card';
import { RuntimeSettingsCard } from '@/components/admin/runtime-settings-card';
import { ScheduledTasksCard } from '@/components/admin/scheduled-tasks-card';
import { Button } from '@/components/ui/button';

/**
 * Compose administrator feature cards behind the administrator permission gate.
 *
 * @returns Administrator dashboard page.
 */
export default function AdminPage() {
  const { user } = useAuth();
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
    <main id="main-content" className="mx-auto max-w-5xl space-y-4 p-4 sm:space-y-6 sm:p-6">
      <div className="flex items-start gap-2 sm:gap-3">
        <Button variant="ghost" size="icon" aria-label="返回首页" asChild>
          <Link href="/">
            <ArrowLeft className="h-5 w-5" />
          </Link>
        </Button>
        <h1 className="flex items-center gap-2 text-xl font-bold sm:text-2xl">
          <Shield className="h-6 w-6" />
          管理面板
        </h1>
      </div>
      <AdminOverviewCard isEnabled />
      <AdminUsersCard currentUserId={user.id} isEnabled />
      <AdminInviteCodesCard isEnabled />
      <RuntimeSettingsCard />
      <ScheduledTasksCard />
      <AnnouncementsCard />
    </main>
  );
}
