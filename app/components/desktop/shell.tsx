'use client';

/**
 * Desktop application shell for protected routes.
 */

import Link from 'next/link';
import { usePathname, useRouter } from 'next/navigation';
import { useQuery } from '@tanstack/react-query';
import {
  Bell,
  CalendarDays,
  ChevronDown,
  LogOut,
  Moon,
  PanelLeftClose,
  PanelLeftOpen,
  Radar,
  Search,
  Settings,
  Shield,
  Star,
  Sun,
  type LucideIcon,
} from 'lucide-react';
import {
  createContext,
  useContext,
  useState,
  useCallback,
  useEffect,
  useMemo,
  useRef,
  type ReactNode,
} from 'react';
import { useAuthSession } from '@/lib/auth-session';
import { useClickOutside } from '@/lib/hooks';
import { getAnnouncements, type AnnouncementInfo } from '@/lib/client-api';
import { formatTimestamp } from '@/lib/format';
import { Badge, IconButton, joinClassNames } from '@/components/desktop/ui';

/**
 * Configuration options for the desktop application shell.
 */
export interface ShellConfig {
  /** The title displayed in the header. */
  title: string;
  /** Optional kicker (small sub-header text) displayed above the title. */
  kicker?: string;
  /** Optional React actions rendered in the top bar. */
  actions?: ReactNode;
  /** Optional React element to be displayed in the sidebar. */
  sidebarExtra?: ReactNode;
}

/**
 * Interface representing the context state and modifiers of the Desktop Shell.
 */
interface ShellContextType {
  /** The current configuration settings for the shell. */
  config: ShellConfig;
  /** Update the active configuration of the shell. */
  setConfig: (config: ShellConfig) => void;
  /** Indicates whether the left sidebar is currently collapsed. */
  isSidebarCollapsed: boolean;
  /** Set the sidebar collapse state. */
  setIsSidebarCollapsed: (collapsed: boolean) => void;
}

const ShellContext = createContext<ShellContextType | undefined>(undefined);

const SIDEBAR_COLLAPSED_KEY = 'paper_scanner_sidebar_collapsed';

/**
 * Retrieve the persisted sidebar collapsed state from local storage.
 *
 * @returns Saved state or false if not stored/server-side.
 */
function readStoredSidebarCollapsed(): boolean {
  if (typeof window === 'undefined') {
    return false;
  }
  return window.localStorage.getItem(SIDEBAR_COLLAPSED_KEY) === 'true';
}

/**
 * React context provider to store shell state and configurations dynamically.
 *
 * @param props - Element containing children.
 * @returns Element with active shell provider.
 */
export function ShellProvider({ children }: { children: ReactNode }) {
  const [config, setConfigState] = useState<ShellConfig>({ title: '' });
  const [isSidebarCollapsed, setIsSidebarCollapsedState] = useState<boolean>(() =>
    readStoredSidebarCollapsed(),
  );

  const setConfig = useCallback((newConfig: ShellConfig) => {
    setConfigState((prev) => {
      if (
        prev.title === newConfig.title &&
        prev.kicker === newConfig.kicker &&
        prev.actions === newConfig.actions &&
        prev.sidebarExtra === newConfig.sidebarExtra
      ) {
        return prev;
      }
      return newConfig;
    });
  }, []);

  const setIsSidebarCollapsed = useCallback((collapsed: boolean) => {
    setIsSidebarCollapsedState(collapsed);
    if (typeof window !== 'undefined') {
      window.localStorage.setItem(SIDEBAR_COLLAPSED_KEY, String(collapsed));
    }
  }, []);

  return (
    <ShellContext.Provider
      value={{
        config,
        setConfig,
        isSidebarCollapsed,
        setIsSidebarCollapsed,
      }}
    >
      {children}
    </ShellContext.Provider>
  );
}

/**
 * Hook to consume the ShellContext.
 *
 * @returns Active shell context state.
 */
export function useShell(): ShellContextType {
  const context = useContext(ShellContext);
  if (!context) {
    throw new Error('useShell must be used within a ShellProvider');
  }
  return context;
}

/**
 * Component to apply layout configuration dynamically from a page component.
 *
 * @param props - Layout configuration attributes.
 * @returns Null component.
 */
export function ShellConfigurator({ title, kicker, actions, sidebarExtra }: ShellConfig) {
  const { setConfig } = useShell();
  useEffect(() => {
    setConfig({ title, kicker, actions, sidebarExtra });
  }, [title, kicker, actions, sidebarExtra, setConfig]);
  return null;
}

interface NavigationItem {
  href: string;
  label: string;
  icon: LucideIcon;
  adminOnly?: boolean;
}

type ShellTheme = 'light' | 'dark';
type ActiveTopbarMenu = 'notifications' | 'account' | null;

const THEME_STORAGE_KEY = 'paper_scanner_theme';

const NAVIGATION_ITEMS: NavigationItem[] = [
  { href: '/', icon: Search, label: '搜索' },
  { href: '/weekly-updates', icon: CalendarDays, label: '每周更新' },
  { href: '/favorites', icon: Star, label: '收藏' },
  { href: '/tracking', icon: Radar, label: '追踪' },
  { href: '/settings', icon: Settings, label: '设置' },
  { href: '/admin', icon: Shield, label: '管理', adminOnly: true },
];

/**
 * Check whether a navigation item is active for the current path.
 *
 * @param pathname - Current pathname.
 * @param href - Item href.
 * @returns Whether the item is active.
 */
function isActivePath(pathname: string, href: string): boolean {
  if (href === '/') {
    return pathname === '/';
  }
  return pathname === href || pathname.startsWith(`${href}/`);
}

/**
 * Build a compact avatar label.
 *
 * @param username - Username.
 * @returns Avatar text.
 */
function getAvatarLabel(username: string): string {
  return username.trim().slice(0, 2).toUpperCase() || 'PS';
}

/**
 * Read the saved shell theme.
 *
 * @returns Stored theme or light mode.
 */
function readStoredTheme(): ShellTheme {
  if (typeof window === 'undefined') {
    return 'light';
  }
  return window.localStorage.getItem(THEME_STORAGE_KEY) === 'dark' ? 'dark' : 'light';
}

/**
 * Apply and persist the shell theme.
 *
 * @param theme - Theme to apply.
 */
function applyTheme(theme: ShellTheme): void {
  document.documentElement.dataset.theme = theme;
  window.localStorage.setItem(THEME_STORAGE_KEY, theme);
}

/**
 * Get the visual tone for an announcement priority.
 *
 * @param priority - Announcement priority.
 * @returns Badge tone.
 */
function getAnnouncementTone(
  priority: AnnouncementInfo['priority'],
): 'coral' | 'neutral' | 'violet' {
  if (priority === 'high') {
    return 'coral';
  }
  if (priority === 'normal') {
    return 'violet';
  }
  return 'neutral';
}

/**
 * Get a short label for an announcement priority.
 *
 * @param priority - Announcement priority.
 * @returns User-facing priority label.
 */
function getAnnouncementPriorityLabel(priority: AnnouncementInfo['priority']): string {
  if (priority === 'high') {
    return '重要';
  }
  if (priority === 'normal') {
    return '更新';
  }
  return '提示';
}

/**
 * Render the protected desktop shell.
 *
 * @param props - Component props containing children.
 * @returns Desktop shell layout.
 */
export function DesktopShell({ children }: { children: ReactNode }) {
  const pathname = usePathname();
  const router = useRouter();
  const { logout, user } = useAuthSession();
  const controlsRef = useRef<HTMLDivElement>(null);
  const [theme, setTheme] = useState<ShellTheme>(() => readStoredTheme());
  const { config, isSidebarCollapsed, setIsSidebarCollapsed } = useShell();
  const { title, kicker, actions, sidebarExtra } = config;
  const [activeMenu, setActiveMenu] = useState<ActiveTopbarMenu>(null);
  const visibleItems = NAVIGATION_ITEMS.filter((item) => !item.adminOnly || user?.is_admin);
  const { data: announcements = [], isError: announcementsFailed } = useQuery({
    queryKey: ['announcements'],
    queryFn: getAnnouncements,
    refetchInterval: 60_000,
  });
  const unreadCount = announcements.length;
  const accountRole = user?.is_admin ? '管理员' : '研究员';
  const accountName = user?.username || '未命名用户';
  const themeLabel = theme === 'dark' ? '切换浅色模式' : '切换暗色模式';

  useEffect(() => {
    applyTheme(theme);
  }, [theme]);

  useClickOutside(controlsRef, () => setActiveMenu(null), Boolean(activeMenu));

  const topAnnouncements = useMemo(() => announcements.slice(0, 5), [announcements]);

  return (
    <div
      className={joinClassNames('desktop-shell', isSidebarCollapsed && 'desktop-shell--collapsed')}
    >
      <aside className="desktop-shell__sidebar">
        <div className="desktop-shell__brand">
          <div className="desktop-shell__brand-mark">P</div>
          <div className="desktop-shell__brand-copy">
            <p className="desktop-shell__brand-title">Paper Scanner</p>
          </div>
        </div>
        <nav className="desktop-shell__nav" aria-label="主导航">
          {visibleItems.map((item) => {
            const Icon = item.icon;
            const active = isActivePath(pathname, item.href);
            return (
              <Link
                key={item.href}
                className={joinClassNames(
                  'desktop-shell__nav-link',
                  active && 'desktop-shell__nav-link--active',
                )}
                href={item.href}
                title={isSidebarCollapsed ? item.label : undefined}
              >
                <Icon size={17} />
                <span className="desktop-shell__nav-label">{item.label}</span>
              </Link>
            );
          })}
        </nav>
        <div className="desktop-shell__sidebar-extra">{sidebarExtra}</div>
        <button
          aria-label={isSidebarCollapsed ? '展开侧栏' : '收起侧栏'}
          className="desktop-shell__collapse"
          title={isSidebarCollapsed ? '展开侧栏' : '收起侧栏'}
          type="button"
          onClick={() => setIsSidebarCollapsed(!isSidebarCollapsed)}
        >
          {isSidebarCollapsed ? <PanelLeftOpen size={16} /> : <PanelLeftClose size={16} />}
          <span className="desktop-shell__collapse-label">
            {isSidebarCollapsed ? '展开' : '收起'}
          </span>
        </button>
      </aside>
      <main className="desktop-shell__main">
        <header className="desktop-shell__topbar">
          <div>
            {kicker ? <p className="desktop-shell__kicker">{kicker}</p> : null}
            <h1 className="desktop-shell__title">{title}</h1>
          </div>
          <div ref={controlsRef} className="desktop-shell__actions">
            {actions}
            <IconButton
              aria-label={themeLabel}
              className="desktop-shell__theme-toggle"
              title={themeLabel}
              onClick={() =>
                setTheme((currentTheme) => (currentTheme === 'dark' ? 'light' : 'dark'))
              }
            >
              {theme === 'dark' ? <Sun size={16} /> : <Moon size={16} />}
            </IconButton>
            <div className="desktop-shell__popover-anchor">
              <button
                aria-expanded={activeMenu === 'notifications'}
                aria-haspopup="dialog"
                className={joinClassNames(
                  'desktop-shell__bell',
                  activeMenu === 'notifications' && 'desktop-shell__bell--active',
                )}
                title="通知"
                type="button"
                onClick={() =>
                  setActiveMenu((currentMenu) =>
                    currentMenu === 'notifications' ? null : 'notifications',
                  )
                }
              >
                <Bell size={18} />
                {unreadCount > 0 ? (
                  <span className="desktop-shell__bell-badge">{Math.min(unreadCount, 99)}</span>
                ) : null}
              </button>
              {activeMenu === 'notifications' ? (
                <section className="desktop-shell__popover desktop-shell__popover--notifications">
                  <div className="desktop-shell__popover-header">
                    <strong>通知</strong>
                    <span>{unreadCount > 0 ? `${unreadCount} 条` : '全部已读'}</span>
                  </div>
                  <div className="desktop-shell__menu-list">
                    {announcementsFailed ? (
                      <div className="desktop-shell__empty-note">通知加载失败，请稍后重试。</div>
                    ) : topAnnouncements.length > 0 ? (
                      topAnnouncements.map((announcement) => (
                        <article key={announcement.id} className="desktop-shell__notification">
                          <div className="toolbar toolbar--wrap">
                            <Badge tone={getAnnouncementTone(announcement.priority)}>
                              {getAnnouncementPriorityLabel(announcement.priority)}
                            </Badge>
                            <time>{formatTimestamp(announcement.updated_at)}</time>
                          </div>
                          <strong>{announcement.title}</strong>
                          <p>{announcement.message}</p>
                        </article>
                      ))
                    ) : (
                      <div className="desktop-shell__empty-note">暂无新的系统通知。</div>
                    )}
                  </div>
                </section>
              ) : null}
            </div>
            <div className="desktop-shell__popover-anchor">
              <button
                aria-expanded={activeMenu === 'account'}
                aria-haspopup="menu"
                className="desktop-shell__account"
                type="button"
                title="用户菜单"
                onClick={() =>
                  setActiveMenu((currentMenu) => (currentMenu === 'account' ? null : 'account'))
                }
              >
                <span className="desktop-shell__avatar">{getAvatarLabel(accountName)}</span>
                <span className="desktop-shell__username">{accountName}</span>
                <ChevronDown size={15} />
              </button>
              {activeMenu === 'account' ? (
                <section
                  className="desktop-shell__popover desktop-shell__popover--account"
                  role="menu"
                >
                  <div className="desktop-shell__account-card">
                    <span className="desktop-shell__avatar">{getAvatarLabel(accountName)}</span>
                    <span>
                      <strong>{accountName}</strong>
                      <small>{accountRole}</small>
                    </span>
                  </div>
                  <div className="desktop-shell__menu-list">
                    <Link
                      className="desktop-shell__menu-item"
                      href="/settings"
                      role="menuitem"
                      onClick={() => setActiveMenu(null)}
                    >
                      <Settings size={15} />
                      <span>
                        <strong>账户与通知设置</strong>
                        <small>管理检索偏好和推送配置</small>
                      </span>
                    </Link>
                    <button
                      className="desktop-shell__menu-item desktop-shell__menu-item--danger"
                      role="menuitem"
                      type="button"
                      onClick={() => {
                        void logout().then(() => router.replace('/login'));
                      }}
                    >
                      <LogOut size={15} />
                      <span>
                        <strong>退出登录</strong>
                        <small>结束当前 Paper Scanner 会话</small>
                      </span>
                    </button>
                  </div>
                </section>
              ) : null}
            </div>
          </div>
        </header>
        <div className="workspace">{children}</div>
      </main>
    </div>
  );
}
