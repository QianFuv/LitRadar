/**
 * Unread system announcements shown as stacked toasts in the bottom-right corner.
 */

import { Toast } from '@animate-ui/react/primitives/base-ui/toast';
import { useQuery } from '@tanstack/react-query';
import { useEffect, useRef, useState } from 'react';
import { z } from 'zod';

import { X } from '@/components/ui/animated-icons';
import { Button } from '@/components/ui/button';
import { getAnnouncements, type AnnouncementInfo } from '@/lib/api';
import {
  readLocalStorageValue,
  removeLocalStorageValue,
  writeLocalStorageValue,
} from '@/lib/browser-storage';
import * as stylex from '@stylexjs/stylex';
import { colors, lineHeights } from '@/lib/theme.stylex';

const STORAGE_PREFIX = 'litradar:v1:announcement_dismissed:';
/** Stored dismissal expiry: a finite millisecond timestamp, or `0` for a permanent dismissal. */
const DISMISSED_UNTIL_SCHEMA = z.coerce.number();
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

  const parsedDismissal = DISMISSED_UNTIL_SCHEMA.safeParse(rawValue);
  if (!parsedDismissal.success) {
    removeLocalStorageValue(storageKey);
    return null;
  }
  const dismissedUntil = parsedDismissal.data;

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
      {...stylex.props(styles.announcementToastToastRoot, styles.stackOrder(TOAST_Z_INDEX))}
    >
      <Toast.Content {...stylex.props(styles.announcementToastToastContent)}>
        <div {...stylex.props(styles.announcementToastDiv)}>
          <Toast.Title {...stylex.props(styles.announcementToastToastTitle)} />
          <span {...stylex.props(styles.announcementToastSpan)}>
            {PRIORITY_LABELS[announcement.priority]}
          </span>
        </div>
        <Toast.Description {...stylex.props(styles.announcementToastToastDescription)} />
        <div {...stylex.props(styles.announcementToastDiv2)}>
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
        <Toast.Close aria-label="关闭" {...stylex.props(styles.announcementToastToastClose)}>
          <X {...stylex.props(styles.announcementToastX)} aria-hidden="true" />
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
          {...stylex.props(styles.announcementToastsToastViewport)}
        >
          <AnnouncementToastList />
        </Toast.Viewport>
      </Toast.Portal>
    </Toast.Provider>
  );
}

/** Newer toasts stack above older ones; Base UI sets `--toast-index` on each toast. */
const TOAST_Z_INDEX = 'calc(1000 - var(--toast-index))';

const styles = stylex.create({
  stackOrder: (zIndex: string) => ({
    zIndex,
  }),
  announcementToastToastRoot: {
    position: 'absolute',
    right: '0px',
    bottom: '0px',
    height: {
      default: 'var(--toast-frontmost-height, var(--toast-height))',
      ':is([data-expanded])': 'var(--toast-height)',
    },
    width: '100%',
    borderRadius: '12px',
    borderStyle: 'solid',
    borderWidth: '1px',
    backgroundColor: colors.popover,
    color: colors.popoverForeground,
    boxShadow: '0 10px 15px -3px rgb(0 0 0 / 0.1), 0 4px 6px -4px rgb(0 0 0 / 0.1)',
    userSelect: 'none',
    '::after': {
      content: "''",
      position: 'absolute',
      top: '100%',
      left: '0px',
      height: 'calc(0.75rem + 1px)',
      width: '100%',
    },
  },
  announcementToastToastContent: {
    overflow: 'hidden',
    padding: '1rem',
    opacity: {
      default: null,
      ':is([data-behind])': '0%',
      ':is([data-expanded])': '100%',
    },
  },
  announcementToastDiv: {
    display: 'flex',
    alignItems: 'flex-start',
    gap: '0.5rem',
    paddingRight: '1.75rem',
  },
  announcementToastToastTitle: {
    minWidth: '0px',
    fontSize: '0.875rem',
    lineHeight: '1.5rem',
    fontWeight: 700,
    overflowWrap: 'anywhere',
  },
  announcementToastSpan: {
    marginTop: '0.125rem',
    flexShrink: 0,
    borderRadius: 'calc(infinity * 1px)',
    backgroundColor: colors.muted,
    paddingInline: '0.5rem',
    paddingBlock: '0.125rem',
    fontSize: '0.75rem',
    lineHeight: lineHeights.textXs,
    color: colors.mutedForeground,
  },
  announcementToastToastDescription: {
    marginTop: '0.375rem',
    maxHeight: '10rem',
    overflowY: 'auto',
    overscrollBehavior: 'contain',
    fontSize: '13px',
    lineHeight: '1.5rem',
    whiteSpace: 'pre-wrap',
    color: `color-mix(in oklab, ${colors.foreground} 80%, transparent)`,
  },
  announcementToastDiv2: {
    marginTop: '0.75rem',
    display: 'flex',
    flexWrap: 'wrap',
    gap: '0.375rem',
  },
  announcementToastToastClose: {
    position: 'absolute',
    top: '0.75rem',
    right: '0.75rem',
    display: 'flex',
    width: '1.75rem',
    height: '1.75rem',
    alignItems: 'center',
    justifyContent: 'center',
    borderRadius: '6px',
    color: {
      default: colors.mutedForeground,
      ':hover': {
        default: null,
        '@media (hover: hover)': colors.foreground,
      },
    },
    transitionProperty:
      'color, background-color, border-color, outline-color, text-decoration-color, fill, stroke',
    transitionTimingFunction: 'cubic-bezier(0.25, 0.1, 0.25, 1)',
    transitionDuration: '200ms',
    outlineStyle: 'none',
    backgroundColor: {
      default: null,
      ':hover': {
        default: null,
        '@media (hover: hover)': colors.accent,
      },
    },
    boxShadow: {
      default: null,
      ':focus-visible': `0 0 0 2px ${colors.ring}`,
    },
  },
  announcementToastX: {
    width: '1rem',
    height: '1rem',
  },
  announcementToastsToastViewport: {
    position: 'fixed',
    right: '1rem',
    bottom: {
      default: 'calc(4.25rem + var(--safe-area-inset-bottom, env(safe-area-inset-bottom,0px)))',
      '@media (min-width: 48rem)': '1rem',
    },
    zIndex: 50,
    width: 'min(24rem, calc(100vw - 2rem))',
  },
});
