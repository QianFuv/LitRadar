'use client';

import { useState } from 'react';
import Link from 'next/link';
import {
  CalendarDays,
  Ellipsis,
  LogIn,
  LogOut,
  Radar,
  Settings,
  Shield,
  Star,
  User,
  X,
} from 'lucide-react';

import { Button } from '@/components/ui/button';
import { useAuth } from '@/lib/auth-context';
import { cn } from '@/lib/utils';

interface FabItem {
  icon: React.ReactNode;
  label: string;
  href?: string;
  onClick?: () => void;
}

export function WeeklyUpdatesFab() {
  const { user, logout } = useAuth();
  const [open, setOpen] = useState(false);

  const items: FabItem[] = [];

  if (user) {
    items.push(
      { icon: <Star className="h-4 w-4" />, label: '我的收藏', href: '/favorites' },
      { icon: <Radar className="h-4 w-4" />, label: '文献追踪', href: '/tracking' },
      { icon: <Settings className="h-4 w-4" />, label: '账号设置', href: '/settings' },
    );
    if (user.is_admin) {
      items.push({ icon: <Shield className="h-4 w-4" />, label: '管理面板', href: '/admin' });
    }
    items.push(
      {
        icon: <CalendarDays className="h-4 w-4" />,
        label: '每周更新',
        href: '/weekly-updates',
      },
      {
        icon: <LogOut className="h-4 w-4" />,
        label: '退出登录',
        onClick: () => {
          void logout();
          setOpen(false);
        },
      },
    );
  } else {
    items.push({ icon: <LogIn className="h-4 w-4" />, label: '登录', href: '/login' });
  }

  return (
    <div className="fixed bottom-6 right-6 z-40 flex flex-col-reverse items-end gap-2">
      <Button
        size="icon"
        className="h-12 w-12 rounded-full shadow-lg"
        aria-label={open ? '关闭菜单' : '打开菜单'}
        onClick={() => setOpen((v) => !v)}
      >
        {open ? (
          <X className="h-5 w-5" />
        ) : user ? (
          <User className="h-5 w-5" />
        ) : (
          <Ellipsis className="h-5 w-5" />
        )}
      </Button>

      {open && (
        <div className="flex flex-col items-end gap-2 animate-in fade-in slide-in-from-bottom-4 duration-200">
          {items.map((item) => {
            const inner = (
              <div className="flex items-center gap-2">
                <span className="text-xs font-medium whitespace-nowrap bg-background border rounded-md px-2 py-1 shadow-sm">
                  {item.label}
                </span>
                <div className="h-10 w-10 rounded-full bg-secondary flex items-center justify-center shadow-md">
                  {item.icon}
                </div>
              </div>
            );

            if (item.href) {
              return (
                <Link
                  key={item.label}
                  href={item.href}
                  onClick={() => setOpen(false)}
                  className={cn('flex items-center gap-2')}
                >
                  {inner}
                </Link>
              );
            }

            return (
              <button
                key={item.label}
                type="button"
                onClick={item.onClick}
                className={cn('flex items-center gap-2')}
              >
                {inner}
              </button>
            );
          })}
        </div>
      )}
    </div>
  );
}
