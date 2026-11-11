/**
 * Query-driven administrator dialog for authenticated administrators.
 */

import {
  BellRing,
  Clock3,
  Gift,
  Layers,
  ShieldCheck,
  SlidersVertical,
  Users,
} from '@/components/ui/animated-icons';
import { Tabs } from '@animate-ui/react/primitives/base-ui/tabs';
import { useSearch } from '@tanstack/react-router';
import { useCallback, useEffect, useRef, useState, type ReactNode } from 'react';

import { AnnouncementsCard } from '@/components/admin/announcements-card';
import { AdminInviteCodesCard } from '@/components/admin/invite-codes-card';
import { LoadTelemetryCard } from '@/components/admin/load-telemetry-card';
import { AdminOverviewCard } from '@/components/admin/overview-card';
import { RuntimeSettingsCard } from '@/components/admin/runtime-settings-card';
import { ScheduledTasksCard } from '@/components/admin/scheduled-tasks-card';
import { AdminUsersCard } from '@/components/admin/users-card';
import {
  SectionedDialogFrame,
  type SectionedDialogSectionDefinition,
} from '@/components/feature/sectioned-dialog';
import { MotionSection, PANEL_VARIANTS } from '@/components/ui/motion';
import { parseAdminSection, withAdminSection, type AdminSectionId } from '@/lib/admin-center';
import { useUpdateWorkspaceSearch, WORKSPACE_ROUTE_ID } from '@/lib/search-params';
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
  section: AdminSectionId;
};

const ADMIN_SECTIONS = [
  {
    description: '系统统计与服务状态',
    icon: ShieldCheck,
    id: 'overview',
    label: '概览',
  },
  {
    description: '接口负载、延迟与检索形态',
    icon: Layers,
    id: 'load',
    label: '负载',
  },
  {
    description: '账号、权限与密码维护',
    icon: Users,
    id: 'users',
    label: '用户',
  },
  {
    description: '注册邀请码创建与回收',
    icon: Gift,
    id: 'invite-codes',
    label: '邀请码',
  },
  {
    description: 'Provider、来源与服务配置',
    icon: SlidersVertical,
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
 * Mount administrator data on its first activation and retain visited panels across navigation.
 *
 * @param props - Stable panel content, active state, and section id.
 * @returns Mounted tab panel, named by its tab, with entry-only visual motion.
 */
function AdminPanel({ children, isActive, section }: AdminPanelProps) {
  const [hasVisited, setHasVisited] = useState(isActive);
  if (isActive && !hasVisited) {
    setHasVisited(true);
  }
  return (
    <Tabs.Panel
      value={section}
      keepMounted
      data-admin-panel-state={isActive ? 'active' : 'inactive'}
      render={
        <MotionSection
          variants={PANEL_VARIANTS}
          initial={false}
          animate={isActive ? 'visible' : 'hidden'}
        />
      }
    >
      {hasVisited ? children : null}
    </Tabs.Panel>
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
  const search = useSearch({ from: WORKSPACE_ROUTE_ID });
  const updateSearch = useUpdateWorkspaceSearch();
  const rawAdminSection = search.admin;
  const parsedAdminSection = parseAdminSection(rawAdminSection);
  const hasSettingsConflict = parseSettingsSection(search.settings) !== null;
  const requestedSection = user?.is_admin && !hasSettingsConflict ? parsedAdminSection : null;
  const [sessionState, setSessionState] = useState<AdminCenterSessionState>({
    activeSection: requestedSection ?? 'overview',
    isDialogOpen: requestedSection !== null,
    isSessionMounted: requestedSection !== null,
    observedRequestedSection: requestedSection,
  });
  const returnFocusRef = useRef<HTMLElement | null>(null);

  if (requestedSection !== sessionState.observedRequestedSection) {
    setSessionState(synchronizeAdminCenterState(sessionState, requestedSection));
  }

  useEffect(() => {
    if (
      rawAdminSection !== undefined &&
      (!user?.is_admin || parsedAdminSection === null || hasSettingsConflict)
    ) {
      void updateSearch((current) => withAdminSection(current, null));
    }
  }, [hasSettingsConflict, parsedAdminSection, rawAdminSection, updateSearch, user?.is_admin]);

  const replaceSection = useCallback(
    (section: AdminSectionId): void => {
      void updateSearch((current) => withAdminSection(current, section));
    },
    [updateSearch],
  );

  /** Close the current administrator session and preserve unrelated workspace query state. */
  function closeCenter(): void {
    void updateSearch((current) => withAdminSection(current, null));
    setSessionState((state) => ({ ...state, isDialogOpen: false }));
  }

  if (!user?.is_admin || !sessionState.isSessionMounted) {
    return null;
  }

  const { activeSection, isDialogOpen } = sessionState;

  return (
    <SectionedDialogFrame
      activeSection={activeSection}
      centerIcon={ShieldCheck}
      contentWidth="wide"
      centerSubtitle="负载、用户、邀请、运行配置、计划任务与公告"
      centerTitle="管理面板"
      contentLabelSuffix="管理内容"
      dialogDescription="在一个弹窗中管理系统概览、负载统计、用户、邀请码、运行配置、计划任务和公告。"
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
      <AdminPanel section="overview" isActive={activeSection === 'overview'}>
        <AdminOverviewCard isEnabled />
      </AdminPanel>
      <AdminPanel section="load" isActive={activeSection === 'load'}>
        <LoadTelemetryCard isEnabled />
      </AdminPanel>
      <AdminPanel section="users" isActive={activeSection === 'users'}>
        <AdminUsersCard currentUserId={user.id} isEnabled />
      </AdminPanel>
      <AdminPanel section="invite-codes" isActive={activeSection === 'invite-codes'}>
        <AdminInviteCodesCard isEnabled />
      </AdminPanel>
      <AdminPanel section="runtime-settings" isActive={activeSection === 'runtime-settings'}>
        <RuntimeSettingsCard />
      </AdminPanel>
      <AdminPanel section="scheduled-tasks" isActive={activeSection === 'scheduled-tasks'}>
        <ScheduledTasksCard isActive={isDialogOpen && activeSection === 'scheduled-tasks'} />
      </AdminPanel>
      <AdminPanel section="announcements" isActive={activeSection === 'announcements'}>
        <AnnouncementsCard />
      </AdminPanel>
    </SectionedDialogFrame>
  );
}
