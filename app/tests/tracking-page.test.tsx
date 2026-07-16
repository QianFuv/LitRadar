/**
 * Tracking section rendering, shared textarea, database, save, and secret coverage.
 */

import { screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { http, HttpResponse } from 'msw';
import { beforeEach, describe, expect, test, vi } from 'vitest';

import { TrackingPageContent } from '@/components/tracking/tracking-page-content';
import { server } from '@/tests/mocks/server';
import { renderWithQuery } from '@/tests/render';

const navigationMocks = vi.hoisted(() => ({
  push: vi.fn(),
}));

vi.mock('next/navigation', () => ({
  useRouter: () => ({ push: navigationMocks.push }),
}));

const NOTIFICATION_SETTINGS_FIXTURE = {
  id: 5,
  user_id: 51,
  keywords: ['systems'],
  directions: ['reliability'],
  selected_databases: [],
  delivery_method: 'folder',
  has_pushplus_token: true,
  pushplus_token_mask: '••••',
  pushplus_template: 'markdown',
  pushplus_topic: '',
  pushplus_channel: 'wechat',
  sync_to_tracking_folder: false,
  ai_base_url: 'https://primary.example/v1',
  has_ai_api_key: true,
  ai_api_key_mask: '••••',
  ai_model: 'primary-model',
  ai_system_prompt: 'Primary prompt',
  ai_backup_base_url: 'https://backup.example/v1',
  has_ai_backup_api_key: true,
  ai_backup_api_key_mask: '••••',
  ai_backup_model: 'backup-model',
  ai_backup_system_prompt: 'Backup prompt',
  ai_retry_attempts: 3,
  enabled: true,
  created_at: 1,
  updated_at: 2,
};

let savedSettingsPayload: Record<string, unknown> | null = null;

/**
 * Return current tracking status.
 *
 * @returns Tracking status response.
 */
function trackingStatusResponse(): Response {
  return HttpResponse.json({
    tracking_folder: { id: 4, name: 'Tracking' },
    total_folders: 1,
    weekly_articles_available: 3,
    notification_configured: true,
  });
}

/**
 * Return available databases.
 *
 * @returns Database list response.
 */
function databasesResponse(): Response {
  return HttpResponse.json(['alpha.sqlite', 'beta.sqlite']);
}

/**
 * Return current favorite folders.
 *
 * @returns Folder list response.
 */
function foldersResponse(): Response {
  return HttpResponse.json([
    { id: 4, name: 'Tracking', is_tracking: true, article_count: 2, created_at: 1 },
  ]);
}

/**
 * Return masked notification settings.
 *
 * @returns Notification settings response.
 */
function notificationSettingsResponse(): Response {
  return HttpResponse.json(NOTIFICATION_SETTINGS_FIXTURE);
}

/**
 * Capture a notification update without echoing plaintext secrets.
 *
 * @param context - MSW request context.
 * @returns Updated masked settings response.
 */
async function updateNotificationSettingsResponse(context: {
  request: Request;
}): Promise<Response> {
  savedSettingsPayload = (await context.request.json()) as Record<string, unknown>;
  return HttpResponse.json({
    ...NOTIFICATION_SETTINGS_FIXTURE,
    selected_databases: savedSettingsPayload.selected_databases,
    ai_system_prompt: savedSettingsPayload.ai_system_prompt,
    has_ai_api_key: savedSettingsPayload.ai_api_key !== null,
    updated_at: 3,
  });
}

/**
 * Install the common tracking-page handlers.
 */
function installTrackingPageHandlers(): void {
  server.use(
    http.get('http://localhost/api/tracking/status', trackingStatusResponse),
    http.get('http://localhost/api/meta/databases', databasesResponse),
    http.get('http://localhost/api/favorites/folders', foldersResponse),
    http.get('http://localhost/api/tracking/notification-settings', notificationSettingsResponse),
    http.put(
      'http://localhost/api/tracking/notification-settings',
      updateNotificationSettingsResponse,
    ),
  );
}

/**
 * Verify section boundaries and both shared system-prompt controls.
 */
async function rendersSectionsWithSharedTextareas(): Promise<void> {
  installTrackingPageHandlers();
  renderWithQuery(<TrackingPageContent userId={51} />);

  expect(await screen.findByRole('heading', { name: '追踪文件夹' })).toBeInTheDocument();
  expect(screen.getByRole('heading', { name: '手动推送' })).toBeInTheDocument();
  expect(screen.getByRole('heading', { name: 'AI 推荐配置' })).toBeInTheDocument();
  expect(screen.getByRole('heading', { name: '文献追踪说明' })).toBeInTheDocument();

  const primaryPrompt = await screen.findByLabelText('System Prompt');
  const backupPrompt = screen.getByLabelText('Backup System Prompt');
  expect(primaryPrompt).toHaveAttribute('data-slot', 'textarea');
  expect(primaryPrompt).toHaveClass('shadow-vercel-ring');
  expect(backupPrompt).toHaveAttribute('data-slot', 'textarea');
  expect(backupPrompt).toHaveClass('shadow-vercel-ring');
}

/**
 * Verify database narrowing and secret preserve/clear semantics survive save.
 */
async function savesDatabaseAndSecretSemantics(): Promise<void> {
  installTrackingPageHandlers();
  const user = userEvent.setup();
  renderWithQuery(<TrackingPageContent userId={51} />);

  const betaDatabase = await screen.findByRole('checkbox', { name: 'beta.sqlite' });
  await user.click(betaDatabase);
  const primaryPrompt = screen.getByLabelText('System Prompt');
  await user.clear(primaryPrompt);
  await user.type(primaryPrompt, 'Updated primary prompt');
  await user.click(screen.getAllByRole('button', { name: '清除密钥' })[0]);
  await user.click(screen.getByRole('button', { name: '保存配置' }));

  await waitFor(() => expect(savedSettingsPayload).not.toBeNull());
  expect(savedSettingsPayload?.selected_databases).toEqual(['alpha.sqlite']);
  expect(savedSettingsPayload?.ai_system_prompt).toBe('Updated primary prompt');
  expect(savedSettingsPayload?.ai_api_key).toBeNull();
  expect(savedSettingsPayload).not.toHaveProperty('ai_backup_api_key');
  expect(savedSettingsPayload).not.toHaveProperty('pushplus_token');
}

beforeEach(() => {
  savedSettingsPayload = null;
});

describe('TrackingPageContent', () => {
  test('renders named sections with shared textareas', rendersSectionsWithSharedTextareas);
  test('preserves database and secret update semantics', savesDatabaseAndSecretSemantics);
});
