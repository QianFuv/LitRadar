'use client';

/**
 * Announcement modal flow for the desktop frontend.
 */

import { useQuery } from '@tanstack/react-query';
import { useMemo, useState } from 'react';
import { BellRing } from 'lucide-react';
import { getAnnouncements, type AnnouncementInfo } from '@/lib/client-api';
import { Badge, Button, Modal, Notice } from '@/components/desktop/ui';

const DISMISS_PREFIX = 'paper_scanner_announcement_dismissed_';

const PRIORITY_TONES: Record<AnnouncementInfo['priority'], 'coral' | 'neutral' | 'violet'> = {
  high: 'coral',
  low: 'neutral',
  normal: 'violet',
};

const PRIORITY_LABELS: Record<AnnouncementInfo['priority'], string> = {
  high: '高优先级',
  low: '低优先级',
  normal: '普通',
};

/**
 * Build the local-storage key for an announcement.
 *
 * @param id - Announcement id.
 * @returns Storage key.
 */
function getDismissKey(id: number): string {
  return `${DISMISS_PREFIX}${id}`;
}

/**
 * Read the timestamp until which an announcement is dismissed.
 *
 * @param id - Announcement id.
 * @returns Dismissed-until timestamp, zero for permanent dismissal, or null.
 */
function readDismissedUntil(id: number): number | null {
  if (typeof window === 'undefined') {
    return null;
  }
  const rawValue = window.localStorage.getItem(getDismissKey(id));
  if (rawValue === null) {
    return null;
  }
  const value = Number(rawValue);
  if (!Number.isFinite(value)) {
    window.localStorage.removeItem(getDismissKey(id));
    return null;
  }
  if (value === 0 || value > Date.now()) {
    return value;
  }
  window.localStorage.removeItem(getDismissKey(id));
  return null;
}

/**
 * Check whether an announcement should be hidden.
 *
 * @param announcement - Announcement record.
 * @returns Whether it is dismissed.
 */
function isDismissed(announcement: AnnouncementInfo): boolean {
  return readDismissedUntil(announcement.id) !== null;
}

/**
 * Get the end timestamp for the current day.
 *
 * @returns End-of-day timestamp.
 */
function getEndOfToday(): number {
  const date = new Date();
  date.setHours(23, 59, 59, 999);
  return date.getTime();
}

/**
 * Persist dismissal for announcements.
 *
 * @param announcements - Announcements to dismiss.
 * @param dismissedUntil - Dismissal timestamp.
 */
function dismissAnnouncements(announcements: AnnouncementInfo[], dismissedUntil: number): void {
  for (const announcement of announcements) {
    window.localStorage.setItem(getDismissKey(announcement.id), String(dismissedUntil));
  }
}

/**
 * Render the announcement modal when unread announcements exist.
 *
 * @returns Announcement modal or null.
 */
export function AnnouncementsModal() {
  const [closedSignature, setClosedSignature] = useState<string | null>(null);
  const { data = [] } = useQuery({
    queryKey: ['announcements'],
    queryFn: getAnnouncements,
    refetchInterval: 60_000,
  });

  const unreadAnnouncements = useMemo(
    () => data.filter((announcement) => !isDismissed(announcement)),
    [data],
  );
  const unreadSignature = unreadAnnouncements.map((announcement) => announcement.id).join(',');
  const open = unreadAnnouncements.length > 0 && closedSignature !== unreadSignature;

  const dismiss = (dismissedUntil: number) => {
    dismissAnnouncements(unreadAnnouncements, dismissedUntil);
    setClosedSignature(unreadSignature);
  };

  if (unreadAnnouncements.length === 0) {
    return null;
  }

  return (
    <Modal
      open={open}
      title={
        <span className="toolbar">
          <BellRing size={18} />
          系统公告
        </span>
      }
      description="以下公告尚未阅读，可按你的工作节奏关闭提示。"
      onClose={() => setClosedSignature(unreadSignature)}
      footer={
        <>
          <Button variant="secondary" onClick={() => dismiss(getEndOfToday())}>
            今日不再提示
          </Button>
          <Button variant="secondary" onClick={() => dismiss(Date.now() + 7 * 24 * 3600 * 1000)}>
            7 天内不再提示
          </Button>
          <Button onClick={() => dismiss(0)}>永久关闭</Button>
        </>
      }
    >
      <div className="list-stack">
        {unreadAnnouncements.map((announcement) => (
          <Notice key={announcement.id}>
            <div className="toolbar toolbar--wrap">
              <strong>{announcement.title}</strong>
              <Badge tone={PRIORITY_TONES[announcement.priority]}>
                {PRIORITY_LABELS[announcement.priority]}
              </Badge>
            </div>
            <p className="modal__description" style={{ whiteSpace: 'pre-wrap' }}>
              {announcement.message}
            </p>
          </Notice>
        ))}
      </div>
    </Modal>
  );
}
