/**
 * Query-driven aggregated settings dialog for every authenticated route.
 */

import { Radar } from 'lucide-react';
import { Bell, KeyRound, Layers, Settings, ShieldCheck } from '@/components/ui/animated-icons';
import { useLocation, useNavigate, useSearchParams } from 'react-router';
import { useCallback, useEffect, useRef, useState } from 'react';

import { AccessTokensCard } from '@/components/settings/access-tokens-card';
import { AccountCard, AccountSignInCard } from '@/components/settings/account-card';
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
import { MotionDiv, MotionPresence, PANEL_VARIANTS } from '@/components/ui/motion';
import { UserAvatar } from '@/components/shell/user-avatar';
import { useAuth, type AuthUser } from '@/lib/auth-context';
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
  oidcLinkResult: string | null;
  requestedSection: SettingsSectionId | null;
  returnFocusRef: React.RefObject<HTMLElement | null>;
  user: AuthUser;
};

type SettingsCategoryContentProps = {
  activeSection: SettingsSectionId;
  copyFeedback: ReturnType<typeof useSettingsCopy>['copyFeedback'];
  handleCopy: ReturnType<typeof useSettingsCopy>['handleCopy'];
  oidcLinkResult: string | null;
  onTrackingControllerChange: (controller: TrackingSettingsController | null) => void;
  user: AuthUser;
};

const SETTINGS_SECTIONS = [
  {
    description: '主题与界面偏好',
    icon: Settings,
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
    icon: Layers,
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
  return { ...observedState, activeSection: requestedSection, isDialogOpen: true };
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
  oidcLinkResult,
  onTrackingControllerChange,
  user,
}: SettingsCategoryContentProps) {
  if (activeSection === 'general') {
    return <GeneralSettingsSection />;
  }
  if (isTrackingSettingsSection(activeSection)) {
    return (
      <TrackingSettingsContent
        userId={user.id}
        section={activeSection}
        onControllerChange={onTrackingControllerChange}
      />
    );
  }
  if (activeSection === 'data-sources') {
    return (
      <CnkiSettingsCard userId={user.id} copyFeedback={copyFeedback} handleCopy={handleCopy} />
    );
  }
  if (activeSection === 'account') {
    return (
      <>
        <AccountCard user={user} />
        <AccountSignInCard oidcLinkResult={oidcLinkResult} />
        {user.auth_kind === 'local' && <PasswordCard />}
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
  oidcLinkResult,
  onReplaceSection,
  requestedSection,
  returnFocusRef,
  user,
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
  const activeContentKey = isTrackingSettingsSection(activeSection)
    ? 'tracking-notifications'
    : activeSection;

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
        centerIcon={Settings}
        sidebarFooter={
          <div className="flex min-w-0 items-center gap-3">
            <UserAvatar username={user.username} size="md" />
            <div className="min-w-0">
              <p className="truncate text-sm font-bold">{user.username}</p>
              <p className="text-xs text-muted-foreground">当前登录账号</p>
            </div>
          </div>
        }
        centerSubtitle="个人偏好、文献追踪、数据源与账号安全"
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
        <MotionPresence mode="wait">
          <MotionDiv
            key={activeContentKey}
            data-settings-content-key={activeContentKey}
            variants={PANEL_VARIANTS}
            initial="hidden"
            animate="visible"
            exit="exit"
          >
            <SettingsCategoryContent
              activeSection={activeSection}
              copyFeedback={copyFeedback}
              handleCopy={handleCopy}
              oidcLinkResult={oidcLinkResult}
              onTrackingControllerChange={handleTrackingControllerChange}
              user={user}
            />
          </MotionDiv>
        </MotionPresence>
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
  const { pathname } = useLocation();
  const navigate = useNavigate();
  const [searchParams] = useSearchParams();
  const rawSection = searchParams.get('settings');
  const requestedSection = parseSettingsSection(rawSection);
  const [mountState, setMountState] = useState<SettingsCenterMountState>({
    initialSection: requestedSection ?? 'general',
    isSessionOpen: requestedSection !== null,
    observedRequestedSection: requestedSection,
  });
  const returnFocusRef = useRef<HTMLElement | null>(null);
  const rawOidcLinkResult = searchParams.get('oidc_link');
  const [oidcLinkResult, setOidcLinkResult] = useState<string | null>(null);

  if (requestedSection !== mountState.observedRequestedSection) {
    setMountState(synchronizeMountState(mountState, requestedSection));
  }
  if (rawOidcLinkResult !== null && rawOidcLinkResult !== oidcLinkResult) {
    setOidcLinkResult(rawOidcLinkResult);
  }

  useEffect(() => {
    if (rawOidcLinkResult === null) {
      return;
    }
    const cleanedSearchParams = new URLSearchParams(searchParams.toString());
    cleanedSearchParams.delete('oidc_link');
    navigate(buildSettingsCenterHref(pathname, cleanedSearchParams, 'account'), { replace: true });
  }, [pathname, rawOidcLinkResult, navigate, searchParams]);

  useEffect(() => {
    if (rawSection !== null && requestedSection === null) {
      navigate(buildSettingsCenterHref(pathname, searchParams, null), { replace: true });
    }
  }, [pathname, rawSection, requestedSection, navigate, searchParams]);

  const replaceSection = useCallback(
    (section: SettingsSectionId): void => {
      navigate(buildSettingsCenterHref(pathname, searchParams, section), { replace: true });
    },
    [pathname, navigate, searchParams],
  );

  const removeSection = useCallback((): void => {
    navigate(buildSettingsCenterHref(pathname, searchParams, null), { replace: true });
  }, [pathname, navigate, searchParams]);

  if (!user || !mountState.isSessionOpen) {
    return null;
  }

  return (
    <SettingsCenterSession
      initialSection={mountState.initialSection}
      requestedSection={requestedSection}
      oidcLinkResult={oidcLinkResult}
      user={user}
      returnFocusRef={returnFocusRef}
      onReplaceSection={replaceSection}
      onRemoveSection={removeSection}
      onCloseSession={() => {
        setOidcLinkResult(null);
        setMountState((state) =>
          state.observedRequestedSection === null ? { ...state, isSessionOpen: false } : state,
        );
      }}
    />
  );
}
