/**
 * Capability-aware Provider configuration component coverage.
 */

import { screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { http, HttpResponse } from 'msw';
import { beforeEach, describe, expect, test, vi } from 'vitest';

import { RuntimeSettingsCard } from '@/components/admin/runtime-settings-card';
import type { ProviderCatalogResponse, RuntimeSettingInfo, RuntimeSettingsUpdate } from '@/lib/api';
import { server } from '@/tests/mocks/server';
import { renderWithQuery } from '@/tests/render';

let updatePayload: RuntimeSettingsUpdate | null = null;

/**
 * Return the three backend-declared Provider runtime settings.
 *
 * @returns Canonical Provider setting descriptors.
 */
function providerSettingsFixture(): RuntimeSettingInfo[] {
  return [
    {
      field: 'index_provider_routes',
      label: '索引 Provider',
      description: '每个目录使用一个索引 Provider。',
      group: 'provider_routing',
      control: 'index_provider_routes',
      apply_mode: 'next_command',
      allowed_values: [],
      input_type: 'text',
      is_secret: false,
      value:
        '{"ccf_computer_journals":"scholarly","chinese_journals":"cnki","legacy_journals":"scholarly"}',
      has_value: true,
      masked_value: '',
      secret_items: [],
      source: 'default',
      updated_at: null,
    },
    {
      field: 'article_abstract_provider_orders',
      label: '摘要页 Provider 顺序',
      description: '按顺序解析在线摘要页。',
      group: 'provider_routing',
      control: 'provider_order',
      apply_mode: 'next_request',
      allowed_values: [],
      input_type: 'text',
      is_secret: false,
      value:
        '{"default":["scholarly","cnki"],"catalogs":{"chinese_journals":["cnki","scholarly"],"legacy_journals":[]}}',
      has_value: true,
      masked_value: '',
      secret_items: [],
      source: 'database',
      updated_at: 1,
    },
    {
      field: 'article_fulltext_provider_orders',
      label: '全文 Provider 顺序',
      description: '按顺序解析在线全文。',
      group: 'provider_routing',
      control: 'provider_order',
      apply_mode: 'next_request',
      allowed_values: [],
      input_type: 'text',
      is_secret: false,
      value: '{"default":["zjlib_cnki"],"catalogs":{}}',
      has_value: true,
      masked_value: '',
      secret_items: [],
      source: 'default',
      updated_at: null,
    },
  ];
}

/**
 * Build one non-secret runtime descriptor from common safe defaults.
 *
 * @param overrides - Field-specific descriptor values.
 * @returns Complete runtime setting descriptor.
 */
function runtimeSettingFixture(
  overrides: Partial<RuntimeSettingInfo> &
    Pick<
      RuntimeSettingInfo,
      | 'field'
      | 'label'
      | 'description'
      | 'group'
      | 'control'
      | 'apply_mode'
      | 'input_type'
      | 'value'
    >,
): RuntimeSettingInfo {
  return {
    allowed_values: [],
    is_secret: false,
    has_value: overrides.value.length > 0,
    masked_value: '',
    secret_items: [],
    source: 'default',
    updated_at: null,
    ...overrides,
  };
}

/**
 * Return every runtime descriptor currently declared by the backend.
 *
 * @returns Complete metadata parity fixture.
 */
function allRuntimeSettingsFixture(): RuntimeSettingInfo[] {
  return [
    runtimeSettingFixture({
      field: 'openalex_api_key_pool',
      label: 'OpenAlex API key pool',
      description: 'OpenAlex authenticated request key pool.',
      group: 'source_access',
      control: 'secret_pool',
      apply_mode: 'next_command',
      input_type: 'password',
      value: '',
      is_secret: true,
    }),
    runtimeSettingFixture({
      field: 'semantic_scholar_api_key_pool',
      label: 'Semantic Scholar API key pool',
      description: 'Semantic Scholar authenticated request key pool.',
      group: 'source_access',
      control: 'secret_pool',
      apply_mode: 'next_command',
      input_type: 'password',
      value: '',
      is_secret: true,
    }),
    runtimeSettingFixture({
      field: 'crossref_mailto_pool',
      label: 'Crossref mailto pool',
      description: 'Crossref request identity pool.',
      group: 'source_access',
      control: 'string_list',
      apply_mode: 'next_command',
      input_type: 'email',
      value: '',
    }),
    runtimeSettingFixture({
      field: 'cors_allowed_origins',
      label: 'CORS allowed origins',
      description: 'Credentialed API origins.',
      group: 'server_security',
      control: 'string_list',
      apply_mode: 'restart_required',
      input_type: 'text',
      value: '',
    }),
    runtimeSettingFixture({
      field: 'mcp_allowed_hosts',
      label: 'MCP allowed hosts',
      description: 'Accepted MCP hosts.',
      group: 'server_security',
      control: 'string_list',
      apply_mode: 'restart_required',
      input_type: 'text',
      value: 'localhost,127.0.0.1,::1',
    }),
    runtimeSettingFixture({
      field: 'mcp_allowed_origins',
      label: 'MCP allowed origins',
      description: 'Accepted MCP origins.',
      group: 'server_security',
      control: 'string_list',
      apply_mode: 'restart_required',
      input_type: 'text',
      value: '',
    }),
    runtimeSettingFixture({
      field: 'secure_cookies',
      label: 'Secure session cookies',
      description: 'Use the Secure cookie attribute.',
      group: 'server_security',
      control: 'boolean',
      apply_mode: 'restart_required',
      allowed_values: ['true', 'false'],
      input_type: 'boolean',
      value: 'false',
    }),
    ...providerSettingsFixture(),
    runtimeSettingFixture({
      field: 'log_format',
      label: 'Log format',
      description: 'Structured process log output format.',
      group: 'observability',
      control: 'select',
      apply_mode: 'restart_required',
      allowed_values: ['json', 'compact'],
      input_type: 'text',
      value: 'json',
    }),
    runtimeSettingFixture({
      field: 'log_filter',
      label: 'Log filter',
      description: 'Tracing filter directives.',
      group: 'observability',
      control: 'text',
      apply_mode: 'restart_required',
      input_type: 'text',
      value: 'warn,litradar=info',
    }),
  ];
}

/**
 * Return capabilities and paired, CSV-only, and database-only catalogs.
 *
 * @returns Safe Provider catalog metadata.
 */
function providerCatalogFixture(): ProviderCatalogResponse {
  return {
    providers: [
      {
        name: 'cnki',
        index_content: true,
        article_abstract: true,
        article_full_text: false,
      },
      {
        name: 'scholarly',
        index_content: true,
        article_abstract: true,
        article_full_text: false,
      },
      {
        name: 'zjlib_cnki',
        index_content: false,
        article_abstract: false,
        article_full_text: true,
      },
    ],
    catalogs: [
      {
        stem: 'ccf_computer_journals',
        csv_filename: 'ccf_computer_journals.csv',
        database_filename: 'ccf_computer_journals.sqlite',
      },
      {
        stem: 'chinese_journals',
        csv_filename: 'chinese_journals.csv',
        database_filename: null,
      },
      {
        stem: 'legacy_journals',
        csv_filename: null,
        database_filename: 'legacy_journals.sqlite',
      },
    ],
  };
}

/**
 * Install Provider catalog and atomic runtime update handlers.
 */
function renderProviderConfiguration(
  runtimeSettings: RuntimeSettingInfo[] = providerSettingsFixture(),
): void {
  server.use(
    http.get('http://localhost/api/admin/runtime-settings', () =>
      HttpResponse.json(runtimeSettings),
    ),
    http.get('http://localhost/api/admin/provider-catalog', () =>
      HttpResponse.json(providerCatalogFixture()),
    ),
    http.put('http://localhost/api/admin/runtime-settings', async ({ request }) => {
      updatePayload = (await request.json()) as RuntimeSettingsUpdate;
      const values = updatePayload.values;
      return HttpResponse.json(
        runtimeSettings.map((setting) => ({
          ...setting,
          value: typeof values[setting.field] === 'string' ? values[setting.field] : setting.value,
        })),
      );
    }),
  );
  renderWithQuery(<RuntimeSettingsCard />);
}

/**
 * Verify every runtime descriptor is represented once without raw Provider JSON controls.
 */
async function rendersRuntimeDescriptorParityAndCatalogMatrix(): Promise<void> {
  const runtimeSettings = allRuntimeSettingsFixture();
  renderProviderConfiguration(runtimeSettings);

  expect(await screen.findByText('ccf_computer_journals')).toBeInTheDocument();
  expect(screen.getByText('chinese_journals')).toBeInTheDocument();
  expect(screen.getByText('legacy_journals')).toBeInTheDocument();
  expect(screen.getAllByText('下次请求生效')).toHaveLength(2);
  expect(screen.getAllByText('下次命令生效')).toHaveLength(4);
  expect(screen.getAllByText('重启后生效')).toHaveLength(6);
  expect(document.querySelectorAll('[data-runtime-setting-field]')).toHaveLength(
    runtimeSettings.length,
  );
  for (const setting of runtimeSettings) {
    expect(
      document.querySelectorAll(`[data-runtime-setting-field="${setting.field}"]`),
    ).toHaveLength(1);
  }
  expect(document.body.textContent).not.toContain('{"default"');
  expect(screen.getAllByText('CSV 已发现')).toHaveLength(2);
  expect(screen.getAllByText('数据库已发现')).toHaveLength(2);
}

/**
 * Verify index and online selectors offer only capability-compatible Providers.
 */
async function filtersProviderCandidatesByCapability(): Promise<void> {
  renderProviderConfiguration();
  const user = userEvent.setup();

  const indexSelect = await screen.findByRole('combobox', {
    name: 'ccf_computer_journals 索引 Provider',
  });
  indexSelect.focus();
  await user.keyboard('{Enter}');
  expect(screen.getByRole('option', { name: 'cnki' })).toBeInTheDocument();
  expect(screen.getByRole('option', { name: 'scholarly' })).toBeInTheDocument();
  expect(screen.queryByRole('option', { name: 'zjlib_cnki' })).not.toBeInTheDocument();
  await user.keyboard('{Escape}');

  const fulltextSelect = screen.getByRole('combobox', {
    name: '默认全文 Provider 顺序第 1 项',
  });
  fulltextSelect.focus();
  await user.keyboard('{Enter}');
  expect(screen.getByRole('option', { name: 'zjlib_cnki' })).toBeInTheDocument();
  expect(screen.queryByRole('option', { name: 'cnki' })).not.toBeInTheDocument();
  expect(screen.queryByRole('option', { name: 'scholarly' })).not.toBeInTheDocument();
}

/**
 * Verify default sequence reordering preserves exact Provider order in one PUT.
 */
async function serializesReorderedDefaultProviderOrder(): Promise<void> {
  renderProviderConfiguration();
  const user = userEvent.setup();

  await user.click(
    await screen.findByRole('button', {
      name: '下移默认摘要页 Provider 顺序第 1 项',
    }),
  );
  await user.click(screen.getByRole('button', { name: '保存配置' }));

  await waitFor(() =>
    expect(updatePayload).toEqual({
      values: {
        article_abstract_provider_orders:
          '{"default":["cnki","scholarly"],"catalogs":{"chinese_journals":["cnki","scholarly"],"legacy_journals":[]}}',
      },
      secret_pool_updates: {},
    }),
  );
  expect(await screen.findByRole('status')).toHaveTextContent('运行配置已保存。');
}

/**
 * Verify inheritance removal and explicit empty override remain distinguishable.
 */
async function serializesInheritanceAndExplicitDisable(): Promise<void> {
  renderProviderConfiguration();
  const user = userEvent.setup();

  await user.click(
    await screen.findByRole('switch', {
      name: 'ccf_computer_journals-abstract继承默认顺序',
    }),
  );
  await user.click(
    screen.getByRole('switch', {
      name: 'ccf_computer_journals-abstract禁用摘要页',
    }),
  );
  await user.click(
    screen.getByRole('switch', {
      name: 'legacy_journals-abstract继承默认顺序',
    }),
  );
  await user.click(screen.getByRole('button', { name: '保存配置' }));

  await waitFor(() =>
    expect(updatePayload).toEqual({
      values: {
        article_abstract_provider_orders:
          '{"default":["scholarly","cnki"],"catalogs":{"ccf_computer_journals":[],"chinese_journals":["cnki","scholarly"]}}',
      },
      secret_pool_updates: {},
    }),
  );
}

/**
 * Verify changing one index selection sends a sorted single-choice route map.
 */
async function serializesOneIndexProviderPerCatalog(): Promise<void> {
  renderProviderConfiguration();
  const user = userEvent.setup();

  const select = await screen.findByRole('combobox', {
    name: 'ccf_computer_journals 索引 Provider',
  });
  select.focus();
  await user.keyboard('{Enter}{Home}{Enter}');
  await waitFor(() => expect(select).toHaveTextContent('cnki'));
  await user.click(screen.getByRole('button', { name: '保存配置' }));

  await waitFor(() =>
    expect(updatePayload).toEqual({
      values: {
        index_provider_routes:
          '{"ccf_computer_journals":"cnki","chinese_journals":"cnki","legacy_journals":"scholarly"}',
      },
      secret_pool_updates: {},
    }),
  );
}

/**
 * Verify a future safe control falls back to one labelled text input.
 */
async function rendersFutureGenericControlOnce(): Promise<void> {
  server.use(
    http.get('http://localhost/api/admin/runtime-settings', () =>
      HttpResponse.json([
        {
          field: 'future_setting',
          label: 'Future setting',
          description: 'Backend-declared future setting.',
          group: 'observability',
          control: 'future_text',
          apply_mode: 'next_command',
          allowed_values: [],
          input_type: 'text',
          is_secret: false,
          value: 'future-value',
          has_value: true,
          masked_value: '',
          secret_items: [],
          source: 'default',
          updated_at: null,
        },
      ]),
    ),
  );
  renderWithQuery(<RuntimeSettingsCard />);

  expect(await screen.findByLabelText('Future setting')).toHaveValue('future-value');
  expect(document.querySelectorAll('[data-runtime-setting-field="future_setting"]')).toHaveLength(
    1,
  );
  expect(screen.getByText('下次命令生效')).toBeInTheDocument();
}

beforeEach(() => {
  updatePayload = null;
  Object.defineProperty(Element.prototype, 'scrollIntoView', {
    configurable: true,
    value: vi.fn(),
  });
});

describe('Provider configuration', () => {
  test(
    'renders every runtime descriptor once with the Provider catalog matrix',
    rendersRuntimeDescriptorParityAndCatalogMatrix,
  );
  test(
    'filters every Provider selector by declared capability',
    filtersProviderCandidatesByCapability,
  );
  test('serializes reordered default Provider candidates', serializesReorderedDefaultProviderOrder);
  test(
    'distinguishes inherited and explicitly disabled catalog orders',
    serializesInheritanceAndExplicitDisable,
  );
  test('serializes one index Provider per catalog', serializesOneIndexProviderPerCatalog);
  test('renders one safe fallback for a future backend control', rendersFutureGenericControlOnce);
});
