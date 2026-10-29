'use client';

/**
 * Recommendation preferences, database scope, and AI configuration.
 */

import { Bot, Database, LifeBuoy, Plus, X } from 'lucide-react';
import type { ReactNode } from 'react';

import {
  SettingsGroup,
  SettingsGroupBody,
  SettingsRow,
  SettingsSection,
  SettingsSectionContent,
  SettingsSectionDescription,
  SettingsSectionHeader,
  SettingsSectionTitle,
  StatusPill,
} from '@/components/settings/settings-section';
import type { TrackingPageViewModel } from '@/components/tracking/use-tracking-page';
import { Button } from '@/components/ui/button';
import { Checkbox } from '@/components/ui/checkbox';
import { Input } from '@/components/ui/input';
import {
  MOTION_DURATION_SECONDS,
  MotionPresence,
  MotionSpan,
  useMotionTransition,
} from '@/components/ui/motion';
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@/components/ui/select';
import { Switch } from '@/components/ui/switch';
import { Textarea } from '@/components/ui/textarea';

const DEFAULT_AI_ENDPOINT_VALUE = '__default__';
const FIELD_WIDTH_CLASS = 'w-full sm:w-72';

type RecommendationSettingsSectionProps = {
  model: TrackingPageViewModel['recommendation'];
};

/**
 * Render recommendation preferences, database scope, and primary/backup AI controls.
 *
 * @param props - Recommendation-specific tracking view model.
 * @returns Recommendation settings sections.
 */
export function RecommendationSettingsSection({ model }: RecommendationSettingsSectionProps) {
  const { backup, endpoints, primary, retryAttempts } = model.ai;
  const { directions, keywords } = model.preferences;
  const databaseSelection = model.databaseSelection;
  const chipTransition = useMotionTransition(MOTION_DURATION_SECONDS.fast);

  const state = {
    model,
    backup,
    endpoints,
    primary,
    retryAttempts,
    directions,
    keywords,
    databaseSelection,
    chipTransition,
  };
  return (
    <>
      <SettingsSection>
        <SettingsSectionHeader>
          <SettingsSectionTitle>AI 推荐配置</SettingsSectionTitle>
          <SettingsSectionDescription>
            只有在启用推荐、填写关键词或研究方向、且至少有一套可用 AI 配置时，系统才会推送文章
          </SettingsSectionDescription>
        </SettingsSectionHeader>
        <SettingsGroup>
          <SettingsRow
            title="启用推荐"
            htmlFor="notify-enabled"
            description="关闭后不会自动筛选或推送任何文章"
          >
            <Switch
              id="notify-enabled"
              name="notification_enabled"
              checked={model.enabled}
              onCheckedChange={(checked: boolean) =>
                model.updateSettings((current) => ({
                  ...current,
                  enabled: checked,
                }))
              }
            />
          </SettingsRow>
          {renderRecommendationKeywords(state)}
          {renderRecommendationDirections(state)}
        </SettingsGroup>
      </SettingsSection>

      {renderRecommendationDatabases(state)}

      {renderPrimaryAiSettings(state)}

      {renderBackupAiSettings(state)}
    </>
  );
}

type PreferenceTagFieldProps = {
  addLabel: string;
  chipKeyPrefix: string;
  inputId: string;
  inputName: string;
  items: readonly string[];
  onAdd: () => void;
  onInputChange: (value: string) => void;
  onRemove: (item: string) => void;
  placeholder: string;
  removeLabelPrefix: string;
  transition: ReturnType<typeof useMotionTransition>;
  value: string;
};

/**
 * Render an inline tag field where chips and the entry input share one surface.
 *
 * @param props - Current tags, draft input, handlers, and accessible labels.
 * @returns Tag entry field.
 */
function PreferenceTagField({
  addLabel,
  chipKeyPrefix,
  inputId,
  inputName,
  items,
  onAdd,
  onInputChange,
  onRemove,
  placeholder,
  removeLabelPrefix,
  transition,
  value,
}: PreferenceTagFieldProps) {
  return (
    <div className="flex min-h-11 w-full flex-wrap items-center gap-1.5 rounded-lg border bg-background p-1.5 motion-control transition-[border-color,box-shadow] focus-within:border-ring focus-within:ring-[3px] focus-within:ring-ring/50">
      <MotionPresence>
        {items.map((item) => (
          <MotionSpan
            key={item}
            className="inline-flex"
            data-motion-preference-chip={`${chipKeyPrefix}-${item}`}
            initial={{ opacity: 0, scale: 0.96 }}
            animate={{ opacity: 1, scale: 1 }}
            exit={{ opacity: 0, pointerEvents: 'none', scale: 0.96 }}
            transition={transition}
          >
            <span className="inline-flex h-7 items-center gap-1 rounded-md bg-info pr-0.5 pl-2.5 text-[13px] font-bold text-info-foreground">
              {item}
              <button
                type="button"
                aria-label={`${removeLabelPrefix} ${item}`}
                onClick={() => onRemove(item)}
                className="motion-control flex size-6 items-center justify-center rounded transition-colors hover:bg-primary/15 focus-visible:ring-2 focus-visible:ring-ring"
              >
                <X className="size-3" />
              </button>
            </span>
          </MotionSpan>
        ))}
      </MotionPresence>
      <Input
        id={inputId}
        name={inputName}
        autoComplete="off"
        spellCheck={false}
        value={value}
        onChange={(event) => onInputChange(event.target.value)}
        onKeyDown={(event) => {
          if (event.key === 'Enter') {
            event.preventDefault();
            onAdd();
          }
        }}
        placeholder={placeholder}
        className="h-8 min-w-40 flex-1 bg-transparent px-1.5 !shadow-none hover:bg-transparent focus-visible:ring-0"
      />
      <Button
        type="button"
        variant="ghost"
        size="icon"
        className="size-8 shrink-0"
        aria-label={addLabel}
        onClick={onAdd}
        disabled={!value.trim()}
      >
        <Plus className="size-4" />
      </Button>
    </div>
  );
}

/** Retain the keyed keyword chips and original add/remove/input handlers. */
function renderRecommendationKeywords(
  state: Pick<RecommendationViewState, 'model' | 'keywords' | 'chipTransition'>,
) {
  const { model, keywords, chipTransition } = state;

  return (
    <SettingsRow
      isStacked
      title="关键词"
      htmlFor="keyword-input"
      description="文章标题或摘要命中这些词时更容易被推荐"
    >
      <PreferenceTagField
        addLabel="添加关键词"
        chipKeyPrefix="keyword"
        inputId="keyword-input"
        inputName="notification_keyword"
        items={keywords.items}
        onAdd={keywords.add}
        onInputChange={keywords.setInput}
        onRemove={(keyword) =>
          model.updateSettings((current) => ({
            ...current,
            keywords: current.keywords.filter((item) => item !== keyword),
          }))
        }
        placeholder="输入关键词后回车添加"
        removeLabelPrefix="移除关键词"
        transition={chipTransition}
        value={keywords.input}
      />
    </SettingsRow>
  );
}

/** Retain the keyed direction chips and original add/remove/input handlers. */
function renderRecommendationDirections(
  state: Pick<RecommendationViewState, 'model' | 'directions' | 'chipTransition'>,
) {
  const { model, directions, chipTransition } = state;

  return (
    <SettingsRow
      isStacked
      title="研究方向"
      htmlFor="direction-input"
      description="用一句话描述关注的问题，AI 会据此判断相关性"
    >
      <PreferenceTagField
        addLabel="添加研究方向"
        chipKeyPrefix="direction"
        inputId="direction-input"
        inputName="notification_direction"
        items={directions.items}
        onAdd={directions.add}
        onInputChange={directions.setInput}
        onRemove={(direction) =>
          model.updateSettings((current) => ({
            ...current,
            directions: current.directions.filter((item) => item !== direction),
          }))
        }
        placeholder="输入研究方向后回车添加"
        removeLabelPrefix="移除研究方向"
        transition={chipTransition}
        value={directions.input}
      />
    </SettingsRow>
  );
}

type SecretFieldProps = {
  clearLabel: string;
  hasStoredSecret: boolean | undefined;
  inputId: string;
  inputName: string;
  keepLabel: string;
  onChange: (value: string) => void;
  onToggleClear: () => void;
  value: string | null | undefined;
};

/**
 * Render a write-only secret input with its stored-state pill and clear toggle.
 *
 * @param props - Secret value, stored state, and handlers.
 * @returns Secret field controls.
 */
function SecretField({
  clearLabel,
  hasStoredSecret,
  inputId,
  inputName,
  keepLabel,
  onChange,
  onToggleClear,
  value,
}: SecretFieldProps) {
  const isPendingClear = value === null;
  return (
    <div className={`${FIELD_WIDTH_CLASS} space-y-2`}>
      <Input
        id={inputId}
        name={inputName}
        type="password"
        autoComplete="new-password"
        spellCheck={false}
        value={value ?? ''}
        onChange={(event) => onChange(event.target.value)}
        placeholder="sk-…"
      />
      <div className="flex items-center justify-between gap-2">
        <StatusPill
          tone={hasStoredSecret ? (isPendingClear ? 'warning' : 'success') : 'neutral'}
          className="h-auto min-h-6 py-0.5 whitespace-normal"
        >
          {hasStoredSecret
            ? isPendingClear
              ? '保存后清除当前密钥'
              : '已安全保存；留空不会覆盖'
            : '尚未配置'}
        </StatusPill>
        {hasStoredSecret && (
          <Button
            type="button"
            variant="ghost"
            size="sm"
            className="h-7 px-2 text-xs"
            onClick={onToggleClear}
          >
            {isPendingClear ? keepLabel : clearLabel}
          </Button>
        )}
      </div>
    </div>
  );
}

/**
 * Render the approved endpoint picker shared by primary and backup configurations.
 *
 * @param props - Endpoint state, selected value, trigger id, placeholder, and change handler.
 * @returns Endpoint select.
 */
function EndpointSelect({
  endpoints,
  onChange,
  placeholder,
  triggerId,
  value,
}: {
  endpoints: RecommendationViewState['endpoints'];
  onChange: (value: string) => void;
  placeholder: string;
  triggerId: string;
  value: string;
}) {
  return (
    <Select
      value={endpoints.available.includes(value) ? value : DEFAULT_AI_ENDPOINT_VALUE}
      onValueChange={(nextValue) =>
        onChange(nextValue === DEFAULT_AI_ENDPOINT_VALUE ? '' : nextValue)
      }
      disabled={endpoints.query.isPending || endpoints.query.isError}
    >
      <SelectTrigger id={triggerId} className={FIELD_WIDTH_CLASS}>
        <SelectValue placeholder={placeholder} />
      </SelectTrigger>
      <SelectContent>
        <SelectItem value={DEFAULT_AI_ENDPOINT_VALUE}>
          使用服务端默认 Endpoint（若已批准）
        </SelectItem>
        {endpoints.available.map((endpoint) => (
          <SelectItem key={endpoint} value={endpoint}>
            {endpoint}
          </SelectItem>
        ))}
      </SelectContent>
    </Select>
  );
}

/**
 * Render a section heading with a leading icon tile for AI configuration groups.
 *
 * @param props - Icon, title, and description.
 * @returns Section header.
 */
function AiSectionHeader({
  description,
  icon,
  title,
}: {
  description: string;
  icon: ReactNode;
  title: string;
}) {
  return (
    <SettingsSectionHeader className="items-center justify-start gap-3">
      <span
        aria-hidden="true"
        className="flex size-8 shrink-0 items-center justify-center rounded-lg bg-foreground text-background [&_svg]:size-4"
      >
        {icon}
      </span>
      <div className="min-w-0 flex-1">
        <SettingsSectionTitle>{title}</SettingsSectionTitle>
        <SettingsSectionDescription>{description}</SettingsSectionDescription>
      </div>
    </SettingsSectionHeader>
  );
}

/** Retain approved endpoint fallback, pending/error guards and feedback. */
function renderPrimaryAiEndpoint(
  state: Pick<RecommendationViewState, 'model' | 'endpoints' | 'primary'>,
) {
  const { model, endpoints, primary } = state;

  return (
    <SettingsRow
      title="Base URL"
      htmlFor="ai-base-url"
      description={
        endpoints.query.isError ? (
          <span role="alert" className="text-destructive">
            {endpoints.query.error instanceof Error
              ? endpoints.query.error.message
              : '加载 AI Endpoint 列表失败'}
          </span>
        ) : endpoints.query.isSuccess && endpoints.available.length === 0 ? (
          '管理员尚未批准任何 AI Endpoint，AI 推荐将保持禁用。'
        ) : (
          '仅可选择管理员批准的 Endpoint'
        )
      }
    >
      <EndpointSelect
        endpoints={endpoints}
        triggerId="ai-base-url"
        placeholder="选择管理员批准的 Endpoint"
        value={primary.baseUrl}
        onChange={(value) =>
          model.updateSettings((current) => ({
            ...current,
            ai_base_url: value,
          }))
        }
      />
    </SettingsRow>
  );
}

/** Retain the backup configuration fields and their distinct payload names. */
function renderBackupAiSettings(
  state: Pick<RecommendationViewState, 'model' | 'backup' | 'endpoints'>,
) {
  const { model, backup, endpoints } = state;

  return (
    <SettingsSection>
      <AiSectionHeader
        icon={<LifeBuoy />}
        title="备用 AI 配置"
        description="当主配置连续失败后，系统会自动切换到这套备用配置重试。"
      />
      <SettingsGroup>
        <SettingsRow title="Backup Base URL" htmlFor="ai-backup-base-url">
          <EndpointSelect
            endpoints={endpoints}
            triggerId="ai-backup-base-url"
            placeholder="选择备用 Endpoint"
            value={backup.baseUrl}
            onChange={(value) =>
              model.updateSettings((current) => ({
                ...current,
                ai_backup_base_url: value,
              }))
            }
          />
        </SettingsRow>
        <SettingsRow title="Backup Model" htmlFor="ai-backup-model">
          <Input
            id="ai-backup-model"
            name="ai_backup_model"
            autoComplete="off"
            spellCheck={false}
            value={backup.model}
            onChange={(event) =>
              model.updateSettings((current) => ({
                ...current,
                ai_backup_model: event.target.value,
              }))
            }
            placeholder="gpt-4.1-mini"
            className={FIELD_WIDTH_CLASS}
          />
        </SettingsRow>
        <SettingsRow title="Backup API Key" htmlFor="ai-backup-api-key" className="sm:items-start">
          <SecretField
            inputId="ai-backup-api-key"
            inputName="ai_backup_api_key"
            value={backup.apiKey}
            hasStoredSecret={model.storedSettings?.has_ai_backup_api_key}
            clearLabel="清除密钥"
            keepLabel="保留原密钥"
            onChange={(value) =>
              model.updateSettings((current) => ({
                ...current,
                ai_backup_api_key: value,
              }))
            }
            onToggleClear={() =>
              model.updateSettings((current) => ({
                ...current,
                ai_backup_api_key: current.ai_backup_api_key === null ? undefined : null,
              }))
            }
          />
        </SettingsRow>
        <SettingsRow
          isStacked
          title="Backup System Prompt"
          htmlFor="ai-backup-system-prompt"
          description="可选，留空时沿用主配置或服务端默认提示词"
        >
          <Textarea
            id="ai-backup-system-prompt"
            name="ai_backup_system_prompt"
            autoComplete="off"
            spellCheck={false}
            value={backup.systemPrompt}
            onChange={(event) =>
              model.updateSettings((current) => ({
                ...current,
                ai_backup_system_prompt: event.target.value,
              }))
            }
            placeholder="Optional backup prompt override."
            className="min-h-28"
          />
        </SettingsRow>
      </SettingsGroup>
      <p className="px-1 text-xs text-muted-foreground">
        未配置关键词或研究方向时不会推送；主备 AI 都不可用时同样会跳过推送。
      </p>
    </SettingsSection>
  );
}

/** Retain primary, retry and backup configuration order. */
function renderPrimaryAiSettings(
  state: Pick<RecommendationViewState, 'model' | 'primary' | 'retryAttempts' | 'endpoints'>,
) {
  const { model, primary, retryAttempts } = state;

  return (
    <SettingsSection>
      <AiSectionHeader
        icon={<Bot />}
        title="主 AI 配置"
        description="优先使用这套配置进行筛选；留空字段会回退到服务端默认值。"
      />
      <SettingsGroup>
        {renderPrimaryAiEndpoint(state)}
        <SettingsRow title="Model" htmlFor="ai-model">
          <Input
            id="ai-model"
            name="ai_model"
            autoComplete="off"
            spellCheck={false}
            value={primary.model}
            onChange={(event) =>
              model.updateSettings((current) => ({
                ...current,
                ai_model: event.target.value,
              }))
            }
            placeholder="gpt-4.1-mini"
            className={FIELD_WIDTH_CLASS}
          />
        </SettingsRow>
        <SettingsRow title="API Key" htmlFor="ai-api-key" className="sm:items-start">
          <SecretField
            inputId="ai-api-key"
            inputName="ai_api_key"
            value={primary.apiKey}
            hasStoredSecret={model.storedSettings?.has_ai_api_key}
            clearLabel="清除密钥"
            keepLabel="保留原密钥"
            onChange={(value) =>
              model.updateSettings((current) => ({
                ...current,
                ai_api_key: value,
              }))
            }
            onToggleClear={() =>
              model.updateSettings((current) => ({
                ...current,
                ai_api_key: current.ai_api_key === null ? undefined : null,
              }))
            }
          />
        </SettingsRow>
        <SettingsRow
          title="失败重试次数"
          htmlFor="ai-retry-attempts"
          description="单次筛选失败后的重试上限（1–10）"
        >
          <Input
            id="ai-retry-attempts"
            name="ai_retry_attempts"
            type="number"
            autoComplete="off"
            inputMode="numeric"
            min={1}
            max={10}
            value={retryAttempts}
            onChange={(event) =>
              model.updateSettings((current) => ({
                ...current,
                ai_retry_attempts: Math.max(1, Math.min(10, Number(event.target.value) || 1)),
              }))
            }
            className="w-24 text-right tabular-nums"
          />
        </SettingsRow>
        <SettingsRow
          isStacked
          title="System Prompt"
          htmlFor="ai-system-prompt"
          description="告诉模型如何判断文章与你的研究是否相关"
        >
          <Textarea
            id="ai-system-prompt"
            name="ai_system_prompt"
            autoComplete="off"
            spellCheck={false}
            value={primary.systemPrompt}
            onChange={(event) =>
              model.updateSettings((current) => ({
                ...current,
                ai_system_prompt: event.target.value,
              }))
            }
            placeholder="Describe how the model should evaluate article relevance."
            className="min-h-28"
          />
        </SettingsRow>
      </SettingsGroup>
    </SettingsSection>
  );
}

/** Retain database pending, error, empty and available selection priority. */
function renderDatabaseSelectionState(state: Pick<RecommendationViewState, 'databaseSelection'>) {
  const { databaseSelection } = state;
  if (databaseSelection.query.isPending)
    return (
      <SettingsGroupBody role="status" className="text-sm text-muted-foreground">
        正在加载数据库列表…
      </SettingsGroupBody>
    );
  if (databaseSelection.query.isError)
    return (
      <SettingsGroupBody role="alert" className="text-sm text-destructive">
        {databaseSelection.query.error instanceof Error
          ? databaseSelection.query.error.message
          : '加载数据库列表失败'}
      </SettingsGroupBody>
    );
  if (databaseSelection.available.length === 0)
    return (
      <SettingsGroupBody className="text-sm text-muted-foreground">
        当前没有可用数据库。
      </SettingsGroupBody>
    );
  const selectedCount = databaseSelection.effectiveSelected.filter((name) =>
    databaseSelection.available.includes(name),
  ).length;
  return (
    <>
      <SettingsRow title="全部数据库" description="选中后，新增加的数据库也会自动纳入推送范围。">
        <Button
          type="button"
          variant={databaseSelection.allSelected ? 'default' : 'outline'}
          size="sm"
          className="w-full sm:w-auto"
          onClick={databaseSelection.selectAll}
        >
          设为全部数据库
        </Button>
      </SettingsRow>
      {databaseSelection.available.map((databaseName) => {
        const isChecked =
          databaseSelection.allSelected ||
          databaseSelection.effectiveSelected.includes(databaseName);
        return (
          <label
            key={databaseName}
            className="motion-control content-visibility-row flex cursor-pointer items-center gap-3 px-4 py-3 text-sm transition-colors hover:bg-accent/60 focus-within:bg-accent/60 sm:px-5"
          >
            <Database className="size-4 shrink-0 text-muted-foreground" aria-hidden="true" />
            <span className="min-w-0 flex-1 break-all">{databaseName}</span>
            <Checkbox
              checked={isChecked}
              onCheckedChange={(checked: boolean | 'indeterminate') =>
                databaseSelection.setSelected(databaseName, Boolean(checked))
              }
            />
          </label>
        );
      })}
      <div className="flex items-center justify-between gap-3 bg-muted/40 px-4 py-2.5 text-xs text-muted-foreground sm:px-5">
        <span>
          当前范围:{' '}
          {databaseSelection.allSelected
            ? `全部数据库（${databaseSelection.available.length} 个）`
            : `已选 ${selectedCount} / ${databaseSelection.available.length} 个数据库`}
        </span>
        <span aria-hidden="true" className="flex h-1.5 w-20 overflow-hidden rounded-full bg-muted">
          <span
            className="h-full rounded-full bg-primary"
            style={{
              width: `${
                databaseSelection.allSelected
                  ? 100
                  : (selectedCount / databaseSelection.available.length) * 100
              }%`,
            }}
          />
        </span>
      </div>
    </>
  );
}

/** Retain available and unavailable database choices and notice siblings. */
function renderRecommendationDatabases(state: Pick<RecommendationViewState, 'databaseSelection'>) {
  const { databaseSelection } = state;

  return (
    <SettingsSection role="group" aria-labelledby="push-databases-label">
      <SettingsSectionHeader>
        <SettingsSectionTitle id="push-databases-label">推送数据库</SettingsSectionTitle>
        <SettingsSectionDescription>
          手动推送和自动推送都会按这里的数据库范围执行；不限制时表示全部数据库。
        </SettingsSectionDescription>
      </SettingsSectionHeader>
      <SettingsSectionContent className="space-y-3">
        <SettingsGroup>{renderDatabaseSelectionState(state)}</SettingsGroup>
        {databaseSelection.unavailable.length > 0 && (
          <div className="space-y-2 rounded-xl border border-destructive/40 bg-destructive/5 p-4 text-sm">
            <p className="font-bold text-destructive">已失效的数据库选择</p>
            {databaseSelection.unavailable.map((dbName) => (
              <div key={dbName} className="flex items-center justify-between gap-2">
                <span className="break-all">{dbName}</span>
                <Button
                  type="button"
                  variant="outline"
                  size="sm"
                  aria-label={`移除失效数据库 ${dbName}`}
                  disabled={databaseSelection.effectiveSelected.length === 1}
                  onClick={() => databaseSelection.setSelected(dbName, false)}
                >
                  移除
                </Button>
              </div>
            ))}
            <p className="text-muted-foreground">
              请先选择可用数据库并移除失效项，或明确设为全部数据库。
            </p>
            {databaseSelection.available.length === 0 && (
              <Button type="button" variant="outline" onClick={databaseSelection.selectAll}>
                设为全部数据库
              </Button>
            )}
          </div>
        )}
        {databaseSelection.notice && (
          <p role="status" className="px-1 text-sm text-muted-foreground">
            {databaseSelection.notice}
          </p>
        )}
      </SettingsSectionContent>
    </SettingsSection>
  );
}

type RecommendationViewState = RecommendationSettingsSectionProps & {
  backup: RecommendationSettingsSectionProps['model']['ai']['backup'];
  endpoints: RecommendationSettingsSectionProps['model']['ai']['endpoints'];
  primary: RecommendationSettingsSectionProps['model']['ai']['primary'];
  retryAttempts: number;
  directions: RecommendationSettingsSectionProps['model']['preferences']['directions'];
  keywords: RecommendationSettingsSectionProps['model']['preferences']['keywords'];
  databaseSelection: RecommendationSettingsSectionProps['model']['databaseSelection'];
  chipTransition: ReturnType<typeof useMotionTransition>;
};
