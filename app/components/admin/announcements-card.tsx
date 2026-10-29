'use client';

import { useMemo, useState } from 'react';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { BellRing, Megaphone, Pencil, Plus, Trash2 } from 'lucide-react';

import {
  adminCreateAnnouncement,
  adminDeleteAnnouncement,
  adminGetAnnouncements,
  adminUpdateAnnouncement,
  type AnnouncementInfo,
} from '@/lib/api';
import { AdminListHeader } from '@/components/admin/admin-list-header';
import { StatusPill } from '@/components/settings/settings-section';
import { Button } from '@/components/ui/button';
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/components/ui/card';
import { ConfirmDialog } from '@/components/ui/confirm-dialog';
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
  DialogTrigger,
} from '@/components/ui/dialog';
import { Input } from '@/components/ui/input';
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@/components/ui/select';
import { Label } from '@/components/ui/label';
import {
  COLLAPSE_VARIANTS,
  FADE_UP_VARIANTS,
  MOTION_DURATION_SECONDS,
  MotionDiv,
  MotionParagraph,
  MotionPresence,
  useMotionTransition,
} from '@/components/ui/motion';
import { Switch } from '@/components/ui/switch';
import { cn } from '@/lib/utils';

type AnnouncementFormState = {
  enabled: boolean;
  message: string;
  priority: 'high' | 'normal' | 'low';
  title: string;
};

const DEFAULT_FORM: AnnouncementFormState = {
  enabled: true,
  message: '',
  priority: 'normal',
  title: '',
};

const PRIORITY_LABELS = {
  high: '高优先级',
  low: '低优先级',
  normal: '普通',
} as const;

const PRIORITY_TONES = {
  high: 'danger',
  low: 'neutral',
  normal: 'info',
} as const;

const PRIORITY_ACCENT_CLASSES = {
  high: 'bg-destructive',
  low: 'bg-muted-foreground/40',
  normal: 'bg-primary',
} as const;

function formatDateTime(value: number): string {
  return new Date(value * 1000).toLocaleString('zh-CN', {
    hour: '2-digit',
    minute: '2-digit',
    month: '2-digit',
    day: '2-digit',
    year: 'numeric',
  });
}

/** Manage announcements with the existing server-authoritative mutations. */
export function AnnouncementsCard() {
  const state = useAnnouncementsViewState();
  const { error, isLoading } = state;
  return (
    <Card>
      <CardHeader>
        <CardTitle className="flex items-center gap-2">
          <BellRing className="h-5 w-5" />
          系统公告
        </CardTitle>
        <CardDescription>管理首页自动弹出的全局公告</CardDescription>
      </CardHeader>
      <CardContent className="space-y-3">
        <AdminListHeader
          summary={`共 ${state.announcements.length} 条公告 · ${
            state.announcements.filter((announcement) => announcement.enabled).length
          } 条展示中`}
        >
          {renderAnnouncementEditor(state)}
        </AdminListHeader>

        {error instanceof Error && (
          <p role="alert" className="text-sm text-destructive">
            {error.message}
          </p>
        )}

        {isLoading ? (
          <p role="status" className="text-sm text-muted-foreground">
            加载中…
          </p>
        ) : (
          <div className="border-t">{renderAnnouncementList(state)}</div>
        )}
        {renderAnnouncementDeletion(state)}
      </CardContent>
    </Card>
  );
}
/** Own the original hooks, draft stores and exact completion/cache order. */
function useAnnouncementsViewState() {
  const queryClient = useQueryClient();
  const [dialogOpen, setDialogOpen] = useState(false);
  const [editingAnnouncement, setEditingAnnouncement] = useState<AnnouncementInfo | null>(null);
  const [announcementToDelete, setAnnouncementToDelete] = useState<AnnouncementInfo | null>(null);
  const [form, setForm] = useState<AnnouncementFormState>(DEFAULT_FORM);
  const feedbackTransition = useMotionTransition(MOTION_DURATION_SECONDS.fast);
  const rowTransition = useMotionTransition(MOTION_DURATION_SECONDS.base);

  const {
    data: announcements = [],
    error,
    isLoading,
  } = useQuery({
    queryKey: ['admin-announcements'],
    queryFn: () => adminGetAnnouncements(),
  });

  const saveMutation = useMutation({
    mutationFn: async () => {
      if (editingAnnouncement) {
        return adminUpdateAnnouncement(editingAnnouncement.id, form);
      }
      return adminCreateAnnouncement(form);
    },
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['admin-announcements'] });
      queryClient.invalidateQueries({ queryKey: ['announcements'] });
      queryClient.invalidateQueries({ queryKey: ['admin-stats'] });
      setDialogOpen(false);
      setEditingAnnouncement(null);
      setForm(DEFAULT_FORM);
    },
  });

  const toggleMutation = useMutation({
    mutationFn: ({ announcementId, enabled }: { announcementId: number; enabled: boolean }) =>
      adminUpdateAnnouncement(announcementId, { enabled }),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['admin-announcements'] });
      queryClient.invalidateQueries({ queryKey: ['announcements'] });
      queryClient.invalidateQueries({ queryKey: ['admin-stats'] });
    },
  });

  const deleteMutation = useMutation({
    mutationFn: (announcementId: number) => adminDeleteAnnouncement(announcementId),
    onSuccess: (_data, announcementId) => {
      queryClient.invalidateQueries({ queryKey: ['admin-announcements'] });
      queryClient.invalidateQueries({ queryKey: ['announcements'] });
      queryClient.invalidateQueries({ queryKey: ['admin-stats'] });
      setAnnouncementToDelete((current) => (current?.id === announcementId ? null : current));
    },
  });

  const mutationError = useMemo(() => {
    if (saveMutation.error instanceof Error) {
      return saveMutation.error.message;
    }
    if (toggleMutation.error instanceof Error) {
      return toggleMutation.error.message;
    }
    if (deleteMutation.error instanceof Error) {
      return deleteMutation.error.message;
    }
    return '';
  }, [deleteMutation.error, saveMutation.error, toggleMutation.error]);

  const openCreateDialog = () => {
    setEditingAnnouncement(null);
    setForm(DEFAULT_FORM);
    setDialogOpen(true);
  };

  const openEditDialog = (announcement: AnnouncementInfo) => {
    setEditingAnnouncement(announcement);
    setForm({
      enabled: announcement.enabled,
      message: announcement.message,
      priority: announcement.priority,
      title: announcement.title,
    });
    setDialogOpen(true);
  };

  return {
    dialogOpen,
    setDialogOpen,
    editingAnnouncement,
    announcementToDelete,
    setAnnouncementToDelete,
    form,
    setForm,
    feedbackTransition,
    rowTransition,
    announcements,
    error,
    isLoading,
    saveMutation,
    toggleMutation,
    deleteMutation,
    mutationError,
    openCreateDialog,
    openEditDialog,
  };
}
type AnnouncementsViewState = ReturnType<typeof useAnnouncementsViewState>;

/** Retain raw announcement fields, admission guards and dialog ownership. */
function renderAnnouncementEditor(state: AnnouncementsViewState) {
  const {
    dialogOpen,
    setDialogOpen,
    editingAnnouncement,
    form,
    setForm,
    feedbackTransition,
    saveMutation,
    mutationError,
    openCreateDialog,
  } = state;

  return (
    <Dialog open={dialogOpen} onOpenChange={setDialogOpen}>
      <DialogTrigger asChild>
        <Button size="sm" onClick={openCreateDialog}>
          <Plus className="size-4" />
          新建公告
        </Button>
      </DialogTrigger>
      <DialogContent className="max-h-[90vh] overflow-y-auto sm:max-w-2xl">
        <DialogHeader>
          <DialogTitle>{editingAnnouncement ? '编辑公告' : '新建公告'}</DialogTitle>
          <DialogDescription>公告会在首页顶部轮询显示，用户可单独关闭。</DialogDescription>
        </DialogHeader>
        <div className="space-y-4 py-2">
          <div className="space-y-2">
            <Label htmlFor="announcement-title">公告标题</Label>
            <Input
              id="announcement-title"
              name="announcement_title"
              autoComplete="off"
              value={form.title}
              onChange={(event) =>
                setForm((current) => ({ ...current, title: event.target.value }))
              }
              placeholder="公告标题"
            />
          </div>
          <div className="space-y-2">
            <Label htmlFor="announcement-priority">优先级</Label>
            <Select
              value={form.priority}
              onValueChange={(value: 'high' | 'normal' | 'low') =>
                setForm((current) => ({ ...current, priority: value }))
              }
            >
              <SelectTrigger id="announcement-priority" className="w-full">
                <SelectValue placeholder="选择优先级" />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="high">高优先级</SelectItem>
                <SelectItem value="normal">普通</SelectItem>
                <SelectItem value="low">低优先级</SelectItem>
              </SelectContent>
            </Select>
          </div>
          <div className="space-y-2">
            <Label htmlFor="announcement-message">公告内容</Label>
            <textarea
              id="announcement-message"
              name="announcement_message"
              autoComplete="off"
              value={form.message}
              onChange={(event) =>
                setForm((current) => ({ ...current, message: event.target.value }))
              }
              placeholder="公告内容"
              className="min-h-32 w-full rounded-lg border border-input bg-transparent px-3 py-2 text-sm outline-none placeholder:text-muted-foreground focus-visible:border-ring focus-visible:ring-[3px] focus-visible:ring-ring/50"
            />
          </div>
          <div className="flex items-center justify-between gap-3 rounded-lg bg-muted/60 px-3 py-3">
            <Label htmlFor="announcement-enabled" className="text-sm">
              启用公告
            </Label>
            <Switch
              id="announcement-enabled"
              checked={form.enabled}
              onCheckedChange={(checked: boolean) =>
                setForm((current) => ({ ...current, enabled: checked }))
              }
            />
          </div>
          <MotionPresence>
            {mutationError && (
              <MotionParagraph
                key="announcement-error"
                data-motion-feedback="announcement"
                role="alert"
                className="text-sm text-destructive"
                initial="hidden"
                animate="visible"
                exit="exit"
                variants={FADE_UP_VARIANTS}
                transition={feedbackTransition}
              >
                {mutationError}
              </MotionParagraph>
            )}
          </MotionPresence>
          <div className="flex flex-col-reverse gap-2 sm:flex-row sm:justify-end">
            <Button
              variant="outline"
              className="w-full sm:w-auto"
              onClick={() => setDialogOpen(false)}
            >
              取消
            </Button>
            <Button
              className="w-full sm:w-auto"
              disabled={!form.title.trim() || !form.message.trim() || saveMutation.isPending}
              onClick={() => saveMutation.mutate()}
            >
              {editingAnnouncement ? '保存' : '创建'}
            </Button>
          </div>
        </div>
      </DialogContent>
    </Dialog>
  );
}

/** Retain empty feedback and directly keyed announcement controls. */
function renderAnnouncementList(state: AnnouncementsViewState) {
  const {
    setAnnouncementToDelete,
    feedbackTransition,
    rowTransition,
    announcements,
    toggleMutation,
    deleteMutation,
    openEditDialog,
  } = state;

  return (
    <MotionPresence>
      {announcements.length === 0 ? (
        <MotionDiv
          key="empty-announcements"
          className="flex flex-col items-center gap-3 px-6 py-12 text-center"
          initial="hidden"
          animate="visible"
          exit="exit"
          variants={FADE_UP_VARIANTS}
          transition={feedbackTransition}
        >
          <span className="flex size-12 items-center justify-center rounded-full bg-muted text-muted-foreground">
            <Megaphone className="size-5" aria-hidden="true" />
          </span>
          <p className="text-sm text-muted-foreground">暂无公告</p>
        </MotionDiv>
      ) : (
        announcements.map((announcement) => (
          <MotionDiv
            key={announcement.id}
            data-motion-announcement-key={announcement.id}
            className={cn(
              'relative overflow-hidden border-b',
              !announcement.enabled && 'opacity-70',
            )}
            initial="hidden"
            animate="visible"
            exit={{ height: 0, opacity: 0, pointerEvents: 'none' }}
            variants={COLLAPSE_VARIANTS}
            transition={rowTransition}
          >
            <span
              aria-hidden="true"
              className={cn(
                'absolute inset-y-0 left-0 w-1',
                PRIORITY_ACCENT_CLASSES[announcement.priority],
              )}
            />
            <div className="space-y-2 px-5 pt-4 pb-3">
              <div className="flex flex-wrap items-center gap-2">
                <StatusPill tone={PRIORITY_TONES[announcement.priority]}>
                  {PRIORITY_LABELS[announcement.priority]}
                </StatusPill>
                <span className="text-xs text-muted-foreground tabular-nums">
                  更新于 {formatDateTime(announcement.updated_at)}
                </span>
              </div>
              <div className="text-[15px] leading-6 font-bold break-all">{announcement.title}</div>
              <p className="text-sm leading-relaxed text-muted-foreground break-words whitespace-pre-wrap">
                {announcement.message}
              </p>
            </div>
            <div className="flex items-center gap-2 px-5 pb-3">
              <Switch
                checked={announcement.enabled}
                aria-label={`${announcement.enabled ? '停用' : '启用'}公告 ${announcement.title}`}
                onCheckedChange={(checked: boolean) =>
                  toggleMutation.mutate({
                    announcementId: announcement.id,
                    enabled: checked,
                  })
                }
              />
              <span aria-hidden="true" className="text-xs text-muted-foreground">
                {announcement.enabled ? '展示中' : '已停用'}
              </span>
              <div className="ml-auto flex items-center gap-1">
                <Button
                  variant="ghost"
                  size="icon"
                  className="size-9"
                  aria-label={`编辑公告 ${announcement.title}`}
                  onClick={() => openEditDialog(announcement)}
                >
                  <Pencil className="size-4" />
                </Button>
                <Button
                  variant="ghost"
                  size="icon"
                  className="size-9 text-muted-foreground hover:bg-destructive/10 hover:text-destructive"
                  aria-label={`删除公告 ${announcement.title}`}
                  disabled={deleteMutation.isPending}
                  onClick={() => {
                    deleteMutation.reset();
                    setAnnouncementToDelete(announcement);
                  }}
                >
                  <Trash2 className="size-4" />
                </Button>
              </div>
            </div>
          </MotionDiv>
        ))
      )}
    </MotionPresence>
  );
}

/** Retain captured target confirmation and pending dismissal safeguards. */
function renderAnnouncementDeletion(
  state: Pick<
    AnnouncementsViewState,
    'announcementToDelete' | 'setAnnouncementToDelete' | 'deleteMutation'
  >,
) {
  const { announcementToDelete, setAnnouncementToDelete, deleteMutation } = state;

  return (
    <ConfirmDialog
      open={announcementToDelete !== null}
      onOpenChange={(nextOpen) => {
        if (!nextOpen && !deleteMutation.isPending) {
          setAnnouncementToDelete(null);
        }
      }}
      title="删除公告？"
      description={`确认删除公告“${announcementToDelete?.title ?? ''}”？`}
      actionLabel="确认删除"
      pendingLabel="删除中…"
      isPending={deleteMutation.isPending}
      error={deleteMutation.error instanceof Error ? deleteMutation.error.message : null}
      onConfirm={() => {
        if (announcementToDelete) {
          deleteMutation.mutate(announcementToDelete.id);
        }
      }}
    />
  );
}
