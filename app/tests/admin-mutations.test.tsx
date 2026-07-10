/**
 * Administrator scheduled-task mutation and cache invalidation coverage.
 */

import { screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { http, HttpResponse } from 'msw';
import { describe, expect, test, vi } from 'vitest';

import { ScheduledTasksCard } from '@/components/admin/scheduled-tasks-card';
import { server } from '@/tests/mocks/server';
import { renderWithQuery } from '@/tests/render';

let isTaskEnabled = true;
let isTaskDeleted = false;
let taskPatch: unknown = null;

/**
 * Build the current scheduled-task fixture.
 *
 * @returns Scheduled task response.
 */
function scheduledTaskFixture() {
  return {
    id: 8,
    name: 'Weekly index',
    command: 'index --resume',
    cron: '0 8 * * *',
    enabled: isTaskEnabled,
    last_run_at: null,
    last_status: '',
    created_at: 1,
    updated_at: 2,
  };
}

/**
 * Return the current scheduled-task list.
 *
 * @returns Task list response.
 */
function scheduledTaskListResponse(): Response {
  return HttpResponse.json(isTaskDeleted ? [] : [scheduledTaskFixture()]);
}

/**
 * Capture and apply a scheduled-task update.
 *
 * @param context - MSW request context.
 * @returns Updated task response.
 */
async function updateScheduledTaskResponse(context: { request: Request }): Promise<Response> {
  taskPatch = await context.request.json();
  if (taskPatch && typeof taskPatch === 'object' && 'enabled' in taskPatch) {
    isTaskEnabled = Boolean((taskPatch as Record<string, unknown>).enabled);
  }
  return HttpResponse.json(scheduledTaskFixture());
}

/**
 * Delete the scheduled-task fixture.
 *
 * @returns Successful delete response.
 */
function deleteScheduledTaskResponse(): Response {
  isTaskDeleted = true;
  return HttpResponse.json({ ok: true });
}

/**
 * Verify update and delete mutations invalidate and refresh the task list.
 */
async function updatesAndDeletesTask(): Promise<void> {
  isTaskEnabled = true;
  isTaskDeleted = false;
  taskPatch = null;
  vi.spyOn(window, 'confirm').mockReturnValue(true);
  server.use(
    http.get('http://localhost/api/admin/scheduled-tasks', scheduledTaskListResponse),
    http.put('http://localhost/api/admin/scheduled-tasks/8', updateScheduledTaskResponse),
    http.delete('http://localhost/api/admin/scheduled-tasks/8', deleteScheduledTaskResponse),
  );
  const user = userEvent.setup();

  renderWithQuery(<ScheduledTasksCard />);

  await user.click(await screen.findByRole('switch', { name: '停用定时任务 Weekly index' }));
  await waitFor(() => expect(taskPatch).toEqual({ enabled: false }));
  expect(
    await screen.findByRole('switch', { name: '启用定时任务 Weekly index' }),
  ).toBeInTheDocument();

  await user.click(screen.getByRole('button', { name: '删除定时任务 Weekly index' }));
  expect(await screen.findByText('暂无定时任务')).toBeInTheDocument();
  expect(isTaskDeleted).toBe(true);
}

describe('administrator mutation flow', () => {
  test('updates and deletes a scheduled task', updatesAndDeletesTask);
});
