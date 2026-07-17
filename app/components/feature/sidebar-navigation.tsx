'use client';

/**
 * Compact icon-only navigation for the search sidebar.
 */

import { CalendarDays, Search, Star, type LucideIcon } from 'lucide-react';
import Link from 'next/link';
import { usePathname } from 'next/navigation';

import { cn } from '@/lib/utils';

type SidebarNavigationItem = {
  href: string;
  icon: LucideIcon;
  label: string;
};

const SIDEBAR_NAVIGATION_ITEMS: readonly SidebarNavigationItem[] = [
  { href: '/', icon: Search, label: '文献检索' },
  { href: '/favorites', icon: Star, label: '我的收藏' },
  { href: '/weekly-updates', icon: CalendarDays, label: '每周更新' },
];

/**
 * Determine whether one sidebar destination represents the current route.
 *
 * @param pathname - Current application pathname.
 * @param href - Sidebar navigation destination.
 * @returns Whether the destination is current.
 */
function isCurrentRoute(pathname: string, href: string): boolean {
  if (href === '/') {
    return pathname === href;
  }
  return pathname === href || pathname.startsWith(`${href}/`);
}

/**
 * Render the three primary page destinations as equal-width icon links.
 *
 * @returns Accessible compact sidebar navigation.
 */
export function SidebarNavigation() {
  const pathname = usePathname();

  return (
    <nav aria-label="页面导航" data-slot="sidebar-navigation" className="grid grid-cols-3 gap-2">
      {SIDEBAR_NAVIGATION_ITEMS.map((item) => {
        const Icon = item.icon;
        const isCurrent = isCurrentRoute(pathname, item.href);

        return (
          <Link
            key={item.href}
            href={item.href}
            aria-label={item.label}
            aria-current={isCurrent ? 'page' : undefined}
            title={item.label}
            className={cn(
              'flex h-10 items-center justify-center rounded-md border border-transparent outline-none transition-colors hover:bg-sidebar-accent hover:text-sidebar-accent-foreground focus-visible:ring-[3px] focus-visible:ring-sidebar-ring/50',
              isCurrent && 'border-sidebar-border bg-sidebar-accent text-sidebar-accent-foreground',
            )}
          >
            <Icon className="size-4.5" aria-hidden="true" />
            <span className="sr-only">{item.label}</span>
          </Link>
        );
      })}
    </nav>
  );
}
