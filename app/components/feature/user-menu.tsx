'use client';

/**
 * Accessible account controls, settings entry, theme preferences, and logout.
 */

import * as DropdownMenuPrimitive from '@radix-ui/react-dropdown-menu';
import {
  AlertTriangle,
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
import { buildAdminCenterHref, parseAdminSection } from '@/lib/admin-center';
import { useAuth, type LogoutRevocationWarning } from '@/lib/auth-context';
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
  "motion-control focus:bg-accent focus:text-accent-foreground [&_svg:not([class*='text-'])]:text-muted-foreground relative flex w-full cursor-default items-center gap-2 rounded-sm px-2 py-1.5 text-sm transition-[background-color,color] outline-hidden select-none data-[disabled]:pointer-events-none data-[disabled]:opacity-50 [&_svg]:pointer-events-none [&_svg]:shrink-0 [&_svg:not([class*='size-'])]:size-4";

const MENU_CONTENT_CLASS =
  'motion-popover bg-popover text-popover-foreground z-50 origin-(--radix-dropdown-menu-content-transform-origin) rounded-md border p-1 shadow-md outline-hidden';

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
 * Render durable warning metadata when server-side logout was not confirmed.
 *
 * @param props - Persisted warning metadata.
 * @returns Recovery notice linking to fresh reauthentication.
 */
function LogoutRevocationNotice({ warning }: { warning: LogoutRevocationWarning }) {
  return (
    <div
      role="alert"
      className="w-[min(24rem,calc(100vw-2rem))] rounded-lg border border-destructive/30 bg-popover p-3 text-sm text-popover-foreground shadow-lg"
    >
      <div className="flex items-start gap-2">
        <AlertTriangle className="mt-0.5 size-4 shrink-0 text-destructive" aria-hidden="true" />
        <div className="min-w-0 space-y-2">
          <p className="font-medium">服务端会话撤销未确认</p>
          <p className="text-xs text-muted-foreground">
            本地会话信息已清除，但旧令牌可能仍有效。请重新认证后撤销全部会话。
          </p>
          {warning.requestId && (
            <p className="break-all text-xs text-muted-foreground">请求 ID：{warning.requestId}</p>
          )}
          <Link
            href="/login?logout_recovery=1"
            className="motion-control inline-flex h-8 items-center rounded-md border border-input bg-background px-3 text-xs font-medium transition-[background-color,color] hover:bg-accent hover:text-accent-foreground"
          >
            重新认证并撤销全部会话
          </Link>
        </div>
      </div>
    </div>
  );
}

/**
 * Render the authenticated account trigger and account-only menu.
 *
 * @returns Account menu or null while authentication is unresolved.
 */
export function UserMenu() {
  const { user, loading, logout, logoutWarning } = useAuth();
  const pathname = usePathname();
  const searchParams = useSearchParams();
  const { setTheme, theme } = useTheme();
  const accountTriggerRef = useRef<HTMLButtonElement>(null);
  const isMounted = useSyncExternalStore(
    subscribeToClientEnvironment,
    getClientEnvironmentSnapshot,
    getServerEnvironmentSnapshot,
  );

  if (loading) {
    return null;
  }

  if (!user) {
    return logoutWarning ? (
      <div data-slot="user-menu-position" className="fixed z-40" style={USER_MENU_POSITION_STYLE}>
        <LogoutRevocationNotice warning={logoutWarning} />
      </div>
    ) : null;
  }

  const selectedTheme = isMounted ? (theme ?? 'system') : 'system';
  const settingsHref = buildSettingsCenterHref(pathname, searchParams, 'general');
  const adminHref = buildAdminCenterHref(pathname, searchParams, 'overview');
  const isAdminOpen = parseAdminSection(searchParams.get('admin')) !== null;

  /**
   * Clear the authenticated session after the menu selection closes.
   */
  function handleLogout(): void {
    void logout().catch(() => undefined);
  }

  /**
   * Mark the persistent account trigger before current-tab sectioned-dialog navigation.
   *
   * @param event - Sectioned-dialog link click event.
   */
  function handleSectionedDialogOpen(event: MouseEvent<HTMLAnchorElement>): void {
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
    <div
      data-slot="user-menu-position"
      className="fixed z-40 flex flex-col items-end gap-2"
      style={USER_MENU_POSITION_STYLE}
    >
      {logoutWarning && <LogoutRevocationNotice warning={logoutWarning} />}
      <DropdownMenuPrimitive.Root>
        <DropdownMenuPrimitive.Trigger asChild>
          <Button
            ref={accountTriggerRef}
            type="button"
            variant="outline"
            className="group/account-menu h-12 max-w-[min(15rem,calc(100vw-2rem))] gap-2 rounded-full bg-popover px-2.5 text-popover-foreground shadow-lg hover:bg-accent hover:text-accent-foreground"
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
            <ChevronUp
              className="motion-chevron size-4 shrink-0 text-muted-foreground transition-transform group-data-[state=open]/account-menu:rotate-180"
              aria-hidden="true"
            />
          </Button>
        </DropdownMenuPrimitive.Trigger>

        <DropdownMenuPrimitive.Portal>
          <DropdownMenuPrimitive.Content
            aria-label="账号菜单"
            align="end"
            side="top"
            sideOffset={8}
            className={cn(MENU_CONTENT_CLASS, 'w-64 max-w-[calc(100vw-2rem)]')}
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
              <Link
                href={settingsHref}
                className={MENU_ITEM_CLASS}
                onClick={handleSectionedDialogOpen}
              >
                <Settings2 />
                <span>打开设置中心</span>
              </Link>
            </DropdownMenuPrimitive.Item>

            <div className="flex items-center justify-between gap-2 px-2 py-1.5">
              <span className="flex shrink-0 items-center gap-2 text-sm">
                <Monitor className="size-4 text-muted-foreground" aria-hidden="true" />
                <span>外观主题</span>
              </span>
              {isMounted && (
                <DropdownMenuPrimitive.RadioGroup
                  aria-label="外观主题"
                  value={selectedTheme}
                  onValueChange={handleThemeChange}
                  className="flex shrink-0 items-center gap-1"
                >
                  {THEME_ITEMS.map((item) => {
                    const Icon = item.icon;

                    return (
                      <DropdownMenuPrimitive.RadioItem
                        key={item.value}
                        value={item.value}
                        textValue={item.label}
                        aria-label={item.label}
                        title={item.label}
                        className="motion-control flex size-11 shrink-0 cursor-default items-center justify-center rounded-md text-muted-foreground outline-none transition-[background-color,color,box-shadow] focus-visible:ring-[3px] focus-visible:ring-ring/50 data-[highlighted]:bg-accent data-[highlighted]:text-foreground data-[state=checked]:bg-accent data-[state=checked]:text-foreground md:size-9"
                      >
                        <Icon className="size-4" aria-hidden="true" />
                      </DropdownMenuPrimitive.RadioItem>
                    );
                  })}
                </DropdownMenuPrimitive.RadioGroup>
              )}
            </div>

            {user.is_admin && (
              <DropdownMenuPrimitive.Item asChild>
                <Link
                  href={adminHref}
                  aria-current={isAdminOpen ? 'page' : undefined}
                  className={cn(MENU_ITEM_CLASS, isAdminOpen && 'bg-accent')}
                  onClick={handleSectionedDialogOpen}
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
