/**
 * Runtime secret-pool management component coverage.
 */

import { screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { http, HttpResponse } from 'msw';
import { describe, expect, test } from 'vitest';

import { RuntimeSettingsCard } from '@/components/admin/runtime-settings-card';
import { server } from '@/tests/mocks/server';
import { renderWithQuery } from '@/tests/render';

let updatePayload: unknown = null;

/**
 * Return configured secret and non-secret runtime pools.
 *
 * @returns Masked runtime settings response.
 */
function runtimeSettingsFixture() {
  return [
    {
      field: 'openalex_api_key_pool',
      label: 'OpenAlex API key pool',
      description: 'OpenAlex authenticated request key pool.',
      input_type: 'password',
      is_secret: true,
      value: '',
      has_value: true,
      masked_value: '••••',
      secret_items: [
        { reference: 'reference-one', masked_value: 'abcde****' },
        { reference: 'reference-two', masked_value: 'vwxyz****' },
      ],
      source: 'database',
      updated_at: 1,
    },
    {
      field: 'crossref_mailto_pool',
      label: 'Crossref mailto pool',
      description: 'Comma- or semicolon-separated Crossref contact emails.',
      input_type: 'email',
      is_secret: false,
      value: 'first@example.test,second@example.test',
      has_value: true,
      masked_value: '',
      secret_items: [],
      source: 'database',
      updated_at: 1,
    },
  ];
}

/**
 * Capture a runtime settings update.
 *
 * @param context - MSW request context.
 * @returns Masked runtime settings response.
 */
async function updateRuntimeSettings(context: { request: Request }): Promise<Response> {
  updatePayload = await context.request.json();
  return HttpResponse.json(runtimeSettingsFixture());
}

/**
 * Render the runtime settings card with mock API handlers.
 */
function renderRuntimeSettings(): void {
  server.use(
    http.get('http://localhost/api/admin/runtime-settings', () =>
      HttpResponse.json(runtimeSettingsFixture()),
    ),
    http.put('http://localhost/api/admin/runtime-settings', updateRuntimeSettings),
  );
  renderWithQuery(<RuntimeSettingsCard />);
}

/**
 * Verify stored masks and count render while an untouched pool is omitted.
 */
async function rendersAndPreservesStoredSecrets(): Promise<void> {
  updatePayload = null;
  renderRuntimeSettings();
  const user = userEvent.setup();

  expect(await screen.findByText('2 个密钥')).toBeInTheDocument();
  expect(screen.getByText('abcde****')).toBeInTheDocument();
  expect(screen.getByText('vwxyz****')).toBeInTheDocument();
  expect(screen.getByText(/已安全保存/)).toBeInTheDocument();
  await user.click(screen.getByRole('button', { name: '保存配置' }));

  await waitFor(() => expect(updatePayload).toEqual({ values: {}, secret_pool_updates: {} }));
}

/**
 * Verify one stored key can be removed, restored, and removed alongside an addition.
 */
async function updatesStoredSecretPool(): Promise<void> {
  updatePayload = null;
  renderRuntimeSettings();
  const user = userEvent.setup();

  const deleteFirst = await screen.findByRole('button', {
    name: '删除OpenAlex API key pool第 1 个密钥',
  });
  await user.click(deleteFirst);
  expect(screen.getByText(/1 个保存后删除/)).toBeInTheDocument();
  await user.click(
    screen.getByRole('button', { name: '撤销删除OpenAlex API key pool第 1 个密钥' }),
  );
  expect(screen.queryByText(/1 个保存后删除/)).not.toBeInTheDocument();
  await user.click(screen.getByRole('button', { name: '删除OpenAlex API key pool第 1 个密钥' }));
  await user.type(
    screen.getByLabelText('OpenAlex API key pool 新密钥 1'),
    'new-secret-key; new-secret-key',
  );
  await user.click(screen.getByRole('button', { name: '保存配置' }));

  await waitFor(() =>
    expect(updatePayload).toEqual({
      values: {},
      secret_pool_updates: {
        openalex_api_key_pool: {
          add: ['new-secret-key'],
          remove: ['reference-one'],
        },
      },
    }),
  );
}

/**
 * Verify explicit clear sends JSON null without redundant item removals.
 */
async function clearsSecretPoolWithNull(): Promise<void> {
  updatePayload = null;
  renderRuntimeSettings();
  const user = userEvent.setup();

  await user.click(await screen.findByRole('button', { name: '清除全部密钥' }));
  expect(screen.getByText(/保存后清除全部/)).toBeInTheDocument();
  expect(screen.getByLabelText('OpenAlex API key pool 新密钥 1')).toBeDisabled();
  await user.click(screen.getByRole('button', { name: '保存配置' }));

  await waitFor(() =>
    expect(updatePayload).toEqual({
      values: { openalex_api_key_pool: null },
      secret_pool_updates: {},
    }),
  );
}

/**
 * Verify the non-secret Crossref pool keeps its existing plaintext row editor.
 */
async function editsNonSecretPool(): Promise<void> {
  updatePayload = null;
  renderRuntimeSettings();
  const user = userEvent.setup();

  expect(await screen.findByDisplayValue('first@example.test')).toBeInTheDocument();
  expect(screen.getByDisplayValue('second@example.test')).toBeInTheDocument();
  await user.click(screen.getByRole('button', { name: '删除Crossref mailto pool第 1 行' }));
  await user.click(screen.getByRole('button', { name: '保存配置' }));

  await waitFor(() =>
    expect(updatePayload).toEqual({
      values: { crossref_mailto_pool: 'second@example.test' },
      secret_pool_updates: {},
    }),
  );
}

describe('runtime secret settings', () => {
  test('renders and preserves stored secret rows', rendersAndPreservesStoredSecrets);
  test('adds and removes individual stored keys', updatesStoredSecretPool);
  test('clears the complete secret pool only through null', clearsSecretPoolWithNull);
  test('keeps the non-secret pool row editor', editsNonSecretPool);
});
