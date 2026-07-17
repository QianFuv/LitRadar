'use client';

/**
 * Compact icon-only navigation for the search sidebar.
 */

import { CalendarDays, Search, Star, type LucideIcon } from 'lucide-react';
import Link from 'next/link';
import { useQueryState } from 'nuqs';

import { cn } from '@/lib/utils';
import {
  getWorkspaceViewHref,
  WORKSPACE_VIEW_PARSER,
  type WorkspaceView,
} from '@/lib/workspace-view';

type SidebarNavigationItem = {
  icon: LucideIcon;
  label: string;
  view: WorkspaceView;
};

const SIDEBAR_NAVIGATION_ITEMS: readonly SidebarNavigationItem[] = [
  { view: 'search', icon: Search, label: '文献检索' },
  { view: 'favorites', icon: Star, label: '我的收藏' },
  { view: 'weekly-updates', icon: CalendarDays, label: '每周更新' },
];

/**
 * Render the three root-workspace views as equal-width icon links.
 *
 * @returns Accessible compact sidebar navigation.
 */
export function SidebarNavigation() {
  const [view] = useQueryState('view', WORKSPACE_VIEW_PARSER);

  return (
    <nav aria-label="页面导航" data-slot="sidebar-navigation" className="grid grid-cols-3 gap-2">
      {SIDEBAR_NAVIGATION_ITEMS.map((item) => {
        const Icon = item.icon;
        const isCurrent = item.view === view;

        return (
          <Link
            key={item.view}
            href={getWorkspaceViewHref(item.view)}
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
