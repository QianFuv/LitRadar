'use client';

/**
 * Compact labeled navigation for the three article workspaces.
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
  shortLabel: string;
  view: WorkspaceView;
};

const SIDEBAR_NAVIGATION_ITEMS: readonly SidebarNavigationItem[] = [
  { view: 'search', icon: Search, label: '文献检索', shortLabel: '检索' },
  { view: 'favorites', icon: Star, label: '我的收藏', shortLabel: '收藏' },
  { view: 'weekly-updates', icon: CalendarDays, label: '每周更新', shortLabel: '周报' },
];

/**
 * Render the three root-workspace views as equal-width labeled links.
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
              'motion-control flex min-h-12 flex-col items-center justify-center gap-1 rounded-md border border-transparent px-1 py-2 text-muted-foreground outline-none transition-[background-color,border-color,color,box-shadow] hover:bg-sidebar-accent hover:text-sidebar-accent-foreground focus-visible:ring-[3px] focus-visible:ring-sidebar-ring/50',
              isCurrent &&
                'border-sidebar-border bg-sidebar-accent text-sidebar-accent-foreground shadow-vercel-ring',
            )}
          >
            <Icon className="size-4" aria-hidden="true" />
            <span className="text-[11px] font-medium leading-none tracking-tight">
              {item.shortLabel}
            </span>
          </Link>
        );
      })}
    </nav>
  );
}
