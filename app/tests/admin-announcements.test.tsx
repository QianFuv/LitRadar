/**
 * Administrator announcement creation, editing, toggling, deletion, and recovery coverage.
 */

import { screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { http, HttpResponse } from 'msw';
import { beforeEach, describe, expect, test } from 'vitest';

import { AnnouncementsCard } from '@/components/admin/announcements-card';
import type { AnnouncementInfo } from '@/lib/api';
import { server } from '@/tests/mocks/server';
import { renderWithQuery } from '@/tests/render';

let announcements: AnnouncementInfo[] = [];

/**
 * Build one administrator announcement response.
 *
 * @param overrides - Announcement fields to replace.
 * @returns Complete announcement response.
 */
function announcementFixture(overrides: Partial<AnnouncementInfo> = {}): AnnouncementInfo {
  return {
    id: 51,
    title: 'Maintenance',
    message: 'Tonight',
    priority: 'normal',
    enabled: true,
    created_at: 1_900_000_000,
    updated_at: 1_900_000_100,
    ...overrides,
  };
}

/**
 * Return the current authoritative announcement list.
 *
 * @returns Current announcements.
 */
function announcementListResponse(): Response {
  return HttpResponse.json(announcements);
}

/**
 * Verify create, edit, priority, and enabled mutations commit after refetch.
 */
async function managesAnnouncementLifecycle(): Promise<void> {
  announcements = [];
  const createPayloads: unknown[] = [];
  const updatePayloads: unknown[] = [];
  server.use(
    http.get('http://localhost/api/admin/announcements', announcementListResponse),
    http.post('http://localhost/api/admin/announcements', async ({ request }) => {
      const payload = (await request.json()) as Omit<
        AnnouncementInfo,
        'id' | 'created_at' | 'updated_at'
      >;
      createPayloads.push(payload);
      const createdAnnouncement = announcementFixture({ id: 52, ...payload });
      announcements = [createdAnnouncement];
      return HttpResponse.json(createdAnnouncement);
    }),
    http.put('http://localhost/api/admin/announcements/:announcementId', async ({ request }) => {
      const payload = (await request.json()) as Partial<AnnouncementInfo>;
      updatePayloads.push(payload);
      announcements = announcements.map((announcement) => ({
        ...announcement,
        ...payload,
        updated_at: announcement.updated_at + 1,
      }));
      return HttpResponse.json(announcements[0]);
    }),
  );
  const user = userEvent.setup();
  renderWithQuery(<AnnouncementsCard />);

  expect(await screen.findByText('暂无公告')).toBeInTheDocument();
  await user.click(screen.getByRole('button', { name: '新建公告' }));
  const createDialog = screen.getByRole('dialog', { name: '新建公告' });
  await user.type(within(createDialog).getByLabelText('公告标题'), 'New release');
  await user.type(within(createDialog).getByLabelText('公告内容'), 'Available now');
  const createPriority = within(createDialog).getByRole('combobox', { name: '优先级' });
  createPriority.focus();
  await user.keyboard('{Enter}{Home}{Enter}');
  await waitFor(() => expect(createPriority).toHaveTextContent('高优先级'));
  await user.click(within(createDialog).getByRole('button', { name: '创建' }));

  expect(await screen.findByText('New release')).toBeInTheDocument();
  expect(screen.getByText('高优先级')).toBeInTheDocument();
  expect(createPayloads).toEqual([
    {
      enabled: true,
      message: 'Available now',
      priority: 'high',
      title: 'New release',
    },
  ]);

  await user.click(screen.getByRole('button', { name: '编辑公告 New release' }));
  const editDialog = screen.getByRole('dialog', { name: '编辑公告' });
  const titleInput = within(editDialog).getByLabelText('公告标题');
  const messageInput = within(editDialog).getByLabelText('公告内容');
  await user.clear(titleInput);
  await user.type(titleInput, 'Updated release');
  await user.clear(messageInput);
  await user.type(messageInput, 'Updated details');
  const editPriority = within(editDialog).getByRole('combobox', { name: '优先级' });
  editPriority.focus();
  await user.keyboard('{Enter}{End}{Enter}');
  await waitFor(() => expect(editPriority).toHaveTextContent('低优先级'));
  await user.click(within(editDialog).getByRole('button', { name: '保存' }));

  expect(await screen.findByText('Updated release')).toBeInTheDocument();
  expect(screen.getByText('低优先级')).toBeInTheDocument();
  await user.click(screen.getByRole('switch', { name: '停用公告 Updated release' }));
  expect(
    await screen.findByRole('switch', { name: '启用公告 Updated release' }),
  ).toBeInTheDocument();
  expect(updatePayloads).toEqual([
    {
      enabled: true,
      message: 'Updated details',
      priority: 'low',
      title: 'Updated release',
    },
    { enabled: false },
  ]);
}

/**
 * Verify a failed create retains the form and can be retried.
 */
async function retriesFailedAnnouncementCreation(): Promise<void> {
  announcements = [];
  let createRequestCount = 0;
  server.use(
    http.get('http://localhost/api/admin/announcements', announcementListResponse),
    http.post('http://localhost/api/admin/announcements', async ({ request }) => {
      createRequestCount += 1;
      const payload = (await request.json()) as Omit<
        AnnouncementInfo,
        'id' | 'created_at' | 'updated_at'
      >;
      if (createRequestCount === 1) {
        return HttpResponse.json({ detail: 'Announcement limit reached' }, { status: 422 });
      }
      const createdAnnouncement = announcementFixture({ id: 52, ...payload });
      announcements = [createdAnnouncement];
      return HttpResponse.json(createdAnnouncement);
    }),
  );
  const user = userEvent.setup();
  renderWithQuery(<AnnouncementsCard />);

  await user.click(await screen.findByRole('button', { name: '新建公告' }));
  const dialog = screen.getByRole('dialog', { name: '新建公告' });
  const titleInput = within(dialog).getByLabelText('公告标题');
  const messageInput = within(dialog).getByLabelText('公告内容');
  await user.type(titleInput, 'Retained title');
  await user.type(messageInput, 'Retained message');
  await user.click(within(dialog).getByRole('button', { name: '创建' }));

  expect(await within(dialog).findByRole('alert')).toHaveTextContent('Announcement limit reached');
  expect(titleInput).toHaveValue('Retained title');
  expect(messageInput).toHaveValue('Retained message');
  await user.click(within(dialog).getByRole('button', { name: '创建' }));

  expect(await screen.findByText('Retained title')).toBeInTheDocument();
  expect(screen.queryByRole('dialog', { name: '新建公告' })).not.toBeInTheDocument();
  expect(createRequestCount).toBe(2);
}

/**
 * Verify failed deletion retains the selected announcement and succeeds on retry.
 */
async function retriesFailedAnnouncementDeletion(): Promise<void> {
  let deleteRequestCount = 0;
  server.use(
    http.get('http://localhost/api/admin/announcements', announcementListResponse),
    http.delete('http://localhost/api/admin/announcements/51', () => {
      deleteRequestCount += 1;
      if (deleteRequestCount === 1) {
        return HttpResponse.json({ detail: 'Announcement deletion unavailable' }, { status: 503 });
      }
      announcements = [];
      return HttpResponse.json({ ok: true });
    }),
  );
  const user = userEvent.setup();
  renderWithQuery(<AnnouncementsCard />);

  await user.click(await screen.findByRole('button', { name: '删除公告 Maintenance' }));
  const dialog = screen.getByRole('alertdialog', { name: '删除公告？' });
  expect(dialog).toHaveTextContent('Maintenance');
  await user.click(within(dialog).getByRole('button', { name: '确认删除' }));

  expect(await within(dialog).findByRole('alert')).toHaveTextContent(
    'Announcement deletion unavailable',
  );
  expect(dialog).toHaveTextContent('Maintenance');
  await user.click(within(dialog).getByRole('button', { name: '确认删除' }));

  expect(await screen.findByText('暂无公告')).toBeInTheDocument();
  expect(deleteRequestCount).toBe(2);
}

beforeEach(() => {
  announcements = [announcementFixture()];
  Object.defineProperty(HTMLElement.prototype, 'scrollIntoView', {
    configurable: true,
    value: () => undefined,
  });
});

describe('administrator announcements', () => {
  test(
    'creates, edits, prioritizes, and toggles an announcement',
    managesAnnouncementLifecycle,
    20_000,
  );
  test('retries a failed announcement creation', retriesFailedAnnouncementCreation);
  test('retries a failed announcement deletion', retriesFailedAnnouncementDeletion);
});
