'use client';

/**
 * Accessible authenticated navigation and theme menu.
 */

import * as DropdownMenuPrimitive from '@radix-ui/react-dropdown-menu';
import {
  CalendarDays,
  Check,
  Home,
  LogOut,
  Monitor,
  Moon,
  Radar,
  Settings,
  Shield,
  Star,
  Sun,
  UserRound,
  type LucideIcon,
} from 'lucide-react';
import { useTheme } from 'next-themes';
import Link from 'next/link';
import { usePathname } from 'next/navigation';
import { useSyncExternalStore, type CSSProperties } from 'react';

import { Button } from '@/components/ui/button';
import { useAuth } from '@/lib/auth-context';
import { cn } from '@/lib/utils';

type NavigationItem = {
  href: string;
  icon: LucideIcon;
  label: string;
};

type ThemePreference = 'system' | 'light' | 'dark';

type ThemeItem = {
  icon: LucideIcon;
  label: string;
  value: ThemePreference;
};

const NAVIGATION_ITEMS: readonly NavigationItem[] = [
  { href: '/', icon: Home, label: '首页' },
  { href: '/favorites', icon: Star, label: '我的收藏' },
  { href: '/tracking', icon: Radar, label: '文献追踪' },
  { href: '/weekly-updates', icon: CalendarDays, label: '每周更新' },
  { href: '/settings', icon: Settings, label: '账号设置' },
];

const ADMIN_NAVIGATION_ITEM: NavigationItem = {
  href: '/admin',
  icon: Shield,
  label: '管理面板',
};

const THEME_ITEMS: readonly ThemeItem[] = [
  { icon: Monitor, label: '跟随系统', value: 'system' },
  { icon: Sun, label: '浅色', value: 'light' },
  { icon: Moon, label: '深色', value: 'dark' },
];

const MENU_ITEM_CLASS =
  "focus:bg-accent focus:text-accent-foreground [&_svg:not([class*='text-'])]:text-muted-foreground relative flex w-full cursor-default items-center gap-2 rounded-sm px-2 py-1.5 text-sm outline-hidden select-none data-[disabled]:pointer-events-none data-[disabled]:opacity-50 [&_svg]:pointer-events-none [&_svg]:shrink-0 [&_svg:not([class*='size-'])]:size-4";

const USER_MENU_POSITION_STYLE: CSSProperties = {
  bottom: 'calc(1rem + var(--safe-area-inset-bottom, env(safe-area-inset-bottom, 0px)))',
  right: 'calc(1rem + env(safe-area-inset-right, 0px))',
};

/**
 * Determine whether a navigation item represents the current route.
 *
 * @param pathname - Current application pathname.
 * @param href - Navigation destination.
 * @returns Whether the destination is current.
 */
function isCurrentRoute(pathname: string, href: string): boolean {
  if (href === '/') {
    return pathname === href;
  }
  return pathname === href || pathname.startsWith(`${href}/`);
}

/**
 * Subscribe to the immutable client-environment signal.
 *
 * @returns No-op unsubscribe callback.
 */
function subscribeToClientEnvironment(): () => void {
  return () => undefined;
}

/**
 * Return the browser snapshot for hydration-safe client detection.
 *
 * @returns Always true in the browser.
 */
function getClientEnvironmentSnapshot(): boolean {
  return true;
}

/**
 * Return the server snapshot for hydration-safe client detection.
 *
 * @returns Always false during server rendering and hydration.
 */
function getServerEnvironmentSnapshot(): boolean {
  return false;
}

/**
 * Render the authenticated global navigation and theme menu.
 *
 * @returns User menu or null while authentication is unresolved.
 */
export function UserMenu() {
  const { user, loading, logout } = useAuth();
  const pathname = usePathname();
  const { setTheme, theme } = useTheme();
  const isMounted = useSyncExternalStore(
    subscribeToClientEnvironment,
    getClientEnvironmentSnapshot,
    getServerEnvironmentSnapshot,
  );

  if (loading || !user) {
    return null;
  }

  const navigationItems = user.is_admin
    ? [...NAVIGATION_ITEMS, ADMIN_NAVIGATION_ITEM]
    : NAVIGATION_ITEMS;
  const selectedTheme = isMounted ? (theme ?? 'system') : 'system';

  /**
   * Clear the authenticated session after the menu selection closes.
   */
  function handleLogout(): void {
    void logout().catch(() => undefined);
  }

  /**
   * Persist the selected application theme.
   *
   * @param value - Selected next-themes preference.
   */
  function handleThemeChange(value: string): void {
    setTheme(value);
  }

  return (
    <div data-slot="user-menu-position" className="fixed z-40" style={USER_MENU_POSITION_STYLE}>
      <DropdownMenuPrimitive.Root>
        <DropdownMenuPrimitive.Trigger asChild>
          <Button
            type="button"
            size="icon-lg"
            className="size-11 rounded-full shadow-lg"
            aria-label="打开用户菜单"
          >
            <UserRound className="size-5" />
          </Button>
        </DropdownMenuPrimitive.Trigger>
        <DropdownMenuPrimitive.Portal>
          <DropdownMenuPrimitive.Content
            aria-label="用户菜单"
            align="end"
            side="top"
            sideOffset={8}
            className="bg-popover text-popover-foreground data-[state=open]:animate-in data-[state=closed]:animate-out data-[state=closed]:fade-out-0 data-[state=open]:fade-in-0 data-[state=closed]:zoom-out-95 data-[state=open]:zoom-in-95 data-[side=top]:slide-in-from-bottom-2 z-50 min-w-52 origin-(--radix-dropdown-menu-content-transform-origin) rounded-md border p-1 shadow-md outline-hidden"
          >
            <DropdownMenuPrimitive.Label className="px-2 py-1.5 text-xs text-muted-foreground">
              {user.username}
            </DropdownMenuPrimitive.Label>
            <DropdownMenuPrimitive.Separator className="bg-border -mx-1 my-1 h-px" />
            {navigationItems.map((item) => {
              const Icon = item.icon;
              const isCurrent = isCurrentRoute(pathname, item.href);

              return (
                <DropdownMenuPrimitive.Item key={item.href} asChild>
                  <Link
                    href={item.href}
                    aria-current={isCurrent ? 'page' : undefined}
                    className={cn(MENU_ITEM_CLASS, isCurrent && 'bg-accent')}
                  >
                    <Icon />
                    <span>{item.label}</span>
                  </Link>
                </DropdownMenuPrimitive.Item>
              );
            })}
            <DropdownMenuPrimitive.Separator className="bg-border -mx-1 my-1 h-px" />
            <DropdownMenuPrimitive.Label className="px-2 py-1.5 text-xs text-muted-foreground">
              主题
            </DropdownMenuPrimitive.Label>
            {isMounted ? (
              <DropdownMenuPrimitive.RadioGroup
                aria-label="主题"
                value={selectedTheme}
                onValueChange={handleThemeChange}
              >
                {THEME_ITEMS.map((item) => {
                  const Icon = item.icon;

                  return (
                    <DropdownMenuPrimitive.RadioItem
                      key={item.value}
                      value={item.value}
                      className={cn(MENU_ITEM_CLASS, 'pr-8')}
                    >
                      <Icon />
                      <span>{item.label}</span>
                      <DropdownMenuPrimitive.ItemIndicator className="absolute right-2 flex size-4 items-center justify-center">
                        <Check className="size-4" />
                      </DropdownMenuPrimitive.ItemIndicator>
                    </DropdownMenuPrimitive.RadioItem>
                  );
                })}
              </DropdownMenuPrimitive.RadioGroup>
            ) : null}
            <DropdownMenuPrimitive.Separator className="bg-border -mx-1 my-1 h-px" />
            <DropdownMenuPrimitive.Item className={MENU_ITEM_CLASS} onSelect={handleLogout}>
              <LogOut />
              <span>退出登录</span>
            </DropdownMenuPrimitive.Item>
          </DropdownMenuPrimitive.Content>
        </DropdownMenuPrimitive.Portal>
      </DropdownMenuPrimitive.Root>
    </div>
  );
}
