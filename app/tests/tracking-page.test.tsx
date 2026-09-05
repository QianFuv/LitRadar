/**
 * Tracking section rendering, shared textarea, database, save, and secret coverage.
 */

import { act, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { http, HttpResponse } from 'msw';
import { beforeEach, describe, expect, test, vi } from 'vitest';

import { TrackingSettingsContent } from '@/components/tracking/tracking-settings-content';
import { idleManualPushStatusResponse } from '@/tests/mocks/handlers/tracking';
import { server } from '@/tests/mocks/server';
import { renderWithQuery } from '@/tests/render';

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
  ai_base_url: 'https://primary.example/v1/',
  has_ai_api_key: true,
  ai_api_key_mask: '••••',
  ai_model: 'primary-model',
  ai_system_prompt: 'Primary prompt',
  ai_backup_base_url: 'https://backup.example/v1/',
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
 * Return the administrator-approved AI endpoint catalog.
 *
 * @returns AI endpoint list response.
 */
function aiEndpointsResponse(): Response {
  return HttpResponse.json([
    'https://primary.example/v1/',
    'https://backup.example/v1/',
    'https://new-primary.example/v1/',
    'https://new-backup.example/v1/',
  ]);
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
    http.get('http://localhost/api/tracking/ai-endpoints', aiEndpointsResponse),
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
  const { rerender } = renderWithQuery(<TrackingSettingsContent userId={51} section="tracking" />);

  expect(await screen.findByRole('heading', { name: '追踪文件夹' })).toBeInTheDocument();
  expect(screen.getByRole('heading', { name: 'AI 推荐配置' })).toBeInTheDocument();
  expect(screen.getByRole('heading', { name: '文献追踪说明' })).toBeInTheDocument();

  const primaryPrompt = await screen.findByLabelText('System Prompt');
  const backupPrompt = screen.getByLabelText('Backup System Prompt');
  expect(primaryPrompt).toHaveAttribute('data-slot', 'textarea');
  expect(primaryPrompt).toHaveClass('shadow-vercel-ring');
  expect(backupPrompt).toHaveAttribute('data-slot', 'textarea');
  expect(backupPrompt).toHaveClass('shadow-vercel-ring');

  rerender(<TrackingSettingsContent userId={51} section="notifications" />);
  expect(screen.getByRole('heading', { name: '通知与推送' })).toBeInTheDocument();
  expect(screen.getByRole('heading', { name: '手动推送' })).toBeInTheDocument();
}

/**
 * Verify database narrowing and secret preserve/clear semantics survive save.
 */
async function savesDatabaseAndSecretSemantics(): Promise<void> {
  installTrackingPageHandlers();
  const user = userEvent.setup();
  renderWithQuery(<TrackingSettingsContent userId={51} section="tracking" />);

  const betaDatabase = await screen.findByRole('checkbox', { name: 'beta.sqlite' });
  await user.click(betaDatabase);
  const primaryPrompt = screen.getByLabelText('System Prompt');
  await user.clear(primaryPrompt);
  await user.type(primaryPrompt, 'Updated primary prompt');
  await user.click(screen.getAllByRole('button', { name: '清除密钥' })[0]);
  await user.click(screen.getByRole('button', { name: '保存更改' }));

  await waitFor(() => expect(savedSettingsPayload).not.toBeNull());
  expect(savedSettingsPayload?.selected_databases).toEqual(['alpha.sqlite']);
  expect(savedSettingsPayload?.ai_system_prompt).toBe('Updated primary prompt');
  expect(savedSettingsPayload?.ai_api_key).toBeNull();
  expect(savedSettingsPayload).not.toHaveProperty('ai_backup_api_key');
  expect(savedSettingsPayload).not.toHaveProperty('pushplus_token');
}

/**
 * Verify a database catalog failure cannot widen a stored narrow scope during an unrelated save.
 */
async function preservesDatabaseScopeWhenCatalogFails(): Promise<void> {
  installTrackingPageHandlers();
  server.use(
    http.get('http://localhost/api/meta/databases', () =>
      HttpResponse.json({ detail: 'Database catalog unavailable' }, { status: 503 }),
    ),
    http.get('http://localhost/api/tracking/notification-settings', () =>
      HttpResponse.json({
        ...NOTIFICATION_SETTINGS_FIXTURE,
        selected_databases: ['legacy.sqlite'],
      }),
    ),
  );
  const user = userEvent.setup();
  renderWithQuery(<TrackingSettingsContent userId={51} section="tracking" />);

  expect(await screen.findByRole('alert')).toHaveTextContent('Database catalog unavailable');
  await user.click(screen.getByRole('switch', { name: '启用推荐' }));
  await user.click(screen.getByRole('button', { name: '保存更改' }));

  await waitFor(() => expect(savedSettingsPayload).not.toBeNull());
  expect(savedSettingsPayload?.selected_databases).toEqual(['legacy.sqlite']);
  expect(savedSettingsPayload?.enabled).toBe(false);
}

/**
 * Verify a failed settings save retains the shared draft and retries the same payload.
 */
async function retriesFailedSettingsSave(): Promise<void> {
  installTrackingPageHandlers();
  const savePayloads: Array<Record<string, unknown>> = [];
  server.use(
    http.put('http://localhost/api/tracking/notification-settings', async ({ request }) => {
      const payload = (await request.json()) as Record<string, unknown>;
      savePayloads.push(payload);
      if (savePayloads.length === 1) {
        return HttpResponse.json({ detail: 'Settings save unavailable' }, { status: 503 });
      }
      return HttpResponse.json({
        ...NOTIFICATION_SETTINGS_FIXTURE,
        enabled: payload.enabled,
        updated_at: 3,
      });
    }),
  );
  const user = userEvent.setup();
  renderWithQuery(<TrackingSettingsContent userId={51} section="tracking" />);

  const recommendationSwitch = await screen.findByRole('switch', { name: '启用推荐' });
  await user.click(recommendationSwitch);
  const saveButton = screen.getByRole('button', { name: '保存更改' });
  await user.click(saveButton);

  expect(await screen.findByRole('alert')).toHaveTextContent('Settings save unavailable');
  expect(recommendationSwitch).not.toBeChecked();
  expect(saveButton).toBeEnabled();
  expect(screen.getByRole('button', { name: '取消更改' })).toBeEnabled();
  await user.click(saveButton);

  expect(await screen.findByText('已保存')).toBeInTheDocument();
  await waitFor(() => expect(saveButton).toBeDisabled());
  expect(savePayloads).toHaveLength(2);
  expect(savePayloads[1]).toEqual(savePayloads[0]);
  expect(savePayloads[1]).toMatchObject({ enabled: false });
}

/** Verify tracking and notification categories retain one unsaved draft instance. */
async function preservesDraftAcrossCategories(): Promise<void> {
  installTrackingPageHandlers();
  const user = userEvent.setup();
  const { rerender } = renderWithQuery(<TrackingSettingsContent userId={51} section="tracking" />);

  const recommendationSwitch = await screen.findByRole('switch', { name: '启用推荐' });
  await user.click(recommendationSwitch);
  expect(recommendationSwitch).not.toBeChecked();
  expect(screen.getByRole('button', { name: '取消更改' })).toBeEnabled();

  rerender(<TrackingSettingsContent userId={51} section="notifications" />);
  expect(screen.getByRole('button', { name: '取消更改' })).toBeEnabled();

  rerender(<TrackingSettingsContent userId={51} section="tracking" />);
  expect(screen.getByRole('switch', { name: '启用推荐' })).not.toBeChecked();
}

/**
 * Verify every recommendation and delivery field reaches the save contract with explicit secrets.
 */
async function savesCompleteRecommendationAndDeliverySettings(): Promise<void> {
  installTrackingPageHandlers();
  const user = userEvent.setup();
  const { rerender } = renderWithQuery(<TrackingSettingsContent userId={51} section="tracking" />);

  await screen.findByLabelText('关键词');
  const existingKeywordChip = document.querySelector(
    '[data-motion-preference-chip="keyword-systems"]',
  );
  expect(existingKeywordChip).not.toBeNull();
  await user.type(screen.getByLabelText('关键词'), 'distributed');
  await user.click(screen.getByRole('button', { name: '添加关键词' }));
  expect(document.querySelector('[data-motion-preference-chip="keyword-systems"]')).toBe(
    existingKeywordChip,
  );
  expect(
    document.querySelector('[data-motion-preference-chip="keyword-distributed"]'),
  ).not.toBeNull();
  await user.click(screen.getByRole('button', { name: '移除关键词 distributed' }));
  await waitFor(() =>
    expect(
      document.querySelector('[data-motion-preference-chip="keyword-distributed"]'),
    ).toBeNull(),
  );
  expect(document.querySelector('[data-motion-preference-chip="keyword-systems"]')).toBe(
    existingKeywordChip,
  );
  await user.type(screen.getByLabelText('关键词'), 'distributed');
  await user.click(screen.getByRole('button', { name: '添加关键词' }));
  await user.type(screen.getByLabelText('研究方向'), 'security');
  await user.click(screen.getByRole('button', { name: '添加研究方向' }));

  const fieldUpdates: Array<[string, string]> = [
    ['Model', 'primary-next'],
    ['API Key', 'primary-secret'],
    ['System Prompt', 'Primary updated'],
    ['Backup Model', 'backup-next'],
    ['Backup API Key', 'backup-secret'],
    ['Backup System Prompt', 'Backup updated'],
  ];
  for (const [label, value] of fieldUpdates) {
    const field = screen.getByLabelText(label);
    await user.clear(field);
    await user.type(field, value);
  }
  const primaryEndpoint = screen.getByRole('combobox', { name: 'Base URL' });
  primaryEndpoint.focus();
  await user.keyboard('{ArrowDown}');
  expect(
    await screen.findByRole('option', { name: 'https://new-primary.example/v1/' }),
  ).toBeInTheDocument();
  await user.keyboard('{ArrowDown}{ArrowDown}{Enter}');
  const backupEndpoint = screen.getByRole('combobox', { name: 'Backup Base URL' });
  backupEndpoint.focus();
  await user.keyboard('{ArrowDown}');
  expect(
    await screen.findByRole('option', { name: 'https://new-backup.example/v1/' }),
  ).toBeInTheDocument();
  await user.keyboard('{ArrowDown}{ArrowDown}{Enter}');
  const retryAttempts = screen.getByLabelText('失败重试次数');
  await user.click(retryAttempts);
  await user.keyboard('{Control>}a{/Control}5');

  rerender(<TrackingSettingsContent userId={51} section="notifications" />);
  const deliverySelect = screen.getByRole('combobox', { name: '推送方式' });
  const deliveryPanel = document.querySelector('[data-motion-delivery-panel="pushplus"]');
  expect(deliveryPanel).toHaveAttribute('inert');
  expect(deliveryPanel).toHaveAttribute('aria-hidden', 'true');
  deliverySelect.focus();
  await user.keyboard('{ArrowDown}');
  expect(await screen.findByRole('option', { name: 'PushPlus 外部推送' })).toBeInTheDocument();
  await user.keyboard('{ArrowDown}{Enter}');
  await waitFor(() => expect(deliveryPanel).not.toHaveAttribute('inert'));
  expect(deliveryPanel).toHaveAttribute('aria-hidden', 'false');

  const deliveryUpdates: Array<[string, string]> = [
    ['PushPlus 令牌', 'pushplus-secret'],
    ['模板', 'html'],
    ['主题', 'Weekly digest'],
    ['渠道', 'email'],
  ];
  for (const [label, value] of deliveryUpdates) {
    const field = screen.getByLabelText(label);
    await user.clear(field);
    await user.type(field, value);
  }
  const pushplusToken = screen.getByLabelText('PushPlus 令牌');
  deliverySelect.focus();
  await user.keyboard('{Enter}{Home}{Enter}');
  await waitFor(() => expect(deliveryPanel).toHaveAttribute('inert'));
  expect(pushplusToken).toHaveValue('pushplus-secret');
  expect(pushplusToken).not.toHaveFocus();
  deliverySelect.focus();
  await user.keyboard('{Enter}{End}{Enter}');
  await waitFor(() => expect(deliveryPanel).not.toHaveAttribute('inert'));
  expect(screen.getByLabelText('PushPlus 令牌')).toHaveValue('pushplus-secret');
  await user.click(screen.getByRole('switch', { name: '同步写入追踪文件夹' }));
  await user.click(screen.getByRole('button', { name: '保存更改' }));

  await waitFor(() => expect(savedSettingsPayload).not.toBeNull());
  expect(savedSettingsPayload).toEqual({
    keywords: ['systems', 'distributed'],
    directions: ['reliability', 'security'],
    selected_databases: [],
    delivery_method: 'pushplus',
    pushplus_token: 'pushplus-secret',
    pushplus_template: 'html',
    pushplus_topic: 'Weekly digest',
    pushplus_channel: 'email',
    sync_to_tracking_folder: true,
    ai_base_url: 'https://new-primary.example/v1/',
    ai_api_key: 'primary-secret',
    ai_model: 'primary-next',
    ai_system_prompt: 'Primary updated',
    ai_backup_base_url: 'https://new-backup.example/v1/',
    ai_backup_api_key: 'backup-secret',
    ai_backup_model: 'backup-next',
    ai_backup_system_prompt: 'Backup updated',
    ai_retry_attempts: 5,
    enabled: true,
  });
}

/**
 * Verify selecting and creating tracking folders refreshes authoritative status.
 */
async function selectsAndCreatesTrackingFolders(): Promise<void> {
  let trackingFolderId = 4;
  const folders = [
    { id: 4, name: 'Tracking', is_tracking: true, article_count: 2, created_at: 1 },
    { id: 5, name: 'Archive', is_tracking: false, article_count: 0, created_at: 2 },
  ];
  const setPayloads: unknown[] = [];
  const createPayloads: unknown[] = [];
  server.use(
    http.get('http://localhost/api/tracking/status', () => {
      const folder = folders.find((candidate) => candidate.id === trackingFolderId) ?? null;
      return HttpResponse.json({
        tracking_folder: folder,
        total_folders: folders.length,
        weekly_articles_available: 3,
        notification_configured: true,
      });
    }),
    http.get('http://localhost/api/meta/databases', databasesResponse),
    http.get('http://localhost/api/favorites/folders', () => HttpResponse.json(folders)),
    http.get('http://localhost/api/tracking/notification-settings', notificationSettingsResponse),
    http.get('http://localhost/api/tracking/ai-endpoints', aiEndpointsResponse),
    http.put('http://localhost/api/favorites/tracking', async ({ request }) => {
      const payload = (await request.json()) as { folder_id: number };
      setPayloads.push(payload);
      trackingFolderId = payload.folder_id;
      for (const folder of folders) {
        folder.is_tracking = folder.id === trackingFolderId;
      }
      return HttpResponse.json({ ok: true });
    }),
    http.post('http://localhost/api/favorites/folders', async ({ request }) => {
      const payload = (await request.json()) as { is_tracking: boolean; name: string };
      createPayloads.push(payload);
      const folder = {
        id: 6,
        name: payload.name,
        is_tracking: payload.is_tracking,
        article_count: 0,
        created_at: 3,
      };
      folders.push(folder);
      trackingFolderId = folder.id;
      return HttpResponse.json(folder);
    }),
  );
  const user = userEvent.setup();
  renderWithQuery(<TrackingSettingsContent userId={51} section="tracking" />);

  expect(await screen.findByText('当前追踪: Tracking')).toBeInTheDocument();
  const folderSelect = screen.getAllByRole('combobox')[0];
  folderSelect.focus();
  await user.keyboard('{ArrowDown}');
  expect(await screen.findByRole('option', { name: 'Archive (0)' })).toBeInTheDocument();
  await user.keyboard('{ArrowDown}{Enter}');
  await waitFor(() => expect(setPayloads).toEqual([{ folder_id: 5 }]));
  expect(await screen.findByText('当前追踪: Archive')).toBeInTheDocument();

  await user.type(screen.getByLabelText('新建追踪文件夹名称'), '  New Tracking  ');
  await user.click(screen.getByRole('button', { name: '创建并设为追踪' }));
  await waitFor(() =>
    expect(createPayloads).toEqual([{ name: 'New Tracking', is_tracking: true }]),
  );
  expect(await screen.findByText('当前追踪: New Tracking')).toBeInTheDocument();
  expect(screen.getByLabelText('新建追踪文件夹名称')).toHaveValue('');
}

/**
 * Verify a failed tracking-folder selection remains visible to the user.
 */
async function reportsTrackingFolderFailure(): Promise<void> {
  server.use(
    http.get('http://localhost/api/tracking/status', trackingStatusResponse),
    http.get('http://localhost/api/meta/databases', databasesResponse),
    http.get('http://localhost/api/favorites/folders', () =>
      HttpResponse.json([
        { id: 4, name: 'Tracking', is_tracking: true, article_count: 2, created_at: 1 },
        { id: 5, name: 'Archive', is_tracking: false, article_count: 0, created_at: 2 },
      ]),
    ),
    http.get('http://localhost/api/tracking/notification-settings', notificationSettingsResponse),
    http.get('http://localhost/api/tracking/ai-endpoints', aiEndpointsResponse),
    http.put('http://localhost/api/favorites/tracking', () =>
      HttpResponse.json({ detail: 'Tracking folder unavailable' }, { status: 503 }),
    ),
  );
  const user = userEvent.setup();
  renderWithQuery(<TrackingSettingsContent userId={51} section="tracking" />);

  const folderSelect = (await screen.findAllByRole('combobox'))[0];
  folderSelect.focus();
  await user.keyboard('{ArrowDown}{ArrowDown}{Enter}');
  expect(await screen.findByRole('alert')).toHaveTextContent('Tracking folder unavailable');
  expect(screen.getByText('当前追踪: Tracking')).toBeInTheDocument();
}

beforeEach(() => {
  savedSettingsPayload = null;
  server.use(
    http.get('http://localhost/api/tracking/push-weekly/status', idleManualPushStatusResponse),
  );
  Object.defineProperty(HTMLElement.prototype, 'scrollIntoView', {
    configurable: true,
    value: vi.fn(),
  });
});

/** Verify pending saves disable every editable tracking field until the response settles. */
async function blocksTrackingEditsWhileSaving(): Promise<void> {
  installTrackingPageHandlers();
  let finishSave: (() => void) | undefined;
  const saveGate = new Promise<void>((resolve) => {
    finishSave = resolve;
  });
  server.use(
    http.put('http://localhost/api/tracking/notification-settings', async (context) => {
      await saveGate;
      return updateNotificationSettingsResponse(context);
    }),
  );
  const user = userEvent.setup();
  renderWithQuery(<TrackingSettingsContent userId={51} section="tracking" />);
  const prompt = await screen.findByLabelText('System Prompt');
  await user.clear(prompt);
  await user.type(prompt, 'Submitted prompt');
  await user.click(screen.getByRole('button', { name: '保存更改' }));
  try {
    expect(prompt).toBeDisabled();
    expect(screen.getByLabelText('关键词')).toBeDisabled();
    await user.type(prompt, ' unwanted edit');
    expect(prompt).toHaveValue('Submitted prompt');
  } finally {
    await act(async () => {
      finishSave?.();
    });
  }
  await waitFor(() => expect(prompt).not.toBeDisabled());
  expect(savedSettingsPayload?.ai_system_prompt).toBe('Submitted prompt');
}

describe('TrackingSettingsContent', () => {
  test('blocks tracking edits during a pending save', blocksTrackingEditsWhileSaving);
  test('renders named sections with shared textareas', rendersSectionsWithSharedTextareas);
  test('preserves database and secret update semantics', savesDatabaseAndSecretSemantics, 10_000);
  test('preserves database scope when the catalog fails', preservesDatabaseScopeWhenCatalogFails);
  test('retries a failed settings save without losing the draft', retriesFailedSettingsSave);
  test('preserves one draft across tracking categories', preservesDraftAcrossCategories);
  test(
    'saves complete recommendation and delivery settings',
    savesCompleteRecommendationAndDeliverySettings,
    30_000,
  );
  test('selects and creates tracking folders', selectsAndCreatesTrackingFolders);
  test('reports tracking folder failures', reportsTrackingFolderFailure);
});
