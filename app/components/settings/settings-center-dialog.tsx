'use client';

/**
 * Query-driven aggregated settings dialog for every authenticated route.
 */

import { Bell, Database, KeyRound, Radar, Settings2, ShieldCheck } from 'lucide-react';
import { usePathname, useRouter, useSearchParams } from 'next/navigation';
import { useCallback, useEffect, useRef, useState } from 'react';

import { AccessTokensCard } from '@/components/settings/access-tokens-card';
import { AccountCard } from '@/components/settings/account-card';
import { CnkiSettingsCard } from '@/components/settings/cnki-card';
import { GeneralSettingsSection } from '@/components/settings/general-settings-section';
import { InviteCodeCard } from '@/components/settings/invite-code-card';
import { PasswordCard } from '@/components/settings/password-card';
import { useSettingsCopy } from '@/components/settings/use-settings-copy';
import {
  TrackingSettingsContent,
  type TrackingSettingsController,
} from '@/components/tracking/tracking-settings-content';
import { ConfirmDialog } from '@/components/ui/confirm-dialog';
import {
  SectionedDialogFrame,
  type SectionedDialogSectionDefinition,
} from '@/components/feature/sectioned-dialog';
import { useAuth } from '@/lib/auth-context';
import {
  buildSettingsCenterHref,
  isTrackingSettingsSection,
  parseSettingsSection,
  type SettingsSectionId,
} from '@/lib/settings-center';

type PendingTransition =
  | { kind: 'close'; restoreUrlOnCancel: boolean }
  | {
      kind: 'section';
      restoreUrlOnCancel: boolean;
      target: SettingsSectionId;
    };

type SettingsCenterMountState = {
  initialSection: SettingsSectionId;
  isSessionOpen: boolean;
  observedRequestedSection: SettingsSectionId | null;
};

type SettingsCenterSessionState = {
  activeSection: SettingsSectionId;
  isDialogOpen: boolean;
  isRestoringUrl: boolean;
  observedRequestedSection: SettingsSectionId | null;
  pendingTransition: PendingTransition | null;
};

type SettingsCenterSessionProps = {
  initialSection: SettingsSectionId;
  onCloseSession: () => void;
  onRemoveSection: () => void;
  onReplaceSection: (section: SettingsSectionId) => void;
  requestedSection: SettingsSectionId | null;
  returnFocusRef: React.RefObject<HTMLElement | null>;
  userId: number;
  username: string;
};

type SettingsCategoryContentProps = {
  activeSection: SettingsSectionId;
  copyFeedback: ReturnType<typeof useSettingsCopy>['copyFeedback'];
  handleCopy: ReturnType<typeof useSettingsCopy>['handleCopy'];
  onTrackingControllerChange: (controller: TrackingSettingsController | null) => void;
  userId: number;
  username: string;
};

const SETTINGS_SECTIONS = [
  {
    description: '主题与界面偏好',
    icon: Settings2,
    id: 'general',
    label: '常规',
  },
  {
    description: '追踪文件夹、推荐规则与 AI',
    icon: Radar,
    id: 'tracking',
    label: '文献追踪',
  },
  {
    description: '投递方式、PushPlus 与手动推送',
    icon: Bell,
    id: 'notifications',
    label: '通知与推送',
  },
  {
    description: '中文数据库全文访问',
    icon: Database,
    id: 'data-sources',
    label: '数据源',
  },
  {
    description: '身份、密码与邀请码',
    icon: ShieldCheck,
    id: 'account',
    label: '账号与安全',
  },
  {
    description: '接口访问与第三方集成',
    icon: KeyRound,
    id: 'tokens',
    label: '访问令牌',
  },
] satisfies readonly [
  SectionedDialogSectionDefinition<SettingsSectionId>,
  ...SectionedDialogSectionDefinition<SettingsSectionId>[],
];

/**
 * Synchronize the retained mount state when the URL opens or closes a settings session.
 *
 * @param state - Current retained mount state.
 * @param requestedSection - Section currently requested by the URL.
 * @returns Updated mount state for the new URL snapshot.
 */
function synchronizeMountState(
  state: SettingsCenterMountState,
  requestedSection: SettingsSectionId | null,
): SettingsCenterMountState {
  return {
    initialSection: requestedSection ?? state.initialSection,
    isSessionOpen: requestedSection ? true : state.isSessionOpen,
    observedRequestedSection: requestedSection,
  };
}

/**
 * Reconcile a new URL snapshot with the active dialog and its unsaved tracking draft.
 *
 * @param state - Current dialog state machine.
 * @param requestedSection - Section currently requested by the URL.
 * @param hasUnsavedSettings - Whether the tracking draft differs from stored settings.
 * @returns Dialog state that either follows the URL or guards the transition.
 */
function synchronizeSessionState(
  state: SettingsCenterSessionState,
  requestedSection: SettingsSectionId | null,
  hasUnsavedSettings: boolean,
): SettingsCenterSessionState {
  const observedState = {
    ...state,
    observedRequestedSection: requestedSection,
  };
  if (state.isRestoringUrl) {
    return {
      ...observedState,
      isRestoringUrl: requestedSection !== state.activeSection,
    };
  }
  if (state.pendingTransition) {
    return observedState;
  }
  if (requestedSection === null) {
    return hasUnsavedSettings
      ? {
          ...observedState,
          pendingTransition: { kind: 'close', restoreUrlOnCancel: true },
        }
      : { ...observedState, isDialogOpen: false };
  }
  if (
    hasUnsavedSettings &&
    isTrackingSettingsSection(state.activeSection) &&
    !isTrackingSettingsSection(requestedSection)
  ) {
    return {
      ...observedState,
      pendingTransition: {
        kind: 'section',
        restoreUrlOnCancel: true,
        target: requestedSection,
      },
    };
  }
  return { ...observedState, activeSection: requestedSection };
}

/**
 * Render only the data owners belonging to the active settings category.
 *
 * @param props - Active category, authenticated identity, copy state, and tracking guard.
 * @returns Active settings category content.
 */
function SettingsCategoryContent({
  activeSection,
  copyFeedback,
  handleCopy,
  onTrackingControllerChange,
  userId,
  username,
}: SettingsCategoryContentProps) {
  if (activeSection === 'general') {
    return <GeneralSettingsSection />;
  }
  if (isTrackingSettingsSection(activeSection)) {
    return (
      <TrackingSettingsContent
        userId={userId}
        section={activeSection}
        onControllerChange={onTrackingControllerChange}
      />
    );
  }
  if (activeSection === 'data-sources') {
    return <CnkiSettingsCard userId={userId} copyFeedback={copyFeedback} handleCopy={handleCopy} />;
  }
  if (activeSection === 'account') {
    return (
      <>
        <AccountCard username={username} />
        <PasswordCard />
        <InviteCodeCard copyFeedback={copyFeedback} handleCopy={handleCopy} />
      </>
    );
  }
  return <AccessTokensCard copyFeedback={copyFeedback} handleCopy={handleCopy} />;
}

/**
 * Render one mounted settings session and coordinate guarded URL transitions.
 *
 * @param props - Session identity, URL state, navigation actions, and focus target.
 * @returns Controlled settings dialog session.
 */
function SettingsCenterSession({
  initialSection,
  onCloseSession,
  onRemoveSection,
  onReplaceSection,
  requestedSection,
  returnFocusRef,
  userId,
  username,
}: SettingsCenterSessionProps) {
  const [hasUnsavedSettings, setHasUnsavedSettings] = useState(false);
  const [isSaving, setIsSaving] = useState(false);
  const [sessionState, setSessionState] = useState<SettingsCenterSessionState>({
    activeSection: initialSection,
    isDialogOpen: true,
    isRestoringUrl: false,
    observedRequestedSection: initialSection,
    pendingTransition: null,
  });
  const trackingControllerRef = useRef<TrackingSettingsController | null>(null);
  const { copyFeedback, handleCopy } = useSettingsCopy();
  const { activeSection, isDialogOpen, isRestoringUrl, pendingTransition } = sessionState;

  if (requestedSection !== sessionState.observedRequestedSection) {
    setSessionState(synchronizeSessionState(sessionState, requestedSection, hasUnsavedSettings));
  }

  const handleTrackingControllerChange = useCallback(
    (controller: TrackingSettingsController | null): void => {
      trackingControllerRef.current = controller;
      setHasUnsavedSettings(controller?.hasUnsavedSettings ?? false);
      setIsSaving(controller?.isSaving ?? false);
    },
    [],
  );

  /** Request a category change while protecting the shared tracking draft. */
  function requestSection(section: SettingsSectionId): void {
    if (section === activeSection) {
      return;
    }
    if (
      hasUnsavedSettings &&
      isTrackingSettingsSection(activeSection) &&
      !isTrackingSettingsSection(section)
    ) {
      setSessionState((state) => ({
        ...state,
        pendingTransition: { kind: 'section', restoreUrlOnCancel: false, target: section },
      }));
      return;
    }
    onReplaceSection(section);
  }

  /** Request dialog closure while protecting the shared tracking draft. */
  function requestClose(): void {
    if (isRestoringUrl) {
      return;
    }
    if (hasUnsavedSettings) {
      setSessionState((state) => ({
        ...state,
        pendingTransition: { kind: 'close', restoreUrlOnCancel: false },
      }));
      return;
    }
    onRemoveSection();
    setSessionState((state) => ({ ...state, isDialogOpen: false }));
  }

  /** Cancel one guarded transition and restore URL state changed by browser history. */
  function cancelPendingTransition(): void {
    if (pendingTransition?.restoreUrlOnCancel) {
      onReplaceSection(activeSection);
    }
    setSessionState((state) => ({
      ...state,
      isRestoringUrl: pendingTransition?.restoreUrlOnCancel ?? false,
      pendingTransition: null,
    }));
  }

  /** Discard the tracking draft and finish the requested close or category transition. */
  function confirmPendingTransition(): void {
    if (!pendingTransition) {
      return;
    }
    trackingControllerRef.current?.discardSettings();
    setHasUnsavedSettings(false);
    const transition = pendingTransition;
    if (transition.kind === 'close') {
      if (!transition.restoreUrlOnCancel) {
        onRemoveSection();
      }
      setSessionState((state) => ({
        ...state,
        isDialogOpen: false,
        pendingTransition: null,
      }));
      return;
    }
    if (!transition.restoreUrlOnCancel) {
      onReplaceSection(transition.target);
    }
    setSessionState((state) => ({
      ...state,
      activeSection: transition.target,
      pendingTransition: null,
    }));
  }

  return (
    <>
      <SectionedDialogFrame
        activeSection={activeSection}
        centerSubtitle="集中管理 LitRadar"
        centerTitle="设置中心"
        contentLabelSuffix="设置内容"
        dialogDescription="在一个弹窗中管理常规、文献追踪、通知、数据源、账号安全和访问令牌设置。"
        isBusy={isRestoringUrl}
        isDismissBlocked={pendingTransition !== null}
        isNavigationDisabled={isRestoringUrl}
        navigationLabel="设置分类"
        open={isDialogOpen}
        onOpenChange={(open) => {
          if (!open) {
            requestClose();
          }
        }}
        onSelectSection={requestSection}
        onSessionClosed={onCloseSession}
        returnFocusRef={returnFocusRef}
        sections={SETTINGS_SECTIONS}
      >
        <SettingsCategoryContent
          activeSection={activeSection}
          copyFeedback={copyFeedback}
          handleCopy={handleCopy}
          onTrackingControllerChange={handleTrackingControllerChange}
          userId={userId}
          username={username}
        />
      </SectionedDialogFrame>

      <ConfirmDialog
        open={pendingTransition !== null}
        onOpenChange={(open) => {
          if (!open && !isSaving) {
            cancelPendingTransition();
          }
        }}
        title="放弃未保存的配置？"
        description="当前推荐配置尚未保存。放弃后将继续关闭设置或切换分类。"
        actionLabel="放弃更改"
        cancelLabel="继续编辑"
        pendingLabel="保存处理中…"
        isPending={isSaving}
        onConfirm={confirmPendingTransition}
      />
    </>
  );
}

/**
 * Mount the settings center from the current protected URL query state.
 *
 * @returns Global settings dialog or null when closed or unauthenticated.
 */
export function SettingsCenterDialog() {
  const { user } = useAuth();
  const pathname = usePathname();
  const router = useRouter();
  const searchParams = useSearchParams();
  const rawSection = searchParams.get('settings');
  const requestedSection = parseSettingsSection(rawSection);
  const [mountState, setMountState] = useState<SettingsCenterMountState>({
    initialSection: requestedSection ?? 'general',
    isSessionOpen: requestedSection !== null,
    observedRequestedSection: requestedSection,
  });
  const returnFocusRef = useRef<HTMLElement | null>(null);

  if (requestedSection !== mountState.observedRequestedSection) {
    setMountState(synchronizeMountState(mountState, requestedSection));
  }

  useEffect(() => {
    if (rawSection !== null && requestedSection === null) {
      router.replace(buildSettingsCenterHref(pathname, searchParams, null), { scroll: false });
    }
  }, [pathname, rawSection, requestedSection, router, searchParams]);

  const replaceSection = useCallback(
    (section: SettingsSectionId): void => {
      router.replace(buildSettingsCenterHref(pathname, searchParams, section), { scroll: false });
    },
    [pathname, router, searchParams],
  );

  const removeSection = useCallback((): void => {
    router.replace(buildSettingsCenterHref(pathname, searchParams, null), { scroll: false });
  }, [pathname, router, searchParams]);

  if (!user || !mountState.isSessionOpen) {
    return null;
  }

  return (
    <SettingsCenterSession
      initialSection={mountState.initialSection}
      requestedSection={requestedSection}
      userId={user.id}
      username={user.username}
      returnFocusRef={returnFocusRef}
      onReplaceSection={replaceSection}
      onRemoveSection={removeSection}
      onCloseSession={() =>
        setMountState((state) => ({
          ...state,
          isSessionOpen: false,
        }))
      }
    />
  );
}
