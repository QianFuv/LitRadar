'use client';

/**
 * Authenticated application frame: desktop navigation rail, main region, and mobile tab bar.
 */

import { Settings2, Shield, type LucideIcon } from 'lucide-react';
import Image from 'next/image';
import Link from 'next/link';
import { usePathname, useSearchParams } from 'next/navigation';
import type { MouseEvent, ReactNode } from 'react';

import { UserMenu } from '@/components/feature/user-menu';
import { SECTIONED_DIALOG_RETURN_FOCUS_ATTRIBUTE } from '@/components/feature/sectioned-dialog';
import { RailNavigation, TabNavigation } from '@/components/shell/primary-navigation';
import { buildAdminCenterHref, parseAdminSection } from '@/lib/admin-center';
import { useAuth } from '@/lib/auth-context';
import { buildSettingsCenterHref, parseSettingsSection } from '@/lib/settings-center';
import { cn } from '@/lib/utils';

/**
 * Mark a rail shortcut as the focus target restored after its sectioned dialog closes.
 *
 * @param event - Plain current-tab click on the shortcut.
 */
function markReturnFocus(event: MouseEvent<HTMLAnchorElement>): void {
  if (event.button !== 0 || event.metaKey || event.ctrlKey || event.shiftKey || event.altKey) {
    return;
  }
  event.currentTarget.setAttribute(SECTIONED_DIALOG_RETURN_FOCUS_ATTRIBUTE, '');
}

/**
 * Render one icon shortcut in the lower rail group.
 *
 * @param props - Destination, icon, accessible label, and current state.
 * @returns Rail shortcut link.
 */
function RailShortcut({
  href,
  icon: Icon,
  isCurrent,
  label,
}: {
  href: string;
  icon: LucideIcon;
  isCurrent: boolean;
  label: string;
}) {
  return (
    <Link
      href={href}
      prefetch={false}
      aria-label={label}
      aria-current={isCurrent ? 'page' : undefined}
      title={label}
      onClick={markReturnFocus}
      className={cn(
        'motion-control flex size-10 items-center justify-center rounded-xl text-rail-foreground outline-none transition-[background-color,color] hover:bg-rail-hover hover:text-rail-active focus-visible:ring-2 focus-visible:ring-ring',
        isCurrent && 'bg-rail-hover text-rail-active',
      )}
    >
      <Icon className="size-5" strokeWidth={1.7} aria-hidden="true" />
    </Link>
  );
}

/**
 * Render the dark desktop rail with brand, workspaces, shortcuts, and account menu.
 *
 * @returns Desktop navigation rail.
 */
function AppRail() {
  const { user } = useAuth();
  const pathname = usePathname();
  const searchParams = useSearchParams();
  const isSettingsOpen = parseSettingsSection(searchParams.get('settings')) !== null;
  const isAdminOpen = parseAdminSection(searchParams.get('admin')) !== null;

  return (
    <aside
      data-slot="app-rail"
      className="hidden h-dvh w-16 shrink-0 flex-col items-center border-r border-rail-border bg-rail py-3 md:flex"
    >
      <Link
        href="/"
        prefetch={false}
        aria-label="LitRadar 首页"
        title="LitRadar"
        className="motion-control mb-5 rounded-xl outline-none transition-opacity hover:opacity-85 focus-visible:ring-2 focus-visible:ring-ring"
      >
        <Image
          src="/litradar-logo.png"
          alt=""
          width={36}
          height={36}
          loading="eager"
          fetchPriority="high"
          className="size-9 rounded-[10px] object-cover"
        />
      </Link>

      <RailNavigation />

      <div className="mt-auto flex flex-col items-center gap-1.5">
        <RailShortcut
          href={buildSettingsCenterHref(pathname, searchParams, 'general')}
          icon={Settings2}
          isCurrent={isSettingsOpen}
          label="设置中心"
        />
        {user?.is_admin && (
          <RailShortcut
            href={buildAdminCenterHref(pathname, searchParams, 'overview')}
            icon={Shield}
            isCurrent={isAdminOpen}
            label="管理面板"
          />
        )}
        <div className="mt-2 border-t border-rail-border pt-3">
          <UserMenu placement="rail" />
        </div>
      </div>
    </aside>
  );
}

/**
 * Render the fixed mobile tab bar with workspace links and the account entry.
 *
 * @returns Mobile tab bar.
 */
function MobileTabBar() {
  return (
    <div
      data-slot="mobile-tab-bar"
      className="mobile-tab-safe-area fixed inset-x-0 bottom-0 z-40 border-t bg-background md:hidden"
    >
      <div className="grid grid-cols-6">
        <TabNavigation />
        <UserMenu placement="tab" />
      </div>
    </div>
  );
}

/**
 * Compose authenticated pages inside the rail, main landmark, and mobile tab bar.
 *
 * @param props - Page content.
 * @returns Application frame.
 */
export function AppFrame({ children }: { children: ReactNode }) {
  return (
    <div className="flex h-dvh w-full overflow-hidden bg-background text-foreground">
      <AppRail />
      <main id="main-content" className="relative flex h-dvh min-w-0 flex-1 flex-col">
        {children}
      </main>
      <MobileTabBar />
    </div>
  );
}
