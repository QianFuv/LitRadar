/**
 * Capability-aware default and per-catalog Provider configuration controls.
 */

'use client';

import { useMemo } from 'react';
import { ArrowDown, ArrowUp, Database, Plus, Trash2 } from 'lucide-react';

import {
  type IndexProviderRoutes,
  type ProviderCapabilityInfo,
  type ProviderCatalogInfo,
  type ProviderCatalogResponse,
  type ProviderOrderConfiguration,
  type RuntimeSettingApplyMode,
  type RuntimeSettingInfo,
} from '@/lib/api';
import {
  parseIndexProviderRoutes,
  parseProviderOrderConfiguration,
  parseProviderProxyPolicy,
  type ProviderProxyPolicy,
} from '@/lib/api-contract';
import {
  SettingsGroup,
  SettingsRow,
  SettingsSection,
  SettingsSectionDescription,
  SettingsSectionHeader,
  SettingsSectionTitle,
  StatusPill,
} from '@/components/settings/settings-section';
import { Button } from '@/components/ui/button';
import { Label } from '@/components/ui/label';
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@/components/ui/select';
import { Switch } from '@/components/ui/switch';

const PROVIDER_PROXY_POLICY_FIELD = 'provider_proxy_policy';
const INDEX_PROVIDER_FIELD = 'index_provider_routes';
const ABSTRACT_PROVIDER_FIELD = 'article_abstract_provider_orders';
const FULLTEXT_PROVIDER_FIELD = 'article_fulltext_provider_orders';

type ProviderConfigurationEditorProps = {
  settings: RuntimeSettingInfo[];
  values: Record<string, string>;
  catalog: ProviderCatalogResponse;
  onChange: (field: string, value: string) => void;
};

type ParsedProviderConfiguration = {
  proxySetting: RuntimeSettingInfo;
  indexSetting: RuntimeSettingInfo;
  abstractSetting: RuntimeSettingInfo;
  fulltextSetting: RuntimeSettingInfo;
  proxyPolicy: ProviderProxyPolicy;
  indexRoutes: IndexProviderRoutes;
  abstractOrders: ProviderOrderConfiguration;
  fulltextOrders: ProviderOrderConfiguration;
};

type ProviderOrderEditorProps = {
  label: string;
  providers: ProviderCapabilityInfo[];
  order: string[];
  onChange: (order: string[]) => void;
};

type DefaultProviderOrderProps = ProviderOrderEditorProps & {
  capabilityLabel: string;
};

type CatalogProviderOrderProps = ProviderOrderEditorProps & {
  capabilityLabel: string;
  defaultOrder: string[];
  isInherited: boolean;
  onInheritanceChange: (isInherited: boolean) => void;
};

/**
 * Render the administrator-facing label for a setting apply mode.
 *
 * @param applyMode - Backend-declared lifecycle point.
 * @returns Concise Chinese apply-mode label.
 */
function getApplyModeLabel(applyMode: RuntimeSettingApplyMode): string {
  if (applyMode === 'next_request') {
    return '下次请求生效';
  }
  if (applyMode === 'next_command') {
    return '下次命令生效';
  }
  return '重启后生效';
}

/**
 * Serialize a string map with deterministic catalog ordering.
 *
 * @param values - Catalog values.
 * @returns Canonical JSON text with sorted keys.
 */
function serializeIndexRoutes(values: IndexProviderRoutes): string {
  return JSON.stringify(
    Object.fromEntries(Object.entries(values).sort(([left], [right]) => left.localeCompare(right))),
  );
}

/**
 * Serialize one explicit boolean decision for every current Provider.
 *
 * @param providers - Current capability catalog.
 * @param values - Parsed and edited proxy decisions.
 * @returns Canonical JSON text with every Provider name sorted.
 */
function serializeProviderProxyPolicy(
  providers: ProviderCapabilityInfo[],
  values: ProviderProxyPolicy,
): string {
  return JSON.stringify(
    Object.fromEntries(
      [...providers]
        .sort((left, right) => left.name.localeCompare(right.name))
        .map((provider) => [provider.name, values[provider.name] === true]),
    ),
  );
}

/**
 * Serialize Provider orders with deterministic catalog ordering and preserved arrays.
 *
 * @param configuration - Default and per-catalog Provider orders.
 * @returns Canonical Provider-order JSON text.
 */
function serializeProviderOrders(configuration: ProviderOrderConfiguration): string {
  return JSON.stringify({
    default: [...configuration.default],
    catalogs: Object.fromEntries(
      Object.entries(configuration.catalogs)
        .sort(([left], [right]) => left.localeCompare(right))
        .map(([stem, order]) => [stem, [...order]]),
    ),
  });
}

/**
 * Return the current form value for one runtime setting.
 *
 * @param setting - Runtime descriptor.
 * @param values - Effective form values.
 * @returns Current JSON setting value.
 */
function getSettingValue(setting: RuntimeSettingInfo, values: Record<string, string>): string {
  return values[setting.field] ?? setting.value;
}

/**
 * Validate that an ordered Provider list advertises the requested capability.
 *
 * @param providers - Provider metadata keyed by name.
 * @param orders - Default and catalog override orders.
 * @param capability - Capability flag required by the setting.
 * @returns Whether every configured Provider is known and capable.
 */
function hasCapableProviderOrders(
  providers: Map<string, ProviderCapabilityInfo>,
  orders: ProviderOrderConfiguration,
  capability: 'article_abstract' | 'article_full_text',
): boolean {
  return [orders.default, ...Object.values(orders.catalogs)].every((order) =>
    order.every((name) => providers.get(name)?.[capability] === true),
  );
}

/**
 * Parse and capability-check the four grouped Provider settings.
 *
 * @param settings - Provider runtime descriptors.
 * @param values - Effective form values.
 * @param catalog - Provider capability metadata.
 * @returns Parsed configuration or null when the contracts disagree.
 */
function parseProviderConfiguration(
  settings: RuntimeSettingInfo[],
  values: Record<string, string>,
  catalog: ProviderCatalogResponse,
): ParsedProviderConfiguration | null {
  const proxySetting = settings.find((setting) => setting.field === PROVIDER_PROXY_POLICY_FIELD);
  const indexSetting = settings.find((setting) => setting.field === INDEX_PROVIDER_FIELD);
  const abstractSetting = settings.find((setting) => setting.field === ABSTRACT_PROVIDER_FIELD);
  const fulltextSetting = settings.find((setting) => setting.field === FULLTEXT_PROVIDER_FIELD);
  if (
    !proxySetting ||
    !indexSetting ||
    !abstractSetting ||
    !fulltextSetting ||
    !hasProviderSettingControls(proxySetting, indexSetting, abstractSetting, fulltextSetting)
  ) {
    return null;
  }

  try {
    const proxyPolicy = parseProviderProxyPolicy(getSettingValue(proxySetting, values));
    const indexRoutes = parseIndexProviderRoutes(getSettingValue(indexSetting, values));
    const abstractOrders = parseProviderOrderConfiguration(
      getSettingValue(abstractSetting, values),
    );
    const fulltextOrders = parseProviderOrderConfiguration(
      getSettingValue(fulltextSetting, values),
    );
    const providers = new Map(catalog.providers.map((provider) => [provider.name, provider]));
    const hasKnownProxyProviders = Object.keys(proxyPolicy).every((name) => providers.has(name));
    const hasCapableIndexRoutes = Object.values(indexRoutes).every(
      (name) => providers.get(name)?.index_content === true,
    );
    if (
      !hasProviderConfigurationCapabilities(
        hasKnownProxyProviders,
        hasCapableIndexRoutes,
        providers,
        abstractOrders,
        fulltextOrders,
      )
    ) {
      return null;
    }
    return {
      proxySetting,
      indexSetting,
      abstractSetting,
      fulltextSetting,
      proxyPolicy,
      indexRoutes,
      abstractOrders,
      fulltextOrders,
    };
  } catch {
    return null;
  }
}

/**
 * Render the capabilities associated with one Provider proxy switch.
 *
 * @param provider - Provider capability descriptor.
 * @returns Capability badges for the Provider row.
 */
function ProviderCapabilityBadges({ provider }: { provider: ProviderCapabilityInfo }) {
  const capabilities = [
    provider.index_content ? '索引' : null,
    provider.article_abstract ? '摘要页' : null,
    provider.article_full_text ? '全文' : null,
  ].filter((capability): capability is string => capability !== null);

  return (
    <span className="mt-1 flex flex-wrap items-center gap-1">
      {capabilities.map((capability) => (
        <span
          key={capability}
          className="rounded-md bg-muted px-1.5 py-0.5 text-[11px] text-muted-foreground"
        >
          {capability}
        </span>
      ))}
    </span>
  );
}

/**
 * Move one ordered Provider without changing the remaining order.
 *
 * @param order - Current Provider order.
 * @param index - Provider index to move.
 * @param offset - Negative or positive adjacent offset.
 * @returns Updated Provider order.
 */
function moveProvider(order: string[], index: number, offset: -1 | 1): string[] {
  const targetIndex = index + offset;
  if (targetIndex < 0 || targetIndex >= order.length) {
    return order;
  }
  const nextOrder = [...order];
  [nextOrder[index], nextOrder[targetIndex]] = [nextOrder[targetIndex], nextOrder[index]];
  return nextOrder;
}

/**
 * Render accessible ordered Provider selection controls.
 *
 * @param props - Editor label, candidates, value, and update callback.
 * @returns Ordered Provider editor.
 */
function ProviderOrderEditor({ label, providers, order, onChange }: ProviderOrderEditorProps) {
  const availableProviders = providers.filter((provider) => !order.includes(provider.name));

  return (
    <ol className="space-y-2">
      {order.map((providerName, index) => {
        const selectableProviders = providers.filter(
          (provider) => provider.name === providerName || !order.includes(provider.name),
        );
        return (
          <li key={providerName} className="flex items-center gap-2">
            <span
              aria-hidden="true"
              className="flex size-6 shrink-0 items-center justify-center rounded-full bg-foreground text-[11px] font-bold text-background tabular-nums"
            >
              {index + 1}
            </span>
            <Select
              value={providerName}
              onValueChange={(nextProvider) => {
                const nextOrder = [...order];
                nextOrder[index] = nextProvider;
                onChange(nextOrder);
              }}
            >
              <SelectTrigger className="min-w-0 flex-1" aria-label={`${label}第 ${index + 1} 项`}>
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {selectableProviders.map((provider) => (
                  <SelectItem key={provider.name} value={provider.name}>
                    {provider.name}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
            <div className="flex shrink-0 items-center">
              <Button
                type="button"
                variant="ghost"
                size="icon-sm"
                disabled={index === 0}
                aria-label={`上移${label}第 ${index + 1} 项`}
                onClick={() => onChange(moveProvider(order, index, -1))}
              >
                <ArrowUp className="h-4 w-4" />
              </Button>
              <Button
                type="button"
                variant="ghost"
                size="icon-sm"
                disabled={index === order.length - 1}
                aria-label={`下移${label}第 ${index + 1} 项`}
                onClick={() => onChange(moveProvider(order, index, 1))}
              >
                <ArrowDown className="h-4 w-4" />
              </Button>
              <Button
                type="button"
                variant="ghost"
                size="icon-sm"
                className="text-muted-foreground hover:bg-destructive/10 hover:text-destructive"
                aria-label={`删除${label}第 ${index + 1} 项`}
                onClick={() => onChange(order.filter((_, itemIndex) => itemIndex !== index))}
              >
                <Trash2 className="h-4 w-4" />
              </Button>
            </div>
          </li>
        );
      })}
      <li>
        <Button
          type="button"
          variant="ghost"
          size="sm"
          className="text-primary hover:text-primary"
          disabled={availableProviders.length === 0}
          onClick={() => onChange([...order, availableProviders[0].name])}
        >
          <Plus className="size-4" />
          添加 Provider
        </Button>
      </li>
    </ol>
  );
}

/**
 * Render a default order with an explicit empty-disable switch.
 *
 * @param props - Default order editor props.
 * @returns Default capability order controls.
 */
function DefaultProviderOrder({
  capabilityLabel,
  label,
  providers,
  order,
  onChange,
}: DefaultProviderOrderProps) {
  const isDisabled = order.length === 0;

  return (
    <div className="space-y-3 px-4 py-4 sm:px-5">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <Label className="font-bold" htmlFor={`default-${capabilityLabel}-disabled`}>
          {label}
        </Label>
        <div className="flex items-center gap-2">
          <span className="text-xs text-muted-foreground">默认禁用</span>
          <Switch
            id={`default-${capabilityLabel}-disabled`}
            aria-label={`默认禁用${capabilityLabel}`}
            checked={isDisabled}
            disabled={providers.length === 0}
            onCheckedChange={(checked: boolean) => {
              if (checked) {
                onChange([]);
              } else {
                onChange(providers.length > 0 ? [providers[0].name] : []);
              }
            }}
          />
        </div>
      </div>
      {isDisabled ? (
        <p className="rounded-lg bg-muted/60 px-3 py-2 text-xs text-muted-foreground">
          所有未覆盖目录均不提供{capabilityLabel}。
        </p>
      ) : (
        <ProviderOrderEditor
          label={label}
          providers={providers}
          order={order}
          onChange={onChange}
        />
      )}
    </div>
  );
}

/**
 * Render one catalog override with distinct inherit and explicit-disable states.
 *
 * @param props - Catalog override editor props.
 * @returns Catalog-specific Provider order controls.
 */
function CatalogProviderOrder({
  capabilityLabel,
  label,
  providers,
  order,
  defaultOrder,
  isInherited,
  onChange,
  onInheritanceChange,
}: CatalogProviderOrderProps) {
  const isDisabled = !isInherited && order.length === 0;

  return (
    <div className="space-y-3 px-4 py-3.5 sm:px-5">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <span className="text-sm font-bold">{capabilityLabel}</span>
        <div className="flex items-center gap-2">
          <Label className="text-xs font-normal" htmlFor={`${label}-inherit`}>
            继承默认顺序
          </Label>
          <Switch
            id={`${label}-inherit`}
            aria-label={`${label}继承默认顺序`}
            checked={isInherited}
            onCheckedChange={(checked: boolean) => onInheritanceChange(checked)}
          />
        </div>
      </div>
      {isInherited ? (
        <p className="text-xs text-muted-foreground">
          {defaultOrder.length > 0 ? defaultOrder.join(' → ') : `默认已禁用${capabilityLabel}`}
        </p>
      ) : (
        <>
          <div className="flex items-center justify-between gap-3">
            <Label className="text-xs font-normal" htmlFor={`${label}-disabled`}>
              禁用此目录的{capabilityLabel}
            </Label>
            <Switch
              id={`${label}-disabled`}
              aria-label={`${label}禁用${capabilityLabel}`}
              checked={isDisabled}
              onCheckedChange={(checked: boolean) => {
                if (checked) {
                  onChange([]);
                  return;
                }
                const enabledOrder =
                  defaultOrder.length > 0
                    ? [...defaultOrder]
                    : providers.length > 0
                      ? [providers[0].name]
                      : [];
                onChange(enabledOrder);
              }}
            />
          </div>
          {!isDisabled && (
            <ProviderOrderEditor
              label={label}
              providers={providers}
              order={order}
              onChange={onChange}
            />
          )}
        </>
      )}
    </div>
  );
}

/**
 * Render file-presence badges for one discovered catalog.
 *
 * @param catalog - Safe catalog metadata.
 * @returns CSV and database presence badges.
 */
function CatalogPresenceBadges({ catalog }: { catalog: ProviderCatalogInfo }) {
  return (
    <div className="flex flex-wrap items-center gap-1.5">
      <StatusPill tone={catalog.csv_filename ? 'success' : 'neutral'}>
        {catalog.csv_filename ? 'CSV 已发现' : '无 CSV'}
      </StatusPill>
      <StatusPill tone={catalog.database_filename ? 'success' : 'neutral'}>
        {catalog.database_filename ? '数据库已发现' : '无数据库'}
      </StatusPill>
    </div>
  );
}

/**
 * Render capability-filtered Provider configuration without exposing raw JSON.
 *
 * @param props - Runtime settings, form values, Provider catalog, and update callback.
 * @returns Provider configuration editor or a fail-closed contract alert.
 */
export function ProviderConfigurationEditor({
  settings,
  values,
  catalog,
  onChange,
}: ProviderConfigurationEditorProps) {
  const configuration = useMemo(
    () => parseProviderConfiguration(settings, values, catalog),
    [catalog, settings, values],
  );
  if (!configuration) {
    return (
      <p role="alert" className="text-sm text-destructive">
        Provider 配置与后端能力目录不一致，已停止编辑以避免覆盖有效配置。
      </p>
    );
  }

  const indexProviders = catalog.providers.filter((provider) => provider.index_content);
  const abstractProviders = catalog.providers.filter((provider) => provider.article_abstract);
  const fulltextProviders = catalog.providers.filter((provider) => provider.article_full_text);

  const updateIndexRoute = (stem: string, provider: string) => {
    onChange(
      configuration.indexSetting.field,
      serializeIndexRoutes({ ...configuration.indexRoutes, [stem]: provider }),
    );
  };

  const updateProxyDecision = (providerName: string, isEnabled: boolean) => {
    onChange(
      configuration.proxySetting.field,
      serializeProviderProxyPolicy(catalog.providers, {
        ...configuration.proxyPolicy,
        [providerName]: isEnabled,
      }),
    );
  };

  const updateDefaultOrder = (
    setting: RuntimeSettingInfo,
    orders: ProviderOrderConfiguration,
    order: string[],
  ) => {
    onChange(setting.field, serializeProviderOrders({ ...orders, default: order }));
  };

  const updateCatalogOrder = (
    setting: RuntimeSettingInfo,
    orders: ProviderOrderConfiguration,
    stem: string,
    order: string[] | undefined,
  ) => {
    const catalogs = { ...orders.catalogs };
    if (order === undefined) {
      delete catalogs[stem];
    } else {
      catalogs[stem] = order;
    }
    onChange(setting.field, serializeProviderOrders({ ...orders, catalogs }));
  };

  return (
    <div className="space-y-10">
      <SettingsSection data-runtime-setting-field={configuration.proxySetting.field}>
        <SettingsSectionHeader>
          <SettingsSectionTitle>{configuration.proxySetting.label}</SettingsSectionTitle>
          <SettingsSectionDescription>
            {configuration.proxySetting.description} ·{' '}
            <span>{getApplyModeLabel(configuration.proxySetting.apply_mode)}</span>
          </SettingsSectionDescription>
        </SettingsSectionHeader>
        <SettingsGroup className="sm:grid sm:grid-cols-2 sm:divide-y-0 sm:gap-px sm:bg-border">
          {catalog.providers.map((provider) => (
            <div
              key={provider.name}
              role="group"
              aria-labelledby={`provider-proxy-label-${provider.name}`}
              className="flex items-center justify-between gap-3 bg-background px-4 py-3 sm:px-5"
            >
              <div className="min-w-0">
                <Label
                  id={`provider-proxy-label-${provider.name}`}
                  htmlFor={`provider-proxy-${provider.name}`}
                  className="font-bold"
                >
                  {provider.name}
                </Label>
                <ProviderCapabilityBadges provider={provider} />
              </div>
              <Switch
                id={`provider-proxy-${provider.name}`}
                aria-label={`${provider.name} 使用 Provider 代理`}
                checked={configuration.proxyPolicy[provider.name] === true}
                onCheckedChange={(checked: boolean) => updateProxyDecision(provider.name, checked)}
              />
            </div>
          ))}
        </SettingsGroup>
      </SettingsSection>

      <section className="space-y-3" aria-labelledby="provider-configuration-title">
        <SettingsSectionHeader>
          <h3 id="provider-configuration-title" className="text-sm leading-6 font-bold">
            Provider 路由
          </h3>
          <SettingsSectionDescription>
            索引 Provider 每个目录单选；摘要页和全文按顺序回退。
          </SettingsSectionDescription>
        </SettingsSectionHeader>

        <div className="flex flex-wrap gap-1.5 px-1">
          {[
            configuration.indexSetting,
            configuration.abstractSetting,
            configuration.fulltextSetting,
          ].map((setting) => (
            <span
              key={setting.field}
              data-runtime-setting-field={setting.field}
              className="inline-flex items-center gap-1.5 rounded-md bg-background px-2 py-1 text-xs shadow-[inset_0_0_0_1px_var(--border)]"
            >
              <span className="font-bold">{setting.label}</span>
              <span className="text-muted-foreground">{getApplyModeLabel(setting.apply_mode)}</span>
            </span>
          ))}
        </div>

        <SettingsGroup className="lg:grid lg:grid-cols-2 lg:divide-y-0 lg:gap-px lg:bg-border [&>*]:bg-background">
          <DefaultProviderOrder
            capabilityLabel="摘要页"
            label="默认摘要页 Provider 顺序"
            providers={abstractProviders}
            order={configuration.abstractOrders.default}
            onChange={(order) =>
              updateDefaultOrder(configuration.abstractSetting, configuration.abstractOrders, order)
            }
          />
          <DefaultProviderOrder
            capabilityLabel="全文"
            label="默认全文 Provider 顺序"
            providers={fulltextProviders}
            order={configuration.fulltextOrders.default}
            onChange={(order) =>
              updateDefaultOrder(configuration.fulltextSetting, configuration.fulltextOrders, order)
            }
          />
        </SettingsGroup>

        <div className="space-y-3 pt-3">
          {catalog.catalogs.map((catalogInfo) => {
            const abstractOverride = configuration.abstractOrders.catalogs[catalogInfo.stem];
            const fulltextOverride = configuration.fulltextOrders.catalogs[catalogInfo.stem];
            return (
              <SettingsGroup
                key={catalogInfo.stem}
                role="region"
                aria-labelledby={`provider-catalog-${catalogInfo.stem}`}
              >
                <div className="flex flex-wrap items-center justify-between gap-3 bg-muted/40 px-4 py-3 sm:px-5">
                  <div className="flex min-w-0 items-center gap-3">
                    <span
                      aria-hidden="true"
                      className="flex size-8 shrink-0 items-center justify-center rounded-lg bg-foreground text-background"
                    >
                      <Database className="size-4" />
                    </span>
                    <div className="min-w-0">
                      <h4
                        id={`provider-catalog-${catalogInfo.stem}`}
                        className="truncate text-sm font-bold"
                      >
                        {catalogInfo.stem}
                      </h4>
                      <p className="text-xs text-muted-foreground">CSV stem / 数据库目录键</p>
                    </div>
                  </div>
                  <CatalogPresenceBadges catalog={catalogInfo} />
                </div>

                <SettingsRow title="索引 Provider" htmlFor={`index-provider-${catalogInfo.stem}`}>
                  <Select
                    value={configuration.indexRoutes[catalogInfo.stem]}
                    disabled={indexProviders.length === 0}
                    onValueChange={(provider) => updateIndexRoute(catalogInfo.stem, provider)}
                  >
                    <SelectTrigger
                      id={`index-provider-${catalogInfo.stem}`}
                      className="w-full sm:w-60"
                      aria-label={`${catalogInfo.stem} 索引 Provider`}
                    >
                      <SelectValue placeholder="请选择索引 Provider" />
                    </SelectTrigger>
                    <SelectContent>
                      {indexProviders.map((provider) => (
                        <SelectItem key={provider.name} value={provider.name}>
                          {provider.name}
                        </SelectItem>
                      ))}
                    </SelectContent>
                  </Select>
                </SettingsRow>

                <CatalogProviderOrder
                  capabilityLabel="摘要页"
                  label={`${catalogInfo.stem}-abstract`}
                  providers={abstractProviders}
                  order={abstractOverride ?? configuration.abstractOrders.default}
                  defaultOrder={configuration.abstractOrders.default}
                  isInherited={abstractOverride === undefined}
                  onChange={(order) =>
                    updateCatalogOrder(
                      configuration.abstractSetting,
                      configuration.abstractOrders,
                      catalogInfo.stem,
                      order,
                    )
                  }
                  onInheritanceChange={(isInherited) =>
                    updateCatalogOrder(
                      configuration.abstractSetting,
                      configuration.abstractOrders,
                      catalogInfo.stem,
                      isInherited ? undefined : [...configuration.abstractOrders.default],
                    )
                  }
                />
                <CatalogProviderOrder
                  capabilityLabel="全文"
                  label={`${catalogInfo.stem}-fulltext`}
                  providers={fulltextProviders}
                  order={fulltextOverride ?? configuration.fulltextOrders.default}
                  defaultOrder={configuration.fulltextOrders.default}
                  isInherited={fulltextOverride === undefined}
                  onChange={(order) =>
                    updateCatalogOrder(
                      configuration.fulltextSetting,
                      configuration.fulltextOrders,
                      catalogInfo.stem,
                      order,
                    )
                  }
                  onInheritanceChange={(isInherited) =>
                    updateCatalogOrder(
                      configuration.fulltextSetting,
                      configuration.fulltextOrders,
                      catalogInfo.stem,
                      isInherited ? undefined : [...configuration.fulltextOrders.default],
                    )
                  }
                />
              </SettingsGroup>
            );
          })}
        </div>
      </section>
    </div>
  );
}

/** Preserve ordered capability admission after all grouped JSON has been parsed. */
function hasProviderConfigurationCapabilities(
  hasKnownProxyProviders: boolean,
  hasCapableIndexRoutes: boolean,
  providers: Map<string, ProviderCapabilityInfo>,
  abstractOrders: ProviderOrderConfiguration,
  fulltextOrders: ProviderOrderConfiguration,
): boolean {
  return !(
    !hasKnownProxyProviders ||
    !hasCapableIndexRoutes ||
    !hasCapableProviderOrders(providers, abstractOrders, 'article_abstract') ||
    !hasCapableProviderOrders(providers, fulltextOrders, 'article_full_text')
  );
}

/** Match each grouped descriptor to its required control before parsing any values. */
function hasProviderSettingControls(
  proxySetting: RuntimeSettingInfo,
  indexSetting: RuntimeSettingInfo,
  abstractSetting: RuntimeSettingInfo,
  fulltextSetting: RuntimeSettingInfo,
): boolean {
  return !(
    proxySetting.control !== 'provider_proxy_policy' ||
    indexSetting.control !== 'index_provider_routes' ||
    abstractSetting.control !== 'provider_order' ||
    fulltextSetting.control !== 'provider_order'
  );
}
