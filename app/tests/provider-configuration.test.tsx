/**
 * Capability-aware Provider configuration component coverage.
 */

import { fireEvent, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { http, HttpResponse } from 'msw';
import { beforeEach, describe, expect, test, vi } from 'vitest';

import { RuntimeSettingsCard } from '@/components/admin/runtime-settings-card';
import type { ProviderCatalogResponse, RuntimeSettingInfo, RuntimeSettingsUpdate } from '@/lib/api';
import { ApiContractError, parseRuntimeSettingList } from '@/lib/api-contract';
import { server } from '@/tests/mocks/server';
import { renderWithQuery } from '@/tests/render';

let updatePayload: RuntimeSettingsUpdate | null = null;

const SCALAR_SECRET_SENTINEL = 'captcha-secret-sentinel';
const PROXY_SECRET_SENTINEL = 'socks5h://proxy-user:ui-proxy-secret-sentinel@proxy.example:1080';

/**
 * Return the four backend-declared Provider runtime settings.
 *
 * @returns Canonical Provider setting descriptors.
 */
function providerSettingsFixture(): RuntimeSettingInfo[] {
  return [
    {
      field: 'provider_proxy_policy',
      label: 'Provider proxy policy',
      description: 'Independently enable the managed proxy for each Provider.',
      group: 'provider_routing',
      control: 'provider_proxy_policy',
      apply_mode: 'restart_required',
      allowed_values: [],
      input_type: 'text',
      is_secret: false,
      value: '{"cnki":true}',
      has_value: true,
      masked_value: '',
      secret_items: [],
      source: 'default',
      updated_at: null,
    },
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
      value: '{"default":["zjlib"],"catalogs":{}}',
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
 * Build the scalar CNKI captcha token descriptor without exposing its value.
 *
 * @param hasValue - Whether the backend reports one configured token.
 * @returns Secret-safe scalar runtime setting metadata.
 */
function scalarSecretSettingFixture(hasValue = true): RuntimeSettingInfo {
  return runtimeSettingFixture({
    field: 'cnki_captcha_token',
    label: 'CNKI captcha solver token',
    description: 'Domestic CNKI captcha solver credential.',
    group: 'source_access',
    control: 'text',
    apply_mode: 'next_command',
    input_type: 'password',
    value: '',
    is_secret: true,
    has_value: hasValue,
    masked_value: hasValue ? '••••' : '',
    secret_items: [],
  });
}

/**
 * Build the scalar Provider proxy URL descriptor without exposing its value.
 *
 * @param hasValue - Whether the backend reports one configured URL.
 * @returns Secret-safe proxy URL metadata.
 */
function providerProxyUrlSettingFixture(hasValue = true): RuntimeSettingInfo {
  return runtimeSettingFixture({
    field: 'provider_proxy_url',
    label: 'Provider proxy URL',
    description: 'Encrypted HTTP, HTTPS, SOCKS5, or SOCKS5h Provider proxy URL.',
    group: 'source_access',
    control: 'text',
    apply_mode: 'restart_required',
    input_type: 'password',
    value: '',
    is_secret: true,
    has_value: hasValue,
    masked_value: hasValue ? '••••' : '',
    secret_items: [],
  });
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
    scalarSecretSettingFixture(),
    providerProxyUrlSettingFixture(),
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
    runtimeSettingFixture({
      field: 'trusted_proxy_cidrs',
      label: 'Trusted proxy CIDRs',
      description: 'Trusted forwarding peers.',
      group: 'server_security',
      control: 'string_list',
      apply_mode: 'restart_required',
      input_type: 'text',
      value: '',
    }),
    runtimeSettingFixture({
      field: 'auth_rate_limit_policy',
      label: 'Authentication rate-limit policy',
      description: 'Strict authentication token-bucket policy.',
      group: 'server_security',
      control: 'text',
      apply_mode: 'restart_required',
      input_type: 'text',
      value: '{"login_ip":{"capacity":30}}',
    }),
    runtimeSettingFixture({
      field: 'audit_retention_days',
      label: 'Security audit retention days',
      description: 'Number of retained security-audit days.',
      group: 'observability',
      control: 'text',
      apply_mode: 'next_request',
      input_type: 'number',
      value: '180',
    }),
    runtimeSettingFixture({
      field: 'delivery_worker_concurrency',
      label: 'Delivery worker concurrency',
      description: 'Maximum supervised delivery child processes.',
      group: 'server_security',
      control: 'text',
      apply_mode: 'restart_required',
      input_type: 'number',
      value: '2',
    }),
    runtimeSettingFixture({
      field: 'ai_allowed_base_urls',
      label: 'AI allowed base URLs',
      description: 'Allowed OpenAI-compatible HTTPS base URLs.',
      group: 'server_security',
      control: 'string_list',
      apply_mode: 'next_request',
      input_type: 'url',
      value: '',
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
        name: 'cnki_oversea',
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
        name: 'zjlib',
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
  let currentRuntimeSettings = runtimeSettings;
  server.use(
    http.get('http://localhost/api/admin/runtime-settings', () =>
      HttpResponse.json(currentRuntimeSettings),
    ),
    http.get('http://localhost/api/admin/provider-catalog', () =>
      HttpResponse.json(providerCatalogFixture()),
    ),
    http.put('http://localhost/api/admin/runtime-settings', async ({ request }) => {
      updatePayload = (await request.json()) as RuntimeSettingsUpdate;
      const values = updatePayload.values;
      currentRuntimeSettings = currentRuntimeSettings.map((setting) => {
        const updatedValue = values[setting.field];
        if (!setting.is_secret) {
          return {
            ...setting,
            value: typeof updatedValue === 'string' ? updatedValue : setting.value,
          };
        }
        if (updatedValue === null) {
          return {
            ...setting,
            value: '',
            has_value: false,
            masked_value: '',
            secret_items: [],
            source: 'database',
            updated_at: 2,
          };
        }
        if (typeof updatedValue === 'string' && updatedValue.trim().length > 0) {
          return {
            ...setting,
            value: '',
            has_value: true,
            masked_value: '••••',
            secret_items: [],
            source: 'database',
            updated_at: 2,
          };
        }
        return setting;
      });
      return HttpResponse.json(currentRuntimeSettings);
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

  expect(runtimeSettings).toHaveLength(20);
  expect(
    await screen.findByText('ccf_computer_journals', {}, { timeout: 5_000 }),
  ).toBeInTheDocument();
  expect(screen.getByText('chinese_journals')).toBeInTheDocument();
  expect(screen.getByText('legacy_journals')).toBeInTheDocument();
  expect(screen.getAllByText('下次请求生效')).toHaveLength(4);
  expect(screen.getAllByText('下次命令生效')).toHaveLength(5);
  expect(screen.getAllByText('重启后生效')).toHaveLength(11);
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
 * Verify local pool rows retain stable identities while values and positions change.
 */
async function preservesRuntimePoolRowIdentity(): Promise<void> {
  renderProviderConfiguration(allRuntimeSettingsFixture());
  const user = userEvent.setup();

  const firstInput = await screen.findByLabelText('MCP allowed hosts 1');
  const secondInput = screen.getByLabelText('MCP allowed hosts 2');
  const firstRow = firstInput.closest('[data-motion-runtime-input-row]');
  const secondRow = secondInput.closest('[data-motion-runtime-input-row]');
  expect(firstRow).not.toBeNull();
  expect(secondRow).not.toBeNull();

  fireEvent.change(firstInput, { target: { value: 'gateway.internal' } });
  expect(
    screen.getByLabelText('MCP allowed hosts 1').closest('[data-motion-runtime-input-row]'),
  ).toBe(firstRow);

  await user.click(screen.getByRole('button', { name: '删除MCP allowed hosts第 1 行' }));
  await waitFor(() => expect(firstRow).not.toBeInTheDocument());
  expect(screen.getByLabelText('MCP allowed hosts 1')).toHaveValue('127.0.0.1');
  expect(
    screen.getByLabelText('MCP allowed hosts 1').closest('[data-motion-runtime-input-row]'),
  ).toBe(secondRow);
}

/**
 * Verify scalar secret metadata rejects unsafe shapes before reaching the component.
 */
function rejectsInvalidScalarSecretMetadata(): void {
  const valid = scalarSecretSettingFixture();
  expect(parseRuntimeSettingList([valid])).toEqual([valid]);

  for (const invalid of [
    { ...valid, input_type: 'text' },
    { ...valid, value: SCALAR_SECRET_SENTINEL },
    { ...valid, masked_value: '' },
    { ...valid, secret_items: [{ reference: 'opaque', masked_value: '*****' }] },
    { ...valid, has_value: false, masked_value: '••••' },
  ]) {
    expect(() => parseRuntimeSettingList([invalid])).toThrow(ApiContractError);
  }
}

/**
 * Verify blank, replacement, and clear updates preserve scalar secret redaction.
 */
async function serializesScalarSecretLifecycle(): Promise<void> {
  renderProviderConfiguration(allRuntimeSettingsFixture());
  const user = userEvent.setup();

  const tokenInput = await screen.findByLabelText('CNKI captcha solver token');
  expect(tokenInput).toHaveAttribute('type', 'password');
  expect(tokenInput).toHaveValue('');
  expect(document.body.textContent).not.toContain(SCALAR_SECRET_SENTINEL);

  const logFilter = screen.getByLabelText('Log filter');
  fireEvent.change(logFilter, { target: { value: 'warn,litradar=debug' } });
  await user.click(screen.getByRole('button', { name: '保存配置' }));
  await waitFor(() =>
    expect(updatePayload).toEqual({
      values: { log_filter: 'warn,litradar=debug' },
      secret_pool_updates: {},
    }),
  );

  fireEvent.change(screen.getByLabelText('CNKI captcha solver token'), {
    target: { value: SCALAR_SECRET_SENTINEL },
  });
  await user.click(screen.getByRole('button', { name: '保存配置' }));
  await waitFor(() =>
    expect(updatePayload).toEqual({
      values: { cnki_captcha_token: SCALAR_SECRET_SENTINEL },
      secret_pool_updates: {},
    }),
  );
  await waitFor(() => expect(screen.getByLabelText('CNKI captcha solver token')).toHaveValue(''));
  expect(document.body.textContent).not.toContain(SCALAR_SECRET_SENTINEL);

  const tokenSetting = document.querySelector('[data-runtime-setting-field="cnki_captcha_token"]');
  expect(tokenSetting).not.toBeNull();
  await user.click(
    within(tokenSetting as HTMLElement).getByRole('button', { name: '清除全部密钥' }),
  );
  await user.click(screen.getByRole('button', { name: '保存配置' }));
  await waitFor(() =>
    expect(updatePayload).toEqual({
      values: { cnki_captcha_token: null },
      secret_pool_updates: {},
    }),
  );
  expect(screen.getByLabelText('CNKI captcha solver token')).toHaveValue('');
  expect(document.body.textContent).not.toContain(SCALAR_SECRET_SENTINEL);
}

/**
 * Verify proxy switches default missing Providers off and save with the URL atomically.
 */
async function serializesProviderProxyUrlAndPolicyAtomically(): Promise<void> {
  renderProviderConfiguration(allRuntimeSettingsFixture());
  const user = userEvent.setup();

  const cnkiSwitch = await screen.findByRole('switch', {
    name: 'cnki 使用 Provider 代理',
  });
  expect(cnkiSwitch).toBeChecked();
  expect(screen.getByRole('switch', { name: 'cnki_oversea 使用 Provider 代理' })).not.toBeChecked();
  expect(screen.getByRole('switch', { name: 'scholarly 使用 Provider 代理' })).not.toBeChecked();
  expect(screen.getByRole('switch', { name: 'zjlib 使用 Provider 代理' })).not.toBeChecked();

  const cnkiGroup = screen.getByRole('group', { name: 'cnki' });
  expect(within(cnkiGroup).getByText('索引')).toBeInTheDocument();
  expect(within(cnkiGroup).getByText('摘要页')).toBeInTheDocument();
  const zjlibGroup = screen.getByRole('group', { name: 'zjlib' });
  expect(within(zjlibGroup).getByText('全文')).toBeInTheDocument();
  expect(within(zjlibGroup).queryByText('索引')).not.toBeInTheDocument();

  const proxyUrl = screen.getByLabelText('Provider proxy URL');
  expect(proxyUrl).toHaveAttribute('type', 'password');
  expect(proxyUrl).toHaveValue('');
  fireEvent.change(proxyUrl, { target: { value: PROXY_SECRET_SENTINEL } });
  await user.click(screen.getByRole('switch', { name: 'zjlib 使用 Provider 代理' }));
  await user.click(screen.getByRole('button', { name: '保存配置' }));

  await waitFor(() =>
    expect(updatePayload).toEqual({
      values: {
        provider_proxy_url: PROXY_SECRET_SENTINEL,
        provider_proxy_policy: '{"cnki":true,"cnki_oversea":false,"scholarly":false,"zjlib":true}',
      },
      secret_pool_updates: {},
    }),
  );
  await waitFor(() => expect(screen.getByLabelText('Provider proxy URL')).toHaveValue(''));
  expect(document.body.textContent).not.toContain(PROXY_SECRET_SENTINEL);
}

/**
 * Verify an empty policy defaults every switch off and edits remain independent.
 */
async function defaultsProviderProxySwitchesOffIndependently(): Promise<void> {
  const settings = providerSettingsFixture().map((setting) =>
    setting.field === 'provider_proxy_policy' ? { ...setting, value: '{}' } : setting,
  );
  renderProviderConfiguration(settings);
  const user = userEvent.setup();

  const switches = await Promise.all(
    ['cnki', 'cnki_oversea', 'scholarly', 'zjlib'].map((provider) =>
      screen.findByRole('switch', { name: `${provider} 使用 Provider 代理` }),
    ),
  );
  for (const providerSwitch of switches) {
    expect(providerSwitch).not.toBeChecked();
  }

  await user.click(switches[0]);
  expect(switches[0]).toBeChecked();
  for (const providerSwitch of switches.slice(1)) {
    expect(providerSwitch).not.toBeChecked();
  }
}

/**
 * Verify policy names absent from the current Provider catalog stop specialized editing.
 */
async function rejectsProviderProxyPolicyCatalogMismatch(): Promise<void> {
  const settings = providerSettingsFixture().map((setting) =>
    setting.field === 'provider_proxy_policy'
      ? { ...setting, value: '{"retired_provider":true}' }
      : setting,
  );
  renderProviderConfiguration(settings);

  expect(
    await screen.findByText('Provider 配置与后端能力目录不一致，已停止编辑以避免覆盖有效配置。'),
  ).toBeInTheDocument();
  expect(screen.queryByRole('switch', { name: 'cnki 使用 Provider 代理' })).not.toBeInTheDocument();
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
  expect(screen.getByRole('option', { name: 'cnki_oversea' })).toBeInTheDocument();
  expect(screen.getByRole('option', { name: 'scholarly' })).toBeInTheDocument();
  expect(screen.queryByRole('option', { name: 'zjlib' })).not.toBeInTheDocument();
  await user.keyboard('{Escape}');

  const fulltextSelect = screen.getByRole('combobox', {
    name: '默认全文 Provider 顺序第 1 项',
  });
  fulltextSelect.focus();
  await user.keyboard('{Enter}');
  expect(screen.getByRole('option', { name: 'zjlib' })).toBeInTheDocument();
  expect(screen.queryByRole('option', { name: 'cnki_oversea' })).not.toBeInTheDocument();
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
    20_000,
  );
  test('preserves stable runtime pool row identities', preservesRuntimePoolRowIdentity, 10_000);
  test(
    'filters every Provider selector by declared capability',
    filtersProviderCandidatesByCapability,
  );
  test('serializes reordered default Provider candidates', serializesReorderedDefaultProviderOrder);
  test(
    'distinguishes inherited and explicitly disabled catalog orders',
    serializesInheritanceAndExplicitDisable,
    20_000,
  );
  test('serializes one index Provider per catalog', serializesOneIndexProviderPerCatalog);
  test('rejects unsafe scalar secret metadata', rejectsInvalidScalarSecretMetadata);
  test(
    'preserves, replaces, and clears a scalar secret safely',
    serializesScalarSecretLifecycle,
    20_000,
  );
  test(
    'saves the Provider proxy URL and deterministic policy atomically',
    serializesProviderProxyUrlAndPolicyAtomically,
    20_000,
  );
  test(
    'defaults Provider proxy switches off and edits them independently',
    defaultsProviderProxySwitchesOffIndependently,
  );
  test(
    'rejects Provider proxy names missing from the capability catalog',
    rejectsProviderProxyPolicyCatalogMismatch,
  );
  test('renders one safe fallback for a future backend control', rendersFutureGenericControlOnce);
});
