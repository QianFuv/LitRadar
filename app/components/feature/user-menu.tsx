'use client';

/**
 * Accessible account controls, settings entry, theme preferences, and logout.
 */

import * as DropdownMenuPrimitive from '@radix-ui/react-dropdown-menu';
import {
  Check,
  ChevronRight,
  ChevronUp,
  LogOut,
  Monitor,
  Moon,
  Settings2,
  Shield,
  Sun,
  type LucideIcon,
} from 'lucide-react';
import { useTheme } from 'next-themes';
import Image from 'next/image';
import Link from 'next/link';
import { usePathname, useSearchParams } from 'next/navigation';
import { useRef, useSyncExternalStore, type CSSProperties, type MouseEvent } from 'react';

import { Button } from '@/components/ui/button';
import { SECTIONED_DIALOG_RETURN_FOCUS_ATTRIBUTE } from '@/components/feature/sectioned-dialog';
import { useAuth } from '@/lib/auth-context';
import { buildSettingsCenterHref } from '@/lib/settings-center';
import { cn } from '@/lib/utils';

type ThemePreference = 'system' | 'light' | 'dark';

type ThemeItem = {
  icon: LucideIcon;
  label: string;
  value: ThemePreference;
};

const THEME_ITEMS: readonly ThemeItem[] = [
  { icon: Monitor, label: '跟随系统', value: 'system' },
  { icon: Sun, label: '浅色', value: 'light' },
  { icon: Moon, label: '深色', value: 'dark' },
];

const MENU_ITEM_CLASS =
  "focus:bg-accent focus:text-accent-foreground [&_svg:not([class*='text-'])]:text-muted-foreground relative flex w-full cursor-default items-center gap-2 rounded-sm px-2 py-1.5 text-sm outline-hidden select-none data-[disabled]:pointer-events-none data-[disabled]:opacity-50 [&_svg]:pointer-events-none [&_svg]:shrink-0 [&_svg:not([class*='size-'])]:size-4";

const MENU_CONTENT_CLASS =
  'bg-popover text-popover-foreground data-[state=open]:animate-in data-[state=closed]:animate-out data-[state=closed]:fade-out-0 data-[state=open]:fade-in-0 data-[state=closed]:zoom-out-95 data-[state=open]:zoom-in-95 z-50 origin-(--radix-dropdown-menu-content-transform-origin) rounded-md border p-1 shadow-md outline-hidden';

const USER_MENU_POSITION_STYLE: CSSProperties = {
  bottom: 'calc(1rem + var(--safe-area-inset-bottom, env(safe-area-inset-bottom, 0px)))',
  right: 'calc(1rem + env(safe-area-inset-right, 0px))',
};

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
 * Render the authenticated account trigger and account-only menu.
 *
 * @returns Account menu or null while authentication is unresolved.
 */
export function UserMenu() {
  const { user, loading, logout } = useAuth();
  const pathname = usePathname();
  const searchParams = useSearchParams();
  const { setTheme, theme } = useTheme();
  const accountTriggerRef = useRef<HTMLButtonElement>(null);
  const isMounted = useSyncExternalStore(
    subscribeToClientEnvironment,
    getClientEnvironmentSnapshot,
    getServerEnvironmentSnapshot,
  );

  if (loading || !user) {
    return null;
  }

  const selectedTheme = isMounted ? (theme ?? 'system') : 'system';
  const selectedThemeLabel =
    THEME_ITEMS.find((item) => item.value === selectedTheme)?.label ?? '跟随系统';
  const settingsHref = buildSettingsCenterHref(pathname, searchParams, 'general');
  const isAdminRoute = pathname === '/admin' || pathname.startsWith('/admin/');

  /**
   * Clear the authenticated session after the menu selection closes.
   */
  function handleLogout(): void {
    void logout().catch(() => undefined);
  }

  /**
   * Mark the persistent account trigger before current-tab settings navigation.
   *
   * @param event - Settings-link click event.
   */
  function handleSettingsOpen(event: MouseEvent<HTMLAnchorElement>): void {
    if (event.button !== 0 || event.metaKey || event.ctrlKey || event.shiftKey || event.altKey) {
      return;
    }
    accountTriggerRef.current?.setAttribute(SECTIONED_DIALOG_RETURN_FOCUS_ATTRIBUTE, '');
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
            ref={accountTriggerRef}
            type="button"
            variant="outline"
            className="h-12 max-w-[min(15rem,calc(100vw-2rem))] gap-2 rounded-full bg-popover px-2.5 text-popover-foreground shadow-lg hover:bg-accent hover:text-accent-foreground"
            aria-label={`打开账号菜单：${user.username}`}
          >
            <Image
              src="/litradar-logo.png"
              alt=""
              width={32}
              height={32}
              className="size-8 shrink-0 rounded-full object-cover"
            />
            <span className="min-w-0 truncate text-sm font-medium">{user.username}</span>
            <ChevronUp className="size-4 shrink-0 text-muted-foreground" aria-hidden="true" />
          </Button>
        </DropdownMenuPrimitive.Trigger>

        <DropdownMenuPrimitive.Portal>
          <DropdownMenuPrimitive.Content
            aria-label="账号菜单"
            align="end"
            side="top"
            sideOffset={8}
            className={cn(MENU_CONTENT_CLASS, 'w-60')}
          >
            <DropdownMenuPrimitive.Label className="flex items-center gap-3 px-2 py-2">
              <Image
                src="/litradar-logo.png"
                alt=""
                width={36}
                height={36}
                className="size-9 shrink-0 rounded-full object-cover"
              />
              <span className="min-w-0">
                <span className="block truncate text-sm font-semibold">{user.username}</span>
                <span className="block text-xs font-normal text-muted-foreground">
                  {user.is_admin ? '管理员' : '普通用户'}
                </span>
              </span>
            </DropdownMenuPrimitive.Label>

            <DropdownMenuPrimitive.Separator className="-mx-1 my-1 h-px bg-border" />

            <DropdownMenuPrimitive.Item asChild>
              <Link href={settingsHref} className={MENU_ITEM_CLASS} onClick={handleSettingsOpen}>
                <Settings2 />
                <span>打开设置中心</span>
              </Link>
            </DropdownMenuPrimitive.Item>

            <DropdownMenuPrimitive.Sub>
              <DropdownMenuPrimitive.SubTrigger
                aria-label="外观主题"
                className={cn(MENU_ITEM_CLASS, 'data-[state=open]:bg-accent')}
              >
                <Monitor />
                <span>外观主题</span>
                <span className="ml-auto text-xs text-muted-foreground">{selectedThemeLabel}</span>
                <ChevronRight className="size-4" />
              </DropdownMenuPrimitive.SubTrigger>
              <DropdownMenuPrimitive.Portal>
                <DropdownMenuPrimitive.SubContent
                  alignOffset={-4}
                  sideOffset={6}
                  className={cn(MENU_CONTENT_CLASS, 'min-w-40')}
                >
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
                </DropdownMenuPrimitive.SubContent>
              </DropdownMenuPrimitive.Portal>
            </DropdownMenuPrimitive.Sub>

            {user.is_admin && (
              <DropdownMenuPrimitive.Item asChild>
                <Link
                  href="/admin"
                  aria-current={isAdminRoute ? 'page' : undefined}
                  className={cn(MENU_ITEM_CLASS, isAdminRoute && 'bg-accent')}
                >
                  <Shield />
                  <span>管理面板</span>
                </Link>
              </DropdownMenuPrimitive.Item>
            )}

            <DropdownMenuPrimitive.Separator className="-mx-1 my-1 h-px bg-border" />

            <DropdownMenuPrimitive.Item
              className={cn(
                MENU_ITEM_CLASS,
                'text-destructive focus:bg-destructive/10 focus:text-destructive',
              )}
              onSelect={handleLogout}
            >
              <LogOut className="text-destructive" />
              <span>退出登录</span>
            </DropdownMenuPrimitive.Item>
          </DropdownMenuPrimitive.Content>
        </DropdownMenuPrimitive.Portal>
      </DropdownMenuPrimitive.Root>
    </div>
  );
}
