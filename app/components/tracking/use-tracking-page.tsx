'use client';

import { useCallback, useEffect, useState } from 'react';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';

import {
  createFolder,
  getDatabases,
  getFolders,
  getNotificationSettings,
  getPushWeeklyStatus,
  getTrackingStatus,
  pushWeeklyToTracking,
  setTrackingFolder,
  updateNotificationSettings,
  type ManualPushStatus,
  type NotificationSettings,
  type NotificationSettingsUpdate,
} from '@/lib/api';

const EMPTY_DATABASES: string[] = [];

/**
 * Own tracking queries, draft state, push polling, and cache invalidation.
 *
 * @param userId - Authenticated user identifier used by stable query keys.
 * @returns Tracking page view model and actions.
 */
export function useTrackingPage(userId: number) {
  const queryClient = useQueryClient();
  const [newFolderName, setNewFolderName] = useState('');
  const [pushResult, setPushResult] = useState<string | null>(null);
  const [draftSettings, setDraftSettings] = useState<NotificationSettingsUpdate | null>(null);
  const [keywordInput, setKeywordInput] = useState('');
  const [directionInput, setDirectionInput] = useState('');
  const [settingsSaved, setSettingsSaved] = useState(false);
  const [isPushPolling, setIsPushPolling] = useState(false);

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

  const normalizeSettings = useCallback(
    (settings: NotificationSettings | null | undefined): NotificationSettingsUpdate => ({
      keywords: settings?.keywords || [],
      directions: settings?.directions || [],
      selected_databases: settings?.selected_databases || [],
      delivery_method: settings?.delivery_method || 'folder',
      pushplus_token: undefined,
      pushplus_template: settings?.pushplus_template || 'markdown',
      pushplus_topic: settings?.pushplus_topic || '',
      pushplus_channel: settings?.pushplus_channel || 'wechat',
      sync_to_tracking_folder: settings?.sync_to_tracking_folder ?? false,
      ai_base_url: settings?.ai_base_url || '',
      ai_api_key: undefined,
      ai_model: settings?.ai_model || '',
      ai_system_prompt: settings?.ai_system_prompt || '',
      ai_backup_base_url: settings?.ai_backup_base_url || '',
      ai_backup_api_key: undefined,
      ai_backup_model: settings?.ai_backup_model || '',
      ai_backup_system_prompt: settings?.ai_backup_system_prompt || '',
      ai_retry_attempts: settings?.ai_retry_attempts ?? 3,
      enabled: settings?.enabled ?? true,
    }),
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
      if (data.status === 'running') {
        setPushResult(data.message || '推送任务已启动，正在后台执行…');
        setIsPushPolling(true);
        return;
      }
      setPushResult(formatManualPushResult(data));
    },
    onError: (err) => {
      setIsPushPolling(false);
      setPushResult(err instanceof Error ? err.message : '推送失败');
    },
  });
  const requiresTrackingFolder = deliveryMethod === 'folder' || syncToTrackingFolder;
  const normalizedSelectedDatabases = useCallback(
    (selection: string[]): string[] => {
      const allowed = new Set(availableDatabases);
      const next = availableDatabases.filter(
        (dbName) => allowed.has(dbName) && selection.includes(dbName),
      );
      if (next.length === 0 || next.length === availableDatabases.length) {
        return [];
      }
      return next;
    },
    [availableDatabases],
  );
  const effectiveSelectedDatabases = normalizedSelectedDatabases(selectedDatabases);
  const allDatabasesSelected =
    availableDatabases.length === 0 || effectiveSelectedDatabases.length === 0;
  const manualPushLabel =
    pushMut.isPending || isPushPolling
      ? '推送中…'
      : deliveryMethod === 'pushplus'
        ? syncToTrackingFolder
          ? '推送到 PushPlus 并同步文件夹'
          : '推送到 PushPlus'
        : '推送到追踪文件夹';
  const manualPushDescription =
    deliveryMethod === 'pushplus'
      ? syncToTrackingFolder
        ? '将选中数据库中最近一周的文章按当前 AI 推荐规则发送到 PushPlus，并同步写入追踪文件夹。任务会在后台执行。'
        : '将选中数据库中最近一周的文章按当前 AI 推荐规则发送到 PushPlus。任务会在后台执行。'
      : '将选中数据库中最近一周的文章按当前 AI 推荐规则同步到追踪文件夹。任务会在后台执行。';

  const formatManualPushResult = useCallback((data: ManualPushStatus): string => {
    if (data.message) {
      return data.pushed > 0 ? `${data.message}（已推送 ${data.pushed} 篇）` : data.message;
    }
    if (data.status === 'failed') {
      return '推送失败';
    }
    return `成功推送 ${data.pushed} 篇文章`;
  }, []);

  useEffect(() => {
    if (!isPushPolling) {
      return;
    }

    let cancelled = false;

    const pollStatus = async () => {
      try {
        const data = await getPushWeeklyStatus();
        if (cancelled) {
          return;
        }
        if (data.status === 'running') {
          setPushResult(data.message || '推送任务执行中…');
          return;
        }
        setPushResult(formatManualPushResult(data));
        setIsPushPolling(false);
        queryClient.invalidateQueries({ queryKey: ['tracking-status'] });
        queryClient.invalidateQueries({ queryKey: ['folders'] });
      } catch (error) {
        if (cancelled) {
          return;
        }
        setPushResult(error instanceof Error ? error.message : '获取推送状态失败');
        setIsPushPolling(false);
      }
    };

    void pollStatus();
    const intervalId = window.setInterval(() => {
      void pollStatus();
    }, 2000);

    return () => {
      cancelled = true;
      window.clearInterval(intervalId);
    };
  }, [formatManualPushResult, isPushPolling, queryClient]);

  const saveSettingsMut = useMutation({
    mutationFn: () =>
      updateNotificationSettings({
        ...formSettings,
        selected_databases: effectiveSelectedDatabases,
      }),
    onSuccess: (savedSettings) => {
      queryClient.setQueryData(['notification-settings', userId], savedSettings);
      setDraftSettings(null);
      queryClient.invalidateQueries({ queryKey: ['notification-settings', userId] });
      queryClient.invalidateQueries({ queryKey: ['tracking-status'] });
      setSettingsSaved(true);
      setTimeout(() => setSettingsSaved(false), 2000);
    },
  });

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
    updateDraftSettings((current) => ({
      ...current,
      selected_databases: [],
    }));
  }

  /** Update one database selection while preserving the empty-list all-selected contract. */
  function setDatabaseSelected(dbName: string, checked: boolean) {
    updateDraftSettings((current) => {
      const currentSelection = normalizedSelectedDatabases(current.selected_databases);
      const baseSelection =
        currentSelection.length === 0 ? [...availableDatabases] : [...currentSelection];
      const nextSelection = checked
        ? [...baseSelection, dbName]
        : baseSelection.filter((name) => name !== dbName);
      return {
        ...current,
        selected_databases: normalizedSelectedDatabases(nextSelection),
      };
    });
  }

  const trackingFolder = status?.tracking_folder ?? null;

  return {
    folder: {
      createAndSetMutation: createAndSetMut,
      folders,
      name: newFolderName,
      setName: setNewFolderName,
      setTrackingMutation: setTrackMut,
      trackingFolder,
    },
    hasUnsavedSettings,
    manualPush: {
      description: manualPushDescription,
      isPolling: isPushPolling,
      label: manualPushLabel,
      mutation: pushMut,
      requiresTrackingFolder,
      result: pushResult,
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
      },
      databaseSelection: {
        allSelected: allDatabasesSelected,
        available: availableDatabases,
        effectiveSelected: effectiveSelectedDatabases,
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
