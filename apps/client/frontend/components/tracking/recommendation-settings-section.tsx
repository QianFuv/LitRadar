/**
 * Recommendation preferences, database scope, and AI configuration.
 */

import { Bot, Layers, Plus, RefreshCw, X, type AnimatedIcon } from '@/components/ui/animated-icons';

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
  settingsGroupItemStyle,
} from '@/components/settings/settings-section';
import type { TrackingPageViewModel } from '@/components/tracking/use-tracking-page';
import { Button } from '@/components/ui/button';
import { Checkbox } from '@/components/ui/checkbox';
import { HoverHighlightGroup, hoverHighlightItemProps } from '@/components/ui/hover-highlight';
import { NumberInput } from '@/components/ui/field';
import { Input } from '@/components/ui/input';
import { Progress } from '@/components/ui/progress';
import { CHIP_VARIANTS, MotionPresence, MotionSpan } from '@/components/ui/motion';
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@/components/ui/select';
import { Switch } from '@/components/ui/switch';
import { Textarea } from '@/components/ui/textarea';
import { LoadingText } from '@/components/ui/loading-text';
import * as stylex from '@stylexjs/stylex';
import { colors, lineHeights } from '@/lib/theme.stylex';

const DEFAULT_AI_ENDPOINT_VALUE = '__default__';
const DEFAULT_AI_ENDPOINT_LABEL = '使用服务端默认 Endpoint（若已批准）';

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

  const state = {
    model,
    backup,
    endpoints,
    primary,
    retryAttempts,
    directions,
    keywords,
    databaseSelection,
  };
  return (
    <>
      <SettingsSection>
        <SettingsSectionHeader>
          <SettingsSectionTitle icon={Bot}>AI 推荐配置</SettingsSectionTitle>
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
  value,
}: PreferenceTagFieldProps) {
  return (
    <div {...stylex.props(styles.preferenceTagFieldDiv)}>
      <MotionPresence>
        {items.map((item) => (
          <MotionSpan
            key={item}
            {...stylex.props(styles.preferenceTagFieldMotionSpan)}
            data-motion-preference-chip={`${chipKeyPrefix}-${item}`}
            variants={CHIP_VARIANTS}
            initial="hidden"
            animate="visible"
            exit="exit"
          >
            <span {...stylex.props(styles.preferenceTagFieldSpan)}>
              {item}
              <button
                type="button"
                aria-label={`${removeLabelPrefix} ${item}`}
                onClick={() => onRemove(item)}
                {...stylex.props(styles.preferenceTagFieldButton)}
              >
                <X xstyle={styles.preferenceTagFieldX} />
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
        xstyle={styles.preferenceTagFieldInput}
      />
      <Button
        type="button"
        variant="ghost"
        size="icon"
        xstyle={styles.preferenceTagFieldButton2}
        aria-label={addLabel}
        onClick={onAdd}
        disabled={!value.trim()}
      >
        <Plus xstyle={styles.preferenceTagFieldPlus} />
      </Button>
    </div>
  );
}

/** Retain the keyed keyword chips and original add/remove/input handlers. */
function renderRecommendationKeywords(state: Pick<RecommendationViewState, 'model' | 'keywords'>) {
  const { model, keywords } = state;

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
        value={keywords.input}
      />
    </SettingsRow>
  );
}

/** Retain the keyed direction chips and original add/remove/input handlers. */
function renderRecommendationDirections(
  state: Pick<RecommendationViewState, 'model' | 'directions'>,
) {
  const { model, directions } = state;

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
    <div {...stylex.props(styles.fieldWidth)}>
      <Input
        xstyle={styles.secretFieldDivItem}
        id={inputId}
        name={inputName}
        type="password"
        autoComplete="new-password"
        spellCheck={false}
        value={value ?? ''}
        onChange={(event) => onChange(event.target.value)}
        placeholder="sk-…"
      />
      <div {...stylex.props(styles.secretFieldDivItem, styles.secretFieldDiv)}>
        <StatusPill
          tone={hasStoredSecret ? (isPendingClear ? 'warning' : 'success') : 'neutral'}
          xstyle={styles.secretFieldStatusPill}
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
            xstyle={styles.secretFieldButton}
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
      items={[
        { value: DEFAULT_AI_ENDPOINT_VALUE, label: DEFAULT_AI_ENDPOINT_LABEL },
        ...endpoints.available.map((endpoint) => ({ value: endpoint, label: endpoint })),
      ]}
      onValueChange={(nextValue) =>
        onChange(nextValue === DEFAULT_AI_ENDPOINT_VALUE ? '' : nextValue)
      }
      disabled={endpoints.query.isPending || endpoints.query.isError}
    >
      <SelectTrigger id={triggerId} xstyle={styles.fieldWidth}>
        <SelectValue placeholder={placeholder} />
      </SelectTrigger>
      <SelectContent>
        <SelectItem value={DEFAULT_AI_ENDPOINT_VALUE}>{DEFAULT_AI_ENDPOINT_LABEL}</SelectItem>
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
  icon: Icon,
  title,
}: {
  description: string;
  icon: AnimatedIcon;
  title: string;
}) {
  return (
    <SettingsSectionHeader xstyle={styles.aiSectionHeaderSettingsSectionHeader}>
      <span aria-hidden="true" {...stylex.props(styles.aiSectionHeaderSpan)}>
        <Icon xstyle={styles.aiSectionHeaderIcon} />
      </span>
      <div {...stylex.props(styles.aiSectionHeaderDiv)}>
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
          <span role="alert" {...stylex.props(styles.renderPrimaryAiEndpointSpan)}>
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
        icon={RefreshCw}
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
            xstyle={styles.fieldWidth}
          />
        </SettingsRow>
        <SettingsRow
          title="Backup API Key"
          htmlFor="ai-backup-api-key"
          xstyle={styles.renderBackupAiSettingsSettingsRow}
        >
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
            xstyle={styles.renderBackupAiSettingsTextarea}
          />
        </SettingsRow>
      </SettingsGroup>
      <p {...stylex.props(styles.renderBackupAiSettingsP)}>
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
        icon={Bot}
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
            xstyle={styles.fieldWidth}
          />
        </SettingsRow>
        <SettingsRow
          title="API Key"
          htmlFor="ai-api-key"
          xstyle={styles.renderBackupAiSettingsSettingsRow}
        >
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
          <NumberInput
            id="ai-retry-attempts"
            name="ai_retry_attempts"
            min={1}
            max={10}
            step={1}
            value={retryAttempts}
            onValueChange={(value) =>
              model.updateSettings((current) => ({
                ...current,
                ai_retry_attempts: Math.max(1, Math.min(10, Math.round(value ?? 1))),
              }))
            }
            xstyle={styles.renderPrimaryAiSettingsInput}
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
            xstyle={styles.renderBackupAiSettingsTextarea}
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
      <SettingsGroupBody xstyle={styles.renderDatabaseSelectionStateSettingsGroupBody}>
        <output>
          <LoadingText text="正在加载数据库列表…" />
        </output>
      </SettingsGroupBody>
    );
  if (databaseSelection.query.isError)
    return (
      <SettingsGroupBody
        role="alert"
        xstyle={styles.renderDatabaseSelectionStateSettingsGroupBody2}
      >
        {databaseSelection.query.error instanceof Error
          ? databaseSelection.query.error.message
          : '加载数据库列表失败'}
      </SettingsGroupBody>
    );
  if (databaseSelection.available.length === 0)
    return (
      <SettingsGroupBody xstyle={styles.renderDatabaseSelectionStateSettingsGroupBody}>
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
          xstyle={styles.renderDatabaseSelectionStateButton}
          onClick={databaseSelection.selectAll}
        >
          设为全部数据库
        </Button>
      </SettingsRow>
      <HoverHighlightGroup xstyle={settingsGroupItemStyle}>
        {databaseSelection.available.map((databaseName) => {
          const isChecked =
            databaseSelection.allSelected ||
            databaseSelection.effectiveSelected.includes(databaseName);
          return (
            <label
              key={databaseName}
              {...hoverHighlightItemProps}
              {...stylex.props(settingsGroupItemStyle, styles.renderDatabaseSelectionStateLabel)}
            >
              <Layers xstyle={styles.renderDatabaseSelectionStateLayers} aria-hidden="true" />
              <span {...stylex.props(styles.renderDatabaseSelectionStateSpan)}>{databaseName}</span>
              <Checkbox
                checked={isChecked}
                onCheckedChange={(checked) => databaseSelection.setSelected(databaseName, checked)}
              />
            </label>
          );
        })}
      </HoverHighlightGroup>
      <div {...stylex.props(settingsGroupItemStyle, styles.renderDatabaseSelectionStateDiv)}>
        <span>
          当前范围:{' '}
          {databaseSelection.allSelected
            ? `全部数据库（${databaseSelection.available.length} 个）`
            : `已选 ${selectedCount} / ${databaseSelection.available.length} 个数据库`}
        </span>
        <Progress
          aria-hidden="true"
          xstyle={styles.renderDatabaseSelectionStateProgress}
          value={
            databaseSelection.allSelected
              ? 100
              : (selectedCount / databaseSelection.available.length) * 100
          }
        />
      </div>
    </>
  );
}

/** Retain available and unavailable database choices and notice siblings. */
function renderRecommendationDatabases(state: Pick<RecommendationViewState, 'databaseSelection'>) {
  const { databaseSelection } = state;

  return (
    <SettingsSection aria-labelledby="push-databases-label">
      <SettingsSectionHeader>
        <SettingsSectionTitle icon={Layers} id="push-databases-label">
          推送数据库
        </SettingsSectionTitle>
        <SettingsSectionDescription>
          手动推送和自动推送都会按这里的数据库范围执行；不限制时表示全部数据库。
        </SettingsSectionDescription>
      </SettingsSectionHeader>
      <SettingsSectionContent>
        <SettingsGroup xstyle={styles.renderRecommendationDatabasesSettingsSectionContentItem}>
          {renderDatabaseSelectionState(state)}
        </SettingsGroup>
        {databaseSelection.unavailable.length > 0 && (
          <div
            {...stylex.props(
              styles.renderRecommendationDatabasesSettingsSectionContentItem,
              styles.renderRecommendationDatabasesDiv,
            )}
          >
            <p {...stylex.props(styles.secretFieldDivItem, styles.renderRecommendationDatabasesP)}>
              已失效的数据库选择
            </p>
            {databaseSelection.unavailable.map((dbName) => (
              <div key={dbName} {...stylex.props(styles.secretFieldDivItem, styles.secretFieldDiv)}>
                <span {...stylex.props(styles.renderRecommendationDatabasesSpan)}>{dbName}</span>
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
            <p {...stylex.props(styles.secretFieldDivItem, styles.renderRecommendationDatabasesP2)}>
              请先选择可用数据库并移除失效项，或明确设为全部数据库。
            </p>
            {databaseSelection.available.length === 0 && (
              <Button
                xstyle={styles.secretFieldDivItem}
                type="button"
                variant="outline"
                onClick={databaseSelection.selectAll}
              >
                设为全部数据库
              </Button>
            )}
          </div>
        )}
        {databaseSelection.notice && (
          <output
            {...stylex.props(
              styles.statusOutput,
              styles.renderRecommendationDatabasesSettingsSectionContentItem,
              styles.renderRecommendationDatabasesP3,
            )}
          >
            {databaseSelection.notice}
          </output>
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
};

const styles = stylex.create({
  statusOutput: {
    display: 'block',
  },
  aiSectionHeaderIcon: {
    width: '1rem',
    height: '1rem',
  },
  preferenceTagFieldDiv: {
    display: 'flex',
    minHeight: '2.75rem',
    width: '100%',
    flexWrap: 'wrap',
    alignItems: 'center',
    gap: '0.375rem',
    borderRadius: '8px',
    borderStyle: 'solid',
    borderWidth: '1px',
    backgroundColor: colors.background,
    padding: '0.375rem',
    transitionProperty: 'border-color, box-shadow',
    transitionTimingFunction: 'cubic-bezier(0.25, 0.1, 0.25, 1)',
    transitionDuration: '200ms',
    borderColor: {
      default: null,
      ':focus-within': colors.ring,
    },
    boxShadow: {
      default: null,
      ':focus-within': `0 0 0 3px color-mix(in oklab, ${colors.ring} 50%, transparent)`,
    },
  },
  preferenceTagFieldMotionSpan: {
    display: 'inline-flex',
  },
  preferenceTagFieldSpan: {
    display: 'inline-flex',
    height: '1.75rem',
    alignItems: 'center',
    gap: '0.25rem',
    borderRadius: '6px',
    backgroundColor: colors.info,
    paddingRight: '0.125rem',
    paddingLeft: '0.625rem',
    fontSize: '13px',
    fontWeight: 700,
    color: colors.infoForeground,
  },
  preferenceTagFieldButton: {
    display: 'flex',
    width: '1.5rem',
    height: '1.5rem',
    alignItems: 'center',
    justifyContent: 'center',
    borderRadius: '0.25rem',
    transitionProperty:
      'color, background-color, border-color, outline-color, text-decoration-color, fill, stroke',
    transitionTimingFunction: 'cubic-bezier(0.25, 0.1, 0.25, 1)',
    transitionDuration: '200ms',
    backgroundColor: {
      default: null,
      ':hover': {
        default: null,
        '@media (hover: hover)': `color-mix(in oklab, ${colors.primary} 15%, transparent)`,
      },
    },
    boxShadow: {
      default: null,
      ':focus-visible': `0 0 0 2px ${colors.ring}`,
    },
  },
  preferenceTagFieldX: {
    width: '0.75rem',
    height: '0.75rem',
  },
  preferenceTagFieldInput: {
    height: '2rem',
    minWidth: '10rem',
    flex: '1',
    backgroundColor: {
      default: 'transparent',
      ':hover': {
        default: null,
        '@media (hover: hover)': 'transparent',
      },
    },
    paddingInline: '0.375rem',
    boxShadow: {
      default: 'none',
      ':focus-visible': '0 0 0 0px currentcolor',
    },
  },
  preferenceTagFieldButton2: {
    width: '2rem',
    height: '2rem',
    flexShrink: 0,
  },
  preferenceTagFieldPlus: {
    width: '1rem',
    height: '1rem',
  },
  secretFieldDivItem: {
    marginBottom: {
      default: null,
      ':not(:last-child)': '0.5rem',
    },
  },
  fieldWidth: {
    width: {
      default: '100%',
      '@media (min-width: 40rem)': '18rem',
    },
  },
  secretFieldDiv: {
    display: 'flex',
    alignItems: 'center',
    justifyContent: 'space-between',
    gap: '0.5rem',
  },
  secretFieldStatusPill: {
    height: 'auto',
    minHeight: '1.5rem',
    paddingBlock: '0.125rem',
    whiteSpace: 'normal',
  },
  secretFieldButton: {
    height: '1.75rem',
    paddingInline: {
      default: '0.5rem',
      ':has( > svg)': '0.625rem',
    },
    fontSize: '0.75rem',
    lineHeight: lineHeights.textXs,
  },
  aiSectionHeaderSettingsSectionHeader: {
    alignItems: 'center',
    justifyContent: 'flex-start',
    gap: '0.75rem',
  },
  aiSectionHeaderSpan: {
    display: 'flex',
    width: '2rem',
    height: '2rem',
    flexShrink: 0,
    alignItems: 'center',
    justifyContent: 'center',
    borderRadius: '8px',
    backgroundColor: colors.foreground,
    color: colors.background,
  },
  aiSectionHeaderDiv: {
    minWidth: '0px',
    flex: '1',
  },
  renderPrimaryAiEndpointSpan: {
    color: colors.destructive,
  },
  renderBackupAiSettingsSettingsRow: {
    alignItems: {
      default: null,
      '@media (min-width: 40rem)': 'flex-start',
    },
  },
  renderBackupAiSettingsTextarea: {
    minHeight: '7rem',
  },
  renderBackupAiSettingsP: {
    paddingInline: '0.25rem',
    fontSize: '0.75rem',
    lineHeight: lineHeights.textXs,
    color: colors.mutedForeground,
  },
  renderPrimaryAiSettingsInput: {
    width: '6rem',
    textAlign: 'right',
    fontVariantNumeric: 'tabular-nums',
  },
  renderDatabaseSelectionStateSettingsGroupBody: {
    fontSize: '0.875rem',
    lineHeight: lineHeights.textSm,
    color: colors.mutedForeground,
  },
  renderDatabaseSelectionStateSettingsGroupBody2: {
    fontSize: '0.875rem',
    lineHeight: lineHeights.textSm,
    color: colors.destructive,
  },
  renderDatabaseSelectionStateButton: {
    width: {
      default: '100%',
      '@media (min-width: 40rem)': 'auto',
    },
  },
  renderDatabaseSelectionStateLabel: {
    display: 'flex',
    cursor: 'pointer',
    alignItems: 'center',
    gap: '0.75rem',
    paddingInline: {
      default: '1rem',
      '@media (min-width: 40rem)': '1.25rem',
    },
    paddingBlock: '0.75rem',
    fontSize: '0.875rem',
    lineHeight: lineHeights.textSm,
    transitionProperty: 'color, border-color, outline-color, text-decoration-color, fill, stroke',
    transitionTimingFunction: 'cubic-bezier(0.25, 0.1, 0.25, 1)',
    transitionDuration: '200ms',
    contentVisibility: 'auto',
    containIntrinsicSize: '0 40px',
    backgroundColor: {
      default: null,
      ':has(:focus-visible)': `color-mix(in oklab, ${colors.accent} 60%, transparent)`,
      ':hover': {
        default: null,
        '@media (hover: hover)': `color-mix(in oklab, ${colors.accent} 60%, transparent)`,
      },
    },
  },
  renderDatabaseSelectionStateLayers: {
    width: '1rem',
    height: '1rem',
    flexShrink: 0,
    color: colors.mutedForeground,
  },
  renderDatabaseSelectionStateSpan: {
    minWidth: '0px',
    flex: '1',
    wordBreak: 'break-all',
  },
  renderDatabaseSelectionStateDiv: {
    display: 'flex',
    alignItems: 'center',
    justifyContent: 'space-between',
    gap: '0.75rem',
    backgroundColor: `color-mix(in oklab, ${colors.muted} 40%, transparent)`,
    paddingInline: {
      default: '1rem',
      '@media (min-width: 40rem)': '1.25rem',
    },
    paddingBlock: '0.625rem',
    fontSize: '0.75rem',
    lineHeight: lineHeights.textXs,
    color: colors.mutedForeground,
  },
  renderDatabaseSelectionStateProgress: {
    width: '5rem',
  },
  renderRecommendationDatabasesSettingsSectionContentItem: {
    marginBottom: {
      default: null,
      ':not(:last-child)': '0.75rem',
    },
  },
  renderRecommendationDatabasesDiv: {
    borderRadius: '12px',
    borderStyle: 'solid',
    borderWidth: '1px',
    borderColor: `color-mix(in oklab, ${colors.destructive} 40%, transparent)`,
    backgroundColor: `color-mix(in oklab, ${colors.destructive} 5%, transparent)`,
    padding: '1rem',
    fontSize: '0.875rem',
    lineHeight: lineHeights.textSm,
  },
  renderRecommendationDatabasesP: {
    fontWeight: 700,
    color: colors.destructive,
  },
  renderRecommendationDatabasesSpan: {
    wordBreak: 'break-all',
  },
  renderRecommendationDatabasesP2: {
    color: colors.mutedForeground,
  },
  renderRecommendationDatabasesP3: {
    paddingInline: '0.25rem',
    fontSize: '0.875rem',
    lineHeight: lineHeights.textSm,
    color: colors.mutedForeground,
  },
});
