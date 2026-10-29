'use client';

/**
 * Primary workspace navigation rendered as a desktop rail or a mobile tab bar.
 */

import { CalendarDays, Megaphone, Search, Star, type LucideIcon } from 'lucide-react';
import Link from 'next/link';
import { useSearchParams } from 'next/navigation';

import { cn } from '@/lib/utils';
import {
  getWorkspaceViewHref,
  WORKSPACE_VIEW_PARSER,
  type WorkspaceView,
} from '@/lib/workspace-view';

type PrimaryNavigationItem = {
  icon: LucideIcon;
  label: string;
  shortLabel: string;
  view: WorkspaceView;
};

/** Root workspaces in navigation order. */
const PRIMARY_NAVIGATION_ITEMS: readonly PrimaryNavigationItem[] = [
  { view: 'search', icon: Search, label: '文献检索', shortLabel: '检索' },
  { view: 'favorites', icon: Star, label: '我的收藏', shortLabel: '收藏' },
  { view: 'weekly-updates', icon: CalendarDays, label: '每周更新', shortLabel: '周报' },
  { view: 'cfp-tracking', icon: Megaphone, label: '征稿追踪', shortLabel: '征稿' },
];

/**
 * Read the current root workspace from the URL without requiring a nuqs adapter.
 *
 * @returns Current workspace view, defaulting to search.
 */
function useCurrentWorkspaceView(): WorkspaceView {
  const searchParams = useSearchParams();
  return WORKSPACE_VIEW_PARSER.parseServerSide(searchParams.get('view') ?? undefined);
}

/**
 * Render the vertical workspace navigation used inside the desktop rail.
 *
 * @returns Accessible rail navigation.
 */
export function RailNavigation() {
  const view = useCurrentWorkspaceView();

  return (
    <nav aria-label="页面导航" className="flex flex-col items-center gap-1.5">
      {PRIMARY_NAVIGATION_ITEMS.map((item) => {
        const Icon = item.icon;
        const isCurrent = item.view === view;

        return (
          <Link
            key={item.view}
            href={getWorkspaceViewHref(item.view)}
            prefetch={false}
            aria-label={item.label}
            aria-current={isCurrent ? 'page' : undefined}
            title={item.label}
            className={cn(
              'motion-control relative flex w-12 flex-col items-center gap-1 rounded-xl py-2 text-rail-foreground outline-none transition-[background-color,color] hover:bg-rail-hover hover:text-rail-active focus-visible:ring-2 focus-visible:ring-ring',
              isCurrent && 'bg-rail-hover text-rail-active',
            )}
          >
            {isCurrent && (
              <span
                aria-hidden="true"
                className="absolute top-1/2 -left-2 h-6 w-[3px] -translate-y-1/2 rounded-r-full bg-primary"
              />
            )}
            <Icon className="size-5" strokeWidth={isCurrent ? 2.1 : 1.7} aria-hidden="true" />
            <span className="text-[10px] leading-none">{item.shortLabel}</span>
          </Link>
        );
      })}
    </nav>
  );
}

/**
 * Render the workspace links of the mobile bottom tab bar.
 *
 * @returns Accessible tab-bar navigation links.
 */
export function TabNavigation() {
  const view = useCurrentWorkspaceView();

  return (
    <nav aria-label="页面导航" className="col-span-4 grid grid-cols-4">
      {PRIMARY_NAVIGATION_ITEMS.map((item) => {
        const Icon = item.icon;
        const isCurrent = item.view === view;

        return (
          <Link
            key={item.view}
            href={getWorkspaceViewHref(item.view)}
            prefetch={false}
            aria-label={item.label}
            aria-current={isCurrent ? 'page' : undefined}
            className={cn(
              'motion-control flex h-14 flex-col items-center justify-center gap-1 text-muted-foreground outline-none transition-colors focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-inset',
              isCurrent && 'text-primary-text',
            )}
          >
            <Icon className="size-5" strokeWidth={isCurrent ? 2.1 : 1.7} aria-hidden="true" />
            <span className={cn('text-[11px] leading-none', isCurrent && 'font-bold')}>
              {item.shortLabel}
            </span>
          </Link>
        );
      })}
    </nav>
  );
}
