'use client';

/**
 * Query-driven administrator dialog for authenticated administrators.
 */

import { BellRing, Clock3, DatabaseZap, Shield, Ticket, Users } from 'lucide-react';
import { usePathname, useRouter, useSearchParams } from 'next/navigation';
import { useCallback, useEffect, useRef, useState, type ReactNode } from 'react';

import { AnnouncementsCard } from '@/components/admin/announcements-card';
import { AdminInviteCodesCard } from '@/components/admin/invite-codes-card';
import { AdminOverviewCard } from '@/components/admin/overview-card';
import { RuntimeSettingsCard } from '@/components/admin/runtime-settings-card';
import { ScheduledTasksCard } from '@/components/admin/scheduled-tasks-card';
import { AdminUsersCard } from '@/components/admin/users-card';
import {
  SectionedDialogFrame,
  type SectionedDialogSectionDefinition,
} from '@/components/feature/sectioned-dialog';
import {
  FADE_UP_VARIANTS,
  MOTION_DURATION_SECONDS,
  MotionSection,
  useMotionTransition,
} from '@/components/ui/motion';
import { buildAdminCenterHref, parseAdminSection, type AdminSectionId } from '@/lib/admin-center';
import { useAuth } from '@/lib/auth-context';
import { parseSettingsSection } from '@/lib/settings-center';

type AdminCenterSessionState = {
  activeSection: AdminSectionId;
  isDialogOpen: boolean;
  isSessionMounted: boolean;
  observedRequestedSection: AdminSectionId | null;
};

type AdminPanelProps = {
  children: ReactNode;
  isActive: boolean;
  label: string;
  transition: ReturnType<typeof useMotionTransition>;
};

const ADMIN_SECTIONS = [
  {
    description: '系统统计与服务状态',
    icon: Shield,
    id: 'overview',
    label: '概览',
  },
  {
    description: '账号、权限与密码维护',
    icon: Users,
    id: 'users',
    label: '用户',
  },
  {
    description: '注册邀请码创建与回收',
    icon: Ticket,
    id: 'invite-codes',
    label: '邀请码',
  },
  {
    description: 'Provider、来源与服务配置',
    icon: DatabaseZap,
    id: 'runtime-settings',
    label: '运行配置',
  },
  {
    description: '定时任务与调度器状态',
    icon: Clock3,
    id: 'scheduled-tasks',
    label: '计划任务',
  },
  {
    description: '站内公告发布与维护',
    icon: BellRing,
    id: 'announcements',
    label: '公告',
  },
] satisfies readonly [
  SectionedDialogSectionDefinition<AdminSectionId>,
  ...SectionedDialogSectionDefinition<AdminSectionId>[],
];

/**
 * Keep one administrator data owner mounted while animating only its next activation.
 *
 * @param props - Stable panel content, active state, accessible label, and motion timing.
 * @returns Mounted tab panel with entry-only visual motion.
 */
function AdminPanel({ children, isActive, label, transition }: AdminPanelProps) {
  return (
    <MotionSection
      role="tabpanel"
      aria-label={label}
      data-admin-panel-state={isActive ? 'active' : 'inactive'}
      hidden={!isActive}
      variants={FADE_UP_VARIANTS}
      initial={false}
      animate={isActive ? 'visible' : 'hidden'}
      transition={transition}
    >
      {children}
    </MotionSection>
  );
}

/**
 * Reconcile retained administrator panels with the current URL request.
 *
 * @param state - Current administrator dialog session.
 * @param requestedSection - Authorized section requested by the URL.
 * @returns Session state matching the new URL without unmounting panels during closure.
 */
function synchronizeAdminCenterState(
  state: AdminCenterSessionState,
  requestedSection: AdminSectionId | null,
): AdminCenterSessionState {
  if (requestedSection) {
    return {
      activeSection: requestedSection,
      isDialogOpen: true,
      isSessionMounted: true,
      observedRequestedSection: requestedSection,
    };
  }
  return {
    ...state,
    isDialogOpen: false,
    observedRequestedSection: null,
  };
}

/**
 * Mount the administrator center from authorized protected-route query state.
 *
 * @returns Global administrator dialog or null when closed or unauthorized.
 */
export function AdminCenterDialog() {
  const { user } = useAuth();
  const pathname = usePathname();
  const router = useRouter();
  const searchParams = useSearchParams();
  const rawAdminSection = searchParams.get('admin');
  const parsedAdminSection = parseAdminSection(rawAdminSection);
  const hasSettingsConflict = parseSettingsSection(searchParams.get('settings')) !== null;
  const requestedSection = user?.is_admin && !hasSettingsConflict ? parsedAdminSection : null;
  const [sessionState, setSessionState] = useState<AdminCenterSessionState>({
    activeSection: requestedSection ?? 'overview',
    isDialogOpen: requestedSection !== null,
    isSessionMounted: requestedSection !== null,
    observedRequestedSection: requestedSection,
  });
  const returnFocusRef = useRef<HTMLElement | null>(null);
  const panelTransition = useMotionTransition(MOTION_DURATION_SECONDS.base);

  if (requestedSection !== sessionState.observedRequestedSection) {
    setSessionState(synchronizeAdminCenterState(sessionState, requestedSection));
  }

  useEffect(() => {
    if (
      rawAdminSection !== null &&
      (!user?.is_admin || parsedAdminSection === null || hasSettingsConflict)
    ) {
      router.replace(buildAdminCenterHref(pathname, searchParams, null), { scroll: false });
    }
  }, [
    hasSettingsConflict,
    parsedAdminSection,
    pathname,
    rawAdminSection,
    router,
    searchParams,
    user?.is_admin,
  ]);

  const replaceSection = useCallback(
    (section: AdminSectionId): void => {
      window.history.replaceState(null, '', buildAdminCenterHref(pathname, searchParams, section));
    },
    [pathname, searchParams],
  );

  /** Close the current administrator session and preserve unrelated workspace query state. */
  function closeCenter(): void {
    router.replace(buildAdminCenterHref(pathname, searchParams, null), { scroll: false });
    setSessionState((state) => ({ ...state, isDialogOpen: false }));
  }

  if (!user?.is_admin || !sessionState.isSessionMounted) {
    return null;
  }

  const { activeSection, isDialogOpen } = sessionState;

  return (
    <SectionedDialogFrame
      activeSection={activeSection}
      centerSubtitle="集中管理 LitRadar"
      centerTitle="管理面板"
      contentLabelSuffix="管理内容"
      dialogDescription="在一个弹窗中管理系统概览、用户、邀请码、运行配置、计划任务和公告。"
      navigationLabel="管理分类"
      open={isDialogOpen}
      onOpenChange={(open) => {
        if (!open) {
          closeCenter();
        }
      }}
      onSelectSection={replaceSection}
      onSessionClosed={() =>
        setSessionState((state) => ({
          ...state,
          isSessionMounted: false,
        }))
      }
      returnFocusRef={returnFocusRef}
      sections={ADMIN_SECTIONS}
    >
      <AdminPanel
        isActive={activeSection === 'overview'}
        label="概览面板"
        transition={panelTransition}
      >
        <AdminOverviewCard isEnabled />
      </AdminPanel>
      <AdminPanel
        isActive={activeSection === 'users'}
        label="用户面板"
        transition={panelTransition}
      >
        <AdminUsersCard currentUserId={user.id} isEnabled />
      </AdminPanel>
      <AdminPanel
        isActive={activeSection === 'invite-codes'}
        label="邀请码面板"
        transition={panelTransition}
      >
        <AdminInviteCodesCard isEnabled />
      </AdminPanel>
      <AdminPanel
        isActive={activeSection === 'runtime-settings'}
        label="运行配置面板"
        transition={panelTransition}
      >
        <RuntimeSettingsCard />
      </AdminPanel>
      <AdminPanel
        isActive={activeSection === 'scheduled-tasks'}
        label="计划任务面板"
        transition={panelTransition}
      >
        <ScheduledTasksCard />
      </AdminPanel>
      <AdminPanel
        isActive={activeSection === 'announcements'}
        label="公告面板"
        transition={panelTransition}
      >
        <AnnouncementsCard />
      </AdminPanel>
    </SectionedDialogFrame>
  );
}
