'use client';

import { useState } from 'react';
import { useQuery } from '@tanstack/react-query';

import { getAnnouncements, type AnnouncementInfo } from '@/lib/api';
import { Button } from '@/components/ui/button';
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog';

const STORAGE_PREFIX = 'announcement_dismissed_';
const PRIORITY_LABELS = {
  high: '高优先级',
  low: '低优先级',
  normal: '普通',
} as const;

function getStorageKey(id: number): string {
  return `${STORAGE_PREFIX}${id}`;
}

function readDismissedUntil(id: number): number | null {
  if (typeof window === 'undefined') {
    return null;
  }

  const rawValue = window.localStorage.getItem(getStorageKey(id));
  if (rawValue === null) {
    return null;
  }

  const dismissedUntil = Number(rawValue);
  if (!Number.isFinite(dismissedUntil)) {
    window.localStorage.removeItem(getStorageKey(id));
    return null;
  }

  if (dismissedUntil == 0) {
    return 0;
  }

  if (dismissedUntil > Date.now()) {
    return dismissedUntil;
  }

  window.localStorage.removeItem(getStorageKey(id));
  return null;
}

function isDismissed(announcement: AnnouncementInfo): boolean {
  const dismissedUntil = readDismissedUntil(announcement.id);
  if (dismissedUntil === null) {
    return false;
  }
  return dismissedUntil === 0 || dismissedUntil > Date.now();
}

function getEndOfTodayTimestamp(): number {
  const endOfDay = new Date();
  endOfDay.setHours(23, 59, 59, 999);
  return endOfDay.getTime();
}

function dismissAnnouncements(announcements: AnnouncementInfo[], dismissedUntil: number): void {
  if (typeof window === 'undefined') {
    return;
  }

  for (const announcement of announcements) {
    window.localStorage.setItem(getStorageKey(announcement.id), String(dismissedUntil));
  }
}

export function AnnouncementsDialog() {
  const [closedSignature, setClosedSignature] = useState<string | null>(null);

  const { data = [] } = useQuery({
    queryKey: ['announcements'],
    queryFn: getAnnouncements,
    refetchInterval: 60_000,
  });

  const unreadAnnouncements = data.filter((announcement) => !isDismissed(announcement));
  const unreadAnnouncementSignature = unreadAnnouncements
    .map((announcement) => String(announcement.id))
    .join(',');
  const open = unreadAnnouncements.length > 0 && closedSignature !== unreadAnnouncementSignature;

  const handleDismiss = (dismissedUntil: number) => {
    dismissAnnouncements(unreadAnnouncements, dismissedUntil);
    setClosedSignature(unreadAnnouncementSignature);
  };

  if (unreadAnnouncements.length === 0) {
    return null;
  }

  return (
    <Dialog
      open={open}
      onOpenChange={(nextOpen: boolean) => {
        if (!nextOpen) {
          setClosedSignature(unreadAnnouncementSignature);
        }
      }}
    >
      <DialogContent className="max-w-2xl">
        <DialogHeader>
          <DialogTitle>系统公告</DialogTitle>
          <DialogDescription>
            以下公告尚未阅读，可选择本日、7 天内或永久关闭提示。
          </DialogDescription>
        </DialogHeader>
        <div className="max-h-[60vh] space-y-3 overflow-y-auto overscroll-contain pr-1">
          {unreadAnnouncements.map((announcement) => (
            <div key={announcement.id} className="rounded-lg border bg-muted/20 p-4">
              <div className="mb-2 flex items-center gap-2">
                <h3 className="font-medium">{announcement.title}</h3>
                <span className="rounded-full border px-2 py-0.5 text-xs text-muted-foreground">
                  {PRIORITY_LABELS[announcement.priority]}
                </span>
              </div>
              <p className="whitespace-pre-wrap text-sm text-muted-foreground">
                {announcement.message}
              </p>
            </div>
          ))}
        </div>
        <DialogFooter className="gap-2 sm:justify-between">
          <Button variant="outline" onClick={() => handleDismiss(getEndOfTodayTimestamp())}>
            今日不再提示
          </Button>
          <Button
            variant="outline"
            onClick={() => handleDismiss(Date.now() + 7 * 24 * 3600 * 1000)}
          >
            7天内不再提示
          </Button>
          <Button onClick={() => handleDismiss(0)}>永久关闭</Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
