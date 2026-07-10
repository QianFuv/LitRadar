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
let createdTask: unknown = null;
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
    job: {
      kind: 'index',
      notify: false,
      push: false,
    },
    legacy_command: null,
    cron: '0 8 * * *',
    enabled: isTaskEnabled,
    last_run_at: null,
    last_status: '',
    created_at: 1,
    updated_at: 2,
  };
}

/**
 * Build a migrated legacy scheduled-task fixture.
 *
 * @returns Disabled legacy task response.
 */
function legacyScheduledTaskFixture() {
  return {
    id: 8,
    name: 'Legacy job',
    job: null,
    legacy_command: 'index --update && push',
    cron: '0 8 * * *',
    enabled: false,
    last_run_at: null,
    last_status: '',
    created_at: 1,
    updated_at: 2,
  };
}

/**
 * Capture a structured scheduled-task creation request.
 *
 * @param context - MSW request context.
 * @returns Created task response.
 */
async function createScheduledTaskResponse(context: { request: Request }): Promise<Response> {
  createdTask = await context.request.json();
  return HttpResponse.json(scheduledTaskFixture());
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

/**
 * Verify creation sends only a typed job specification.
 */
async function createsTypedScheduledTask(): Promise<void> {
  isTaskDeleted = true;
  createdTask = null;
  server.use(
    http.get('http://localhost/api/admin/scheduled-tasks', scheduledTaskListResponse),
    http.post('http://localhost/api/admin/scheduled-tasks', createScheduledTaskResponse),
  );
  const user = userEvent.setup();

  renderWithQuery(<ScheduledTasksCard />);

  await user.click(await screen.findByRole('button', { name: '新建任务' }));
  await user.type(screen.getByLabelText('任务名称'), 'Daily typed task');
  expect(screen.queryByLabelText('自定义执行命令')).not.toBeInTheDocument();
  await user.click(screen.getByRole('button', { name: '创建' }));

  await waitFor(() =>
    expect(createdTask).toEqual({
      cron: '0 8 * * *',
      enabled: true,
      job: {
        kind: 'index',
        notify: true,
        push: false,
      },
      name: 'Daily typed task',
    }),
  );
}

/**
 * Verify a legacy command is read-only until replaced by a typed job.
 */
async function replacesLegacyScheduledTask(): Promise<void> {
  taskPatch = null;
  server.use(
    http.get('http://localhost/api/admin/scheduled-tasks', () =>
      HttpResponse.json([legacyScheduledTaskFixture()]),
    ),
    http.put('http://localhost/api/admin/scheduled-tasks/8', updateScheduledTaskResponse),
  );
  const user = userEvent.setup();

  renderWithQuery(<ScheduledTasksCard />);

  const legacySwitch = await screen.findByRole('switch', {
    name: '旧定时任务 Legacy job 需替换',
  });
  expect(legacySwitch).toBeDisabled();
  expect(screen.getByText(/旧命令（只读）：index --update && push/)).toBeInTheDocument();

  await user.click(screen.getByRole('button', { name: '编辑定时任务 Legacy job' }));
  expect(await screen.findByRole('alert')).toHaveTextContent('旧任务已自动停用');
  expect(screen.queryByLabelText('自定义执行命令')).not.toBeInTheDocument();
  await user.click(screen.getByLabelText('启用任务'));
  await user.click(screen.getByRole('button', { name: '保存' }));

  await waitFor(() =>
    expect(taskPatch).toEqual({
      cron: '0 8 * * *',
      enabled: true,
      job: {
        kind: 'index',
        notify: true,
        push: false,
      },
      name: 'Legacy job',
    }),
  );
}

describe('administrator mutation flow', () => {
  test('updates and deletes a scheduled task', updatesAndDeletesTask);
  test(
    'creates a typed scheduled task without an arbitrary command field',
    createsTypedScheduledTask,
  );
  test('keeps legacy commands read-only until typed replacement', replacesLegacyScheduledTask);
});
