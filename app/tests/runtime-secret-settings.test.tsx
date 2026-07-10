/**
 * Runtime credential preserve and explicit-clear component coverage.
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
 * Return one configured secret runtime setting.
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
 * Verify an untouched blank password control preserves the stored credential.
 */
async function preservesBlankSecret(): Promise<void> {
  updatePayload = null;
  renderRuntimeSettings();
  const user = userEvent.setup();

  expect(await screen.findByText(/已安全保存，留空会保留/)).toBeInTheDocument();
  await user.click(screen.getByRole('button', { name: '保存配置' }));

  await waitFor(() => expect(updatePayload).toEqual({ values: { openalex_api_key_pool: '' } }));
}

/**
 * Verify explicit clear sends JSON null instead of an empty password value.
 */
async function clearsSecretWithNull(): Promise<void> {
  updatePayload = null;
  renderRuntimeSettings();
  const user = userEvent.setup();

  await user.click(await screen.findByRole('button', { name: '清除密钥' }));
  expect(screen.getByText(/保存后清除/)).toBeInTheDocument();
  await user.click(screen.getByRole('button', { name: '保存配置' }));

  await waitFor(() => expect(updatePayload).toEqual({ values: { openalex_api_key_pool: null } }));
}

describe('runtime secret settings', () => {
  test('preserves a configured secret when the blank input is untouched', preservesBlankSecret);
  test('clears a configured secret only through an explicit null update', clearsSecretWithNull);
});
