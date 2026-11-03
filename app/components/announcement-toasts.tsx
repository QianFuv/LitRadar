'use client';

/**
 * Unread system announcements shown as stacked toasts in the bottom-right corner.
 */

import { Toast } from '@animate-ui/react/primitives/base-ui/toast';
import { useQuery } from '@tanstack/react-query';
import { useEffect, useRef, useState } from 'react';

import { X } from '@/components/ui/animated-icons';
import { Button } from '@/components/ui/button';
import { getAnnouncements, type AnnouncementInfo } from '@/lib/api';
import {
  readLocalStorageValue,
  removeLocalStorageValue,
  writeLocalStorageValue,
} from '@/lib/browser-storage';

const STORAGE_PREFIX = 'litradar:v1:announcement_dismissed:';
const PRIORITY_LABELS = {
  high: '高优先级',
  low: '低优先级',
  normal: '普通',
} as const;

/** Order toasts are added in: later toasts stack in front, so high priority lands on top. */
const PRIORITY_STACK_ORDER = { low: 0, normal: 1, high: 2 } as const;

/** How long normal and low priority announcements stay before closing, in milliseconds. */
const ANNOUNCEMENT_TIMEOUT_MS = 8000;

/** Length of the seven-day dismissal window, in milliseconds. */
const SEVEN_DAYS_MS = 7 * 24 * 3600 * 1000;

type AnnouncementToastData = { announcement: AnnouncementInfo };

/**
 * Build the storage key that records one announcement's dismissal.
 *
 * @param id - Announcement identifier.
 * @returns Local storage key.
 */
function getStorageKey(id: number): string {
  return `${STORAGE_PREFIX}${id}`;
}

/**
 * Read when an announcement's dismissal expires, clearing malformed or expired records.
 *
 * @param id - Announcement identifier.
 * @returns Expiry timestamp, `0` for a permanent dismissal, or null when not dismissed.
 */
function readDismissedUntil(id: number): number | null {
  const storageKey = getStorageKey(id);
  const rawValue = readLocalStorageValue(storageKey);
  if (rawValue === null) {
    return null;
  }

  const dismissedUntil = Number(rawValue);
  if (!Number.isFinite(dismissedUntil)) {
    removeLocalStorageValue(storageKey);
    return null;
  }

  if (dismissedUntil == 0) {
    return 0;
  }

  if (dismissedUntil > Date.now()) {
    return dismissedUntil;
  }

  removeLocalStorageValue(storageKey);
  return null;
}

/**
 * Report whether the user has dismissed an announcement for now.
 *
 * @param announcement - Announcement to check.
 * @returns Whether the announcement should stay hidden.
 */
function isDismissed(announcement: AnnouncementInfo): boolean {
  const dismissedUntil = readDismissedUntil(announcement.id);
  if (dismissedUntil === null) {
    return false;
  }
  return dismissedUntil === 0 || dismissedUntil > Date.now();
}

/**
 * Read the last millisecond of the current local day.
 *
 * @returns End-of-day timestamp.
 */
function getEndOfTodayTimestamp(): number {
  const endOfDay = new Date();
  endOfDay.setHours(23, 59, 59, 999);
  return endOfDay.getTime();
}

/**
 * Render one announcement toast with its dismissal choices.
 *
 * @param props - Toast object carrying the announcement.
 * @returns Announcement toast.
 */
function AnnouncementToast({ toast }: { toast: Toast.Root.ToastObject<AnnouncementToastData> }) {
  const { close } = Toast.useToastManager();
  const announcement = toast.data?.announcement;
  if (!announcement) {
    return null;
  }

  /**
   * Hide this announcement until the given time and close its toast.
   *
   * @param dismissedUntil - Expiry timestamp, or `0` to hide it permanently.
   */
  const dismissUntil = (dismissedUntil: number) => {
    writeLocalStorageValue(getStorageKey(announcement.id), String(dismissedUntil));
    close(toast.id);
  };

  return (
    <Toast.Root
      toast={toast}
      className="absolute right-0 bottom-0 h-[var(--toast-frontmost-height,var(--toast-height))] w-full rounded-xl border bg-popover text-popover-foreground shadow-lg select-none [z-index:calc(1000-var(--toast-index))] after:absolute after:top-full after:left-0 after:h-[calc(0.75rem+1px)] after:w-full after:content-[''] data-[expanded]:h-[var(--toast-height)]"
    >
      <Toast.Content className="overflow-hidden p-4 data-[behind]:opacity-0 data-[expanded]:opacity-100">
        <div className="flex items-start gap-2 pr-7">
          <Toast.Title className="min-w-0 text-sm leading-6 font-bold [overflow-wrap:anywhere]" />
          <span className="mt-0.5 shrink-0 rounded-full bg-muted px-2 py-0.5 text-xs text-muted-foreground">
            {PRIORITY_LABELS[announcement.priority]}
          </span>
        </div>
        <Toast.Description className="mt-1.5 max-h-40 overflow-y-auto overscroll-contain text-[13px] leading-6 whitespace-pre-wrap text-foreground/80" />
        <div className="mt-3 flex flex-wrap gap-1.5">
          <Button
            size="xs"
            variant="outline"
            onClick={() => dismissUntil(getEndOfTodayTimestamp())}
          >
            今日不再提示
          </Button>
          <Button
            size="xs"
            variant="outline"
            onClick={() => dismissUntil(Date.now() + SEVEN_DAYS_MS)}
          >
            7天内不再提示
          </Button>
          <Button size="xs" onClick={() => dismissUntil(0)}>
            永久关闭
          </Button>
        </div>
        <Toast.Close
          aria-label="关闭"
          className="motion-control absolute top-3 right-3 flex size-7 items-center justify-center rounded-md text-muted-foreground outline-none transition-colors hover:bg-accent hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring"
        >
          <X className="size-4" aria-hidden="true" />
        </Toast.Close>
      </Toast.Content>
    </Toast.Root>
  );
}

/**
 * Render every queued announcement toast.
 *
 * @returns Announcement toast stack.
 */
function AnnouncementToastList() {
  const { toasts } = Toast.useToastManager<AnnouncementToastData>();
  return toasts.map((toast) => <AnnouncementToast key={toast.id} toast={toast} />);
}

/**
 * Show each unread announcement once per page as a toast. High priority announcements stay until
 * the user closes them and are announced urgently; others close after a few seconds unless
 * hovered. Closing a toast hides it for this page only; the dismissal buttons persist the choice.
 *
 * @returns Announcement toast region.
 */
export function AnnouncementToasts() {
  const [toastManager] = useState(() => Toast.createToastManager());
  const shownIdsRef = useRef(new Set<number>());
  const { data } = useQuery({
    queryKey: ['announcements'],
    queryFn: getAnnouncements,
    refetchInterval: 60_000,
  });

  useEffect(() => {
    const byStackOrder = [...(data ?? [])].sort(
      (first, second) =>
        PRIORITY_STACK_ORDER[first.priority] - PRIORITY_STACK_ORDER[second.priority],
    );
    for (const announcement of byStackOrder) {
      if (shownIdsRef.current.has(announcement.id) || isDismissed(announcement)) {
        continue;
      }
      shownIdsRef.current.add(announcement.id);
      const isHighPriority = announcement.priority === 'high';
      toastManager.add<AnnouncementToastData>({
        id: `announcement-${announcement.id}`,
        title: announcement.title,
        description: announcement.message,
        priority: isHighPriority ? 'high' : 'low',
        timeout: isHighPriority ? 0 : ANNOUNCEMENT_TIMEOUT_MS,
        data: { announcement },
      });
    }
  }, [data, toastManager]);

  return (
    <Toast.Provider toastManager={toastManager}>
      <Toast.Portal>
        <Toast.Viewport
          aria-label="系统公告"
          className="fixed right-4 bottom-[calc(4.25rem+var(--safe-area-inset-bottom,env(safe-area-inset-bottom,0px)))] z-50 w-[min(24rem,calc(100vw-2rem))] md:bottom-4"
        >
          <AnnouncementToastList />
        </Toast.Viewport>
      </Toast.Portal>
    </Toast.Provider>
  );
}
