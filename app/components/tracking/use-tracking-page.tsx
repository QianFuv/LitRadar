'use client';

import { useCallback, useEffect, useState } from 'react';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';

import {
  acknowledgeUnknownPushWeeklyRun,
  cancelPushWeeklyRun,
  createFolder,
  getAiEndpoints,
  getDatabases,
  getFolders,
  getNotificationSettings,
  getPushWeeklyStatus,
  getTrackingStatus,
  pushWeeklyToTracking,
  setTrackingFolder,
  updateNotificationSettings,
  type ManualPushState,
  type ManualPushStatus,
  type NotificationSettings,
  type NotificationSettingsUpdate,
} from '@/lib/api';

const EMPTY_DATABASES: string[] = [];
const EMPTY_AI_ENDPOINTS: string[] = [];

/**
 * Return a fallback label for every declared manual-push state.
 *
 * @param status - Generated manual-push state.
 * @param pushed - Number of delivered articles.
 * @returns User-facing fallback text.
 */
function manualPushFallbackMessage(status: ManualPushState, pushed: number): string {
  switch (status) {
    case 'idle':
      return '暂无手动推送任务';
    case 'pending':
      return '推送任务正在排队';
    case 'running':
      return '推送任务正在执行';
    case 'completed':
      return `成功推送 ${pushed} 篇文章`;
    case 'failed':
      return '推送失败';
    case 'cancelled':
      return '推送已取消';
    case 'timed_out':
      return '推送已超时';
    case 'unknown':
      return '推送结果未知，请先检查投递状态';
    default: {
      const unreachable: never = status;
      return unreachable;
    }
  }
}

/**
 * Own tracking queries, draft state, push polling, and cache invalidation.
 *
 * @param userId - Authenticated user identifier used by stable query keys.
 * @returns Tracking page view model and actions.
 */
export function useTrackingPage(userId: number) {
  const queryClient = useQueryClient();
  const [newFolderName, setNewFolderName] = useState('');
  const [draftSettings, setDraftSettings] = useState<NotificationSettingsUpdate | null>(null);
  const [keywordInput, setKeywordInput] = useState('');
  const [directionInput, setDirectionInput] = useState('');
  const [settingsSaved, setSettingsSaved] = useState(false);
  const [databaseSelectionNotice, setDatabaseSelectionNotice] = useState<string | null>(null);

  const { data: status } = useQuery({
    queryKey: ['tracking-status'],
    queryFn: () => getTrackingStatus(),
    enabled: true,
  });

  const databasesQuery = useQuery({
    queryKey: ['databases'],
    queryFn: () => getDatabases(),
    enabled: true,
  });
  const availableDatabases = databasesQuery.data ?? EMPTY_DATABASES;

  const { data: folders = [] } = useQuery({
    queryKey: ['folders', userId],
    queryFn: () => getFolders(),
    enabled: true,
  });

  const notificationSettingsQuery = useQuery({
    queryKey: ['notification-settings', userId],
    queryFn: () => getNotificationSettings(),
    enabled: true,
  });
  const notifySettings = notificationSettingsQuery.data;
  const aiEndpointsQuery = useQuery({
    queryKey: ['ai-endpoints', userId],
    queryFn: () => getAiEndpoints(),
    enabled: true,
  });
  const availableAiEndpoints = aiEndpointsQuery.data ?? EMPTY_AI_ENDPOINTS;
  const manualPushQuery = useQuery({
    queryKey: ['manual-push', userId],
    queryFn: () => getPushWeeklyStatus(),
    enabled: true,
    retry: false,
    refetchInterval: (query) => {
      const pushStatus = query.state.data?.status;
      return pushStatus === 'pending' || pushStatus === 'running' ? 2000 : false;
    },
  });

  const normalizeSettings = useCallback(
    (settings: NotificationSettings | null | undefined): NotificationSettingsUpdate =>
      normalizeTrackingSettings(settings),
    [],
  );

  const formSettings = draftSettings || normalizeSettings(notifySettings);
  const hasUnsavedSettings = draftSettings !== null;
  const {
    keywords,
    directions,
    selected_databases: selectedDatabases,
    delivery_method: deliveryMethod,
    pushplus_token: pushplusToken,
    pushplus_template: pushplusTemplate,
    pushplus_topic: pushplusTopic,
    pushplus_channel: pushplusChannel,
    sync_to_tracking_folder: syncToTrackingFolder,
    ai_base_url: aiBaseUrl,
    ai_api_key: aiApiKey,
    ai_model: aiModel,
    ai_system_prompt: aiSystemPrompt,
    ai_backup_base_url: aiBackupBaseUrl,
    ai_backup_api_key: aiBackupApiKey,
    ai_backup_model: aiBackupModel,
    ai_backup_system_prompt: aiBackupSystemPrompt,
    ai_retry_attempts: aiRetryAttempts,
    enabled: notifyEnabled,
  } = formSettings;

  const updateDraftSettings = useCallback(
    (updater: (current: NotificationSettingsUpdate) => NotificationSettingsUpdate) => {
      setDraftSettings((current) => updater(current || normalizeSettings(notifySettings)));
      setSettingsSaved(false);
    },
    [normalizeSettings, notifySettings],
  );

  useEffect(() => {
    if (!hasUnsavedSettings) {
      return;
    }
    const handleBeforeUnload = (event: BeforeUnloadEvent) => {
      event.preventDefault();
      event.returnValue = '';
    };
    window.addEventListener('beforeunload', handleBeforeUnload);
    return () => window.removeEventListener('beforeunload', handleBeforeUnload);
  }, [hasUnsavedSettings]);

  const setTrackMut = useMutation({
    mutationFn: (folderId: number) => setTrackingFolder(folderId),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['tracking-status'] });
      queryClient.invalidateQueries({ queryKey: ['folders'] });
    },
  });

  const createAndSetMut = useMutation({
    mutationFn: async (name: string) => {
      const folder = await createFolder(name, true);
      return folder;
    },
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['tracking-status'] });
      queryClient.invalidateQueries({ queryKey: ['folders'] });
      setNewFolderName('');
    },
  });

  const pushMut = useMutation({
    mutationFn: () => pushWeeklyToTracking(),
    onSuccess: (data) => {
      queryClient.setQueryData(['manual-push', userId], data);
    },
  });
  const cancelPushMut = useMutation({
    mutationFn: (jobId: string) => cancelPushWeeklyRun(jobId),
    onSuccess: (data) => {
      queryClient.setQueryData(['manual-push', userId], data);
    },
  });
  const acknowledgeUnknownPushMut = useMutation({
    mutationFn: (jobId: string) => acknowledgeUnknownPushWeeklyRun(jobId),
    onSuccess: (data) => {
      queryClient.setQueryData(['manual-push', userId], data);
    },
  });
  const requiresTrackingFolder = deliveryMethod === 'folder' || syncToTrackingFolder;
  const effectiveSelectedDatabases = Array.from(new Set(selectedDatabases));
  const unavailableSelectedDatabases = getUnavailableTrackingDatabases(
    databasesQuery.isSuccess,
    effectiveSelectedDatabases,
    availableDatabases,
  );
  const allDatabasesSelected = effectiveSelectedDatabases.length === 0;
  const manualPushStatus = manualPushQuery.data;
  const isManualPushActive = isActiveManualPush(manualPushStatus);
  const manualPushLabel = getManualPushLabel(
    pushMut.isPending,
    manualPushStatus,
    deliveryMethod,
    syncToTrackingFolder,
  );
  const manualPushDescription = getManualPushDescription(deliveryMethod, syncToTrackingFolder);

  const formatManualPushResult = useCallback((data: ManualPushStatus): string => {
    if (data.message) {
      return data.pushed > 0 ? `${data.message}（已推送 ${data.pushed} 篇）` : data.message;
    }
    return manualPushFallbackMessage(data.status, data.pushed);
  }, []);
  const manualPushError =
    pushMut.error ??
    cancelPushMut.error ??
    acknowledgeUnknownPushMut.error ??
    manualPushQuery.error;
  const pushResult = getManualPushResult(manualPushError, manualPushStatus, formatManualPushResult);

  useEffect(() => {
    if (
      !manualPushStatus ||
      manualPushStatus.status === 'idle' ||
      manualPushStatus.status === 'pending' ||
      manualPushStatus.status === 'running'
    ) {
      return;
    }
    queryClient.invalidateQueries({ queryKey: ['tracking-status'] });
    queryClient.invalidateQueries({ queryKey: ['folders'] });
  }, [manualPushStatus, queryClient]);

  const saveSettingsMut = useMutation({
    mutationFn: () =>
      updateNotificationSettings({
        ...formSettings,
        selected_databases: effectiveSelectedDatabases,
        ai_base_url:
          aiEndpointsQuery.isSuccess && !availableAiEndpoints.includes(formSettings.ai_base_url)
            ? ''
            : formSettings.ai_base_url,
        ai_backup_base_url:
          aiEndpointsQuery.isSuccess &&
          !availableAiEndpoints.includes(formSettings.ai_backup_base_url)
            ? ''
            : formSettings.ai_backup_base_url,
      }),
    onSuccess: (savedSettings) => {
      queryClient.setQueryData(['notification-settings', userId], savedSettings);
      setDraftSettings(null);
      setDatabaseSelectionNotice(null);
      queryClient.invalidateQueries({ queryKey: ['notification-settings', userId] });
      queryClient.invalidateQueries({ queryKey: ['tracking-status'] });
      setSettingsSaved(true);
      setTimeout(() => setSettingsSaved(false), 2000);
    },
  });
  const resetSaveSettingsMutation = saveSettingsMut.reset;

  /** Restore the recommendation draft to the latest stored query value. */
  const discardSettings = useCallback((): void => {
    setDraftSettings(null);
    setDatabaseSelectionNotice(null);
    setKeywordInput('');
    setDirectionInput('');
    setSettingsSaved(false);
    resetSaveSettingsMutation();
  }, [resetSaveSettingsMutation]);

  /** Add the trimmed keyword draft when it is new. */
  function addKeyword() {
    const val = keywordInput.trim();
    if (val && !keywords.includes(val)) {
      updateDraftSettings((current) => ({
        ...current,
        keywords: [...current.keywords, val],
      }));
    }
    setKeywordInput('');
  }

  /** Add the trimmed research direction draft when it is new. */
  function addDirection() {
    const val = directionInput.trim();
    if (val && !directions.includes(val)) {
      updateDraftSettings((current) => ({
        ...current,
        directions: [...current.directions, val],
      }));
    }
    setDirectionInput('');
  }

  /** Represent all database selection with the existing empty-list contract. */
  function selectAllDatabases() {
    setDatabaseSelectionNotice(null);
    updateDraftSettings((current) => ({
      ...current,
      selected_databases: [],
    }));
  }

  /** Update one database selection while preserving the empty-list all-selected contract. */
  function setDatabaseSelected(dbName: string, checked: boolean) {
    const currentSelection = allDatabasesSelected ? availableDatabases : effectiveSelectedDatabases;
    if (!checked && currentSelection.length === 1 && currentSelection[0] === dbName) {
      setDatabaseSelectionNotice('至少保留一个数据库；如需全部，请点击“设为全部数据库”。');
      return;
    }
    setDatabaseSelectionNotice(null);
    updateDraftSettings((current) => {
      const baseSelection =
        current.selected_databases.length === 0 ? availableDatabases : current.selected_databases;
      const nextSelection = checked
        ? Array.from(new Set([...baseSelection, dbName]))
        : baseSelection.filter((name) => name !== dbName);
      return nextSelection.length === 0
        ? current
        : {
            ...current,
            selected_databases: nextSelection,
          };
    });
  }

  const trackingFolder = getTrackingFolder(status);

  return {
    folder: {
      createAndSetMutation: createAndSetMut,
      folders,
      name: newFolderName,
      setName: setNewFolderName,
      setTrackingMutation: setTrackMut,
      trackingFolder,
    },
    discardSettings,
    hasUnsavedSettings,
    manualPush: {
      acknowledgeUnknownMutation: acknowledgeUnknownPushMut,
      cancelMutation: cancelPushMut,
      description: manualPushDescription,
      hasError: Boolean(manualPushError),
      isLoading: manualPushQuery.isPending,
      isPolling: isManualPushActive,
      label: manualPushLabel,
      mutation: pushMut,
      requiresTrackingFolder,
      result: pushResult,
      status: manualPushStatus,
      trackingFolder,
      weeklyArticlesAvailable: status?.weekly_articles_available,
    },
    recommendation: {
      ai: {
        backup: {
          apiKey: aiBackupApiKey,
          baseUrl: aiBackupBaseUrl,
          model: aiBackupModel,
          systemPrompt: aiBackupSystemPrompt,
        },
        primary: {
          apiKey: aiApiKey,
          baseUrl: aiBaseUrl,
          model: aiModel,
          systemPrompt: aiSystemPrompt,
        },
        retryAttempts: aiRetryAttempts,
        endpoints: {
          available: availableAiEndpoints,
          query: aiEndpointsQuery,
        },
      },
      databaseSelection: {
        allSelected: allDatabasesSelected,
        available: availableDatabases,
        effectiveSelected: effectiveSelectedDatabases,
        unavailable: unavailableSelectedDatabases,
        notice: databaseSelectionNotice,
        query: databasesQuery,
        selectAll: selectAllDatabases,
        setSelected: setDatabaseSelected,
      },
      delivery: {
        method: deliveryMethod,
        pushplus: {
          channel: pushplusChannel,
          template: pushplusTemplate,
          token: pushplusToken,
          topic: pushplusTopic,
        },
        syncToTrackingFolder,
      },
      enabled: notifyEnabled,
      hasDraft: draftSettings !== null,
      notificationQuery: notificationSettingsQuery,
      preferences: {
        directions: {
          add: addDirection,
          input: directionInput,
          items: directions,
          setInput: setDirectionInput,
        },
        keywords: {
          add: addKeyword,
          input: keywordInput,
          items: keywords,
          setInput: setKeywordInput,
        },
      },
      save: {
        didSave: settingsSaved,
        mutation: saveSettingsMut,
      },
      storedSettings: notifySettings,
      trackingFolder,
      updateSettings: updateDraftSettings,
    },
  };
}

/** Tracking page view model grouped by rendered section. */
export type TrackingPageViewModel = ReturnType<typeof useTrackingPage>;

/** Retain preferences arrays and the original delivery-method default. */
function normalizeTrackingPreferences(settings: NotificationSettings | null | undefined) {
  return {
    keywords: settings?.keywords || [],
    directions: settings?.directions || [],
    selected_databases: settings?.selected_databases || [],
    delivery_method: settings?.delivery_method || 'folder',
  };
}

/** Retain omitted PushPlus secret and each original transport fallback. */
function normalizeTrackingDelivery(settings: NotificationSettings | null | undefined) {
  return {
    pushplus_token: undefined,
    pushplus_template: settings?.pushplus_template || 'markdown',
    pushplus_topic: settings?.pushplus_topic || '',
    pushplus_channel: settings?.pushplus_channel || 'wechat',
    sync_to_tracking_folder: settings?.sync_to_tracking_folder ?? false,
  };
}

/** Retain primary AI defaults without exposing the stored credential. */
function normalizePrimaryAiSettings(settings: NotificationSettings | null | undefined) {
  return {
    ai_base_url: settings?.ai_base_url || '',
    ai_api_key: undefined,
    ai_model: settings?.ai_model || '',
    ai_system_prompt: settings?.ai_system_prompt || '',
  };
}

/** Retain backup AI defaults without exposing the stored credential. */
function normalizeBackupAiSettings(settings: NotificationSettings | null | undefined) {
  return {
    ai_backup_base_url: settings?.ai_backup_base_url || '',
    ai_backup_api_key: undefined,
    ai_backup_model: settings?.ai_backup_model || '',
    ai_backup_system_prompt: settings?.ai_backup_system_prompt || '',
  };
}

/** Retain nullish retry and enablement defaults. */
function normalizeTrackingEnablement(settings: NotificationSettings | null | undefined) {
  return {
    ai_retry_attempts: settings?.ai_retry_attempts ?? 3,
    enabled: settings?.enabled ?? true,
  };
}

/** Normalize saved tracking fields in their original key and evaluation order. */
function normalizeTrackingSettings(
  settings: NotificationSettings | null | undefined,
): NotificationSettingsUpdate {
  return {
    ...normalizeTrackingPreferences(settings),
    ...normalizeTrackingDelivery(settings),
    ...normalizePrimaryAiSettings(settings),
    ...normalizeBackupAiSettings(settings),
    ...normalizeTrackingEnablement(settings),
  };
}

/** Preserve unavailable-database filtering only after successful catalog acquisition. */
function getUnavailableTrackingDatabases(
  isSuccess: boolean,
  selected: string[],
  available: string[],
): string[] {
  return isSuccess ? selected.filter((name) => !available.includes(name)) : [];
}
/** Identify only pending and running jobs as active. */
function isActiveManualPush(status: ManualPushStatus | undefined): boolean {
  return status?.status === 'pending' || status?.status === 'running';
}
/** Resolve queued/running feedback before delivery configuration. */
function getManualPushLabel(
  isPending: boolean,
  status: ManualPushStatus | undefined,
  method: NotificationSettingsUpdate['delivery_method'],
  shouldSync: boolean,
): string {
  if (isPending || status?.status === 'pending') return '排队中…';
  if (status?.status === 'running') return '推送中…';
  if (method === 'pushplus') return shouldSync ? '推送到 PushPlus 并同步文件夹' : '推送到 PushPlus';
  return '推送到追踪文件夹';
}
/** Retain the original delivery-specific explanation. */
function getManualPushDescription(
  method: NotificationSettingsUpdate['delivery_method'],
  shouldSync: boolean,
): string {
  if (method === 'pushplus')
    return shouldSync
      ? '将选中数据库中最近一周的文章按当前 AI 推荐规则发送到 PushPlus，并同步写入追踪文件夹。任务会在后台执行。'
      : '将选中数据库中最近一周的文章按当前 AI 推荐规则发送到 PushPlus。任务会在后台执行。';
  return '将选中数据库中最近一周的文章按当前 AI 推荐规则同步到追踪文件夹。任务会在后台执行。';
}
/** Resolve the existing error priority before a non-idle job's message. */
function getManualPushResult(
  error: unknown,
  status: ManualPushStatus | undefined,
  format: (data: ManualPushStatus) => string,
): string | null {
  if (error) return error instanceof Error ? error.message : '推送任务操作失败';
  if (status && status.status !== 'idle') return format(status);
  return null;
}
/** Retain the server tracking-folder fallback without changing query ownership. */
function getTrackingFolder(status: Awaited<ReturnType<typeof getTrackingStatus>> | undefined) {
  return status?.tracking_folder ?? null;
}
