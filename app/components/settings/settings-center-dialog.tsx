'use client';

/**
 * Query-driven aggregated settings dialog for every authenticated route.
 */

import {
  Bell,
  Database,
  KeyRound,
  Radar,
  Settings2,
  ShieldCheck,
  type LucideIcon,
} from 'lucide-react';
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
import { Dialog, DialogContent, DialogDescription, DialogTitle } from '@/components/ui/dialog';
import { useAuth } from '@/lib/auth-context';
import {
  buildSettingsCenterHref,
  isTrackingSettingsSection,
  parseSettingsSection,
  type SettingsSectionId,
} from '@/lib/settings-center';
import { cn } from '@/lib/utils';

type SettingsSectionDefinition = {
  description: string;
  icon: LucideIcon;
  id: SettingsSectionId;
  label: string;
};

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

type SettingsCenterNavigationProps = {
  activeSection: SettingsSectionId;
  className?: string;
  isDisabled?: boolean;
  onSelect: (section: SettingsSectionId) => void;
};

type SettingsCategoryContentProps = {
  activeSection: SettingsSectionId;
  copyFeedback: ReturnType<typeof useSettingsCopy>['copyFeedback'];
  handleCopy: ReturnType<typeof useSettingsCopy>['handleCopy'];
  onTrackingControllerChange: (controller: TrackingSettingsController | null) => void;
  userId: number;
  username: string;
};

const SETTINGS_SECTIONS: readonly SettingsSectionDefinition[] = [
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
];

/**
 * Return the metadata for one stable settings section.
 *
 * @param section - Section identifier.
 * @returns Matching settings definition.
 */
function getSettingsSectionDefinition(section: SettingsSectionId): SettingsSectionDefinition {
  return SETTINGS_SECTIONS.find((item) => item.id === section) ?? SETTINGS_SECTIONS[0];
}

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
 * Render the category navigation for desktop or mobile layouts.
 *
 * @param props - Active section, selection action, and layout class.
 * @returns Accessible settings category navigation.
 */
function SettingsCenterNavigation({
  activeSection,
  className,
  isDisabled = false,
  onSelect,
}: SettingsCenterNavigationProps) {
  return (
    <nav aria-label="设置分类" className={className}>
      {SETTINGS_SECTIONS.map((section) => {
        const Icon = section.icon;
        const isActive = section.id === activeSection;
        return (
          <button
            key={section.id}
            type="button"
            aria-current={isActive ? 'page' : undefined}
            disabled={isDisabled}
            className={cn(
              'flex shrink-0 items-center gap-3 rounded-md px-3 py-2.5 text-left text-sm font-medium outline-none transition-colors hover:bg-accent focus-visible:ring-[3px] focus-visible:ring-ring/50 disabled:pointer-events-none disabled:opacity-50',
              isActive && 'bg-accent text-accent-foreground',
            )}
            onClick={() => onSelect(section.id)}
          >
            <Icon className="size-4" />
            <span>{section.label}</span>
          </button>
        );
      })}
    </nav>
  );
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
  const activeDefinition = getSettingsSectionDefinition(activeSection);

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
      <Dialog
        open={isDialogOpen}
        onOpenChange={(open) => {
          if (!open) {
            requestClose();
          }
        }}
      >
        <DialogContent
          aria-busy={isRestoringUrl || undefined}
          className="flex h-dvh w-screen max-w-none translate-x-[-50%] translate-y-[-50%] gap-0 overflow-hidden rounded-none border-0 p-0 shadow-none [&>[data-slot=dialog-close]]:top-5 [&>[data-slot=dialog-close]]:right-5 [&>[data-slot=dialog-close]]:flex [&>[data-slot=dialog-close]]:size-10 [&>[data-slot=dialog-close]]:items-center [&>[data-slot=dialog-close]]:justify-center [&>[data-slot=dialog-close]]:rounded-md [&>[data-slot=dialog-close]]:border [&>[data-slot=dialog-close]]:bg-background [&>[data-slot=dialog-close]]:opacity-100 [&>[data-slot=dialog-close]]:hover:bg-accent md:h-[min(90dvh,52rem)] md:w-[min(calc(100vw-2rem),72rem)] md:max-w-6xl md:rounded-lg md:border md:shadow-lg md:[&>[data-slot=dialog-close]]:right-auto md:[&>[data-slot=dialog-close]]:left-5"
          onOpenAutoFocus={() => {
            if (
              !returnFocusRef.current &&
              document.activeElement instanceof HTMLElement &&
              document.activeElement !== document.body
            ) {
              returnFocusRef.current = document.activeElement;
            }
          }}
          onCloseAutoFocus={(event) => {
            const focusTarget = returnFocusRef.current;
            returnFocusRef.current = null;
            if (focusTarget?.isConnected) {
              event.preventDefault();
              focusTarget.focus();
            }
            onCloseSession();
          }}
          onEscapeKeyDown={(event) => {
            if (pendingTransition) {
              event.preventDefault();
            }
          }}
          onInteractOutside={(event) => {
            if (pendingTransition) {
              event.preventDefault();
            }
          }}
        >
          <DialogTitle className="sr-only">设置中心</DialogTitle>
          <DialogDescription className="sr-only">
            在一个弹窗中管理常规、文献追踪、通知、数据源、账号安全和访问令牌设置。
          </DialogDescription>

          <aside className="hidden w-60 shrink-0 flex-col border-r bg-muted/20 px-3 pb-5 pt-16 md:flex">
            <div className="px-3 pb-5">
              <div className="text-lg font-semibold">设置中心</div>
              <p className="mt-1 text-xs text-muted-foreground">集中管理 LitRadar</p>
            </div>
            <SettingsCenterNavigation
              activeSection={activeSection}
              className="flex flex-col gap-1"
              isDisabled={isRestoringUrl}
              onSelect={requestSection}
            />
          </aside>

          <div className="flex min-w-0 flex-1 flex-col">
            <header className="shrink-0 border-b bg-background px-5 pb-4 pt-5 pr-14 md:px-8 md:py-5 md:pr-8">
              <div className="md:hidden">
                <div className="text-lg font-semibold">设置中心</div>
                <SettingsCenterNavigation
                  activeSection={activeSection}
                  className="-mx-1 mt-4 flex gap-1 overflow-x-auto px-1 pb-1"
                  isDisabled={isRestoringUrl}
                  onSelect={requestSection}
                />
              </div>
              <div className="mt-4 md:mt-0">
                <h2 className="text-xl font-semibold">{activeDefinition.label}</h2>
                <p className="mt-1 text-sm text-muted-foreground">{activeDefinition.description}</p>
              </div>
            </header>

            <div
              role="region"
              aria-label={`${activeDefinition.label}设置内容`}
              className="min-h-0 flex-1 overflow-y-auto px-5 py-6 pb-[calc(1.5rem+var(--safe-area-inset-bottom,env(safe-area-inset-bottom,0px)))] md:px-8"
            >
              <SettingsCategoryContent
                activeSection={activeSection}
                copyFeedback={copyFeedback}
                handleCopy={handleCopy}
                onTrackingControllerChange={handleTrackingControllerChange}
                userId={userId}
                username={username}
              />
            </div>
          </div>
        </DialogContent>
      </Dialog>

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
