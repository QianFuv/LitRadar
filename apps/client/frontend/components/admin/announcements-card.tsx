import { useMemo, useState } from 'react';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { BellRing, Megaphone, Pencil, Plus, Trash2 } from '@/components/ui/animated-icons';

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
  DISCLOSURE_VARIANTS,
  MotionDiv,
  MotionParagraph,
  MotionPresence,
  REVEAL_VARIANTS,
} from '@/components/ui/motion';
import { Switch } from '@/components/ui/switch';
import { RevealItem } from '@/components/ui/reveal';
import { LoadingText } from '@/components/ui/loading-text';
import * as stylex from '@stylexjs/stylex';
import { colors, lineHeights } from '@/lib/theme.stylex';

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

/** Priority choices in the editor, from most to least urgent. */
const PRIORITY_ITEMS = [
  { value: 'high', label: PRIORITY_LABELS.high },
  { value: 'normal', label: PRIORITY_LABELS.normal },
  { value: 'low', label: PRIORITY_LABELS.low },
] as const;

const PRIORITY_TONES = {
  high: 'danger',
  low: 'neutral',
  normal: 'info',
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
        <CardTitle xstyle={styles.announcementsCardCardTitle}>
          <BellRing xstyle={styles.announcementsCardBellRing} />
          系统公告
        </CardTitle>
        <CardDescription>管理首页自动弹出的全局公告</CardDescription>
      </CardHeader>
      <CardContent xstyle={styles.announcementsCardCardContent}>
        <AdminListHeader
          summary={`共 ${state.announcements.length} 条公告 · ${
            state.announcements.filter((announcement) => announcement.enabled).length
          } 条展示中`}
        >
          {renderAnnouncementEditor(state)}
        </AdminListHeader>

        {error instanceof Error && (
          <p role="alert" {...stylex.props(styles.announcementsCardP)}>
            {error.message}
          </p>
        )}

        <RevealItem
          isReady={!isLoading}
          xstyle={styles.announcementsCardRevealItem}
          placeholderXstyle={null}
          placeholder={
            <output {...stylex.props(styles.statusOutput, styles.announcementsCardP2)}>
              <LoadingText text="加载中…" />
            </output>
          }
        >
          {renderAnnouncementList(state)}
        </RevealItem>
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
      void queryClient.invalidateQueries({ queryKey: ['admin-announcements'] });
      void queryClient.invalidateQueries({ queryKey: ['announcements'] });
      void queryClient.invalidateQueries({ queryKey: ['admin-stats'] });
      setDialogOpen(false);
      setEditingAnnouncement(null);
      setForm(DEFAULT_FORM);
    },
  });

  const toggleMutation = useMutation({
    mutationFn: ({ announcementId, enabled }: { announcementId: number; enabled: boolean }) =>
      adminUpdateAnnouncement(announcementId, { enabled }),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: ['admin-announcements'] });
      void queryClient.invalidateQueries({ queryKey: ['announcements'] });
      void queryClient.invalidateQueries({ queryKey: ['admin-stats'] });
    },
  });

  const deleteMutation = useMutation({
    mutationFn: (announcementId: number) => adminDeleteAnnouncement(announcementId),
    onSuccess: (_data, announcementId) => {
      void queryClient.invalidateQueries({ queryKey: ['admin-announcements'] });
      void queryClient.invalidateQueries({ queryKey: ['announcements'] });
      void queryClient.invalidateQueries({ queryKey: ['admin-stats'] });
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
    saveMutation,
    mutationError,
    openCreateDialog,
  } = state;

  return (
    <Dialog open={dialogOpen} onOpenChange={setDialogOpen}>
      <DialogTrigger render={<Button size="sm" onClick={openCreateDialog} />}>
        <Plus xstyle={styles.renderAnnouncementEditorPlus} />
        新建公告
      </DialogTrigger>
      <DialogContent xstyle={styles.renderAnnouncementEditorDialogContent}>
        <DialogHeader>
          <DialogTitle>{editingAnnouncement ? '编辑公告' : '新建公告'}</DialogTitle>
          <DialogDescription>公告会在首页顶部轮询显示，用户可单独关闭。</DialogDescription>
        </DialogHeader>
        <div {...stylex.props(styles.renderAnnouncementEditorDiv)}>
          <div>
            <Label xstyle={styles.renderAnnouncementEditorDivItem} htmlFor="announcement-title">
              公告标题
            </Label>
            <Input
              xstyle={styles.renderAnnouncementEditorDivItem}
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
          <div {...stylex.props(styles.renderAnnouncementEditorDiv2)}>
            <Label htmlFor="announcement-priority">优先级</Label>
            <Select
              value={form.priority}
              items={PRIORITY_ITEMS}
              onValueChange={(value) => setForm((current) => ({ ...current, priority: value }))}
            >
              <SelectTrigger
                id="announcement-priority"
                xstyle={styles.renderAnnouncementEditorSelectTrigger}
              >
                <SelectValue placeholder="选择优先级" />
              </SelectTrigger>
              <SelectContent>
                {PRIORITY_ITEMS.map((item) => (
                  <SelectItem key={item.value} value={item.value}>
                    {item.label}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>
          <div>
            <Label xstyle={styles.renderAnnouncementEditorDivItem} htmlFor="announcement-message">
              公告内容
            </Label>
            <textarea
              id="announcement-message"
              name="announcement_message"
              autoComplete="off"
              value={form.message}
              onChange={(event) =>
                setForm((current) => ({ ...current, message: event.target.value }))
              }
              placeholder="公告内容"
              {...stylex.props(
                styles.renderAnnouncementEditorDivItem,
                styles.renderAnnouncementEditorTextarea,
              )}
            />
          </div>
          <div {...stylex.props(styles.renderAnnouncementEditorDiv3)}>
            <Label htmlFor="announcement-enabled" xstyle={styles.renderAnnouncementEditorLabel}>
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
                {...stylex.props(styles.announcementsCardP)}
                initial="hidden"
                animate="visible"
                exit="exit"
                variants={REVEAL_VARIANTS}
              >
                {mutationError}
              </MotionParagraph>
            )}
          </MotionPresence>
          <div {...stylex.props(styles.renderAnnouncementEditorDiv4)}>
            <Button
              variant="outline"
              xstyle={styles.renderAnnouncementEditorButton}
              onClick={() => setDialogOpen(false)}
            >
              取消
            </Button>
            <Button
              xstyle={styles.renderAnnouncementEditorButton}
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
  const { setAnnouncementToDelete, announcements, toggleMutation, deleteMutation, openEditDialog } =
    state;

  return (
    <MotionPresence>
      {announcements.length === 0 ? (
        <MotionDiv
          key="empty-announcements"
          {...stylex.props(styles.renderAnnouncementListMotionDiv)}
          initial="hidden"
          animate="visible"
          exit="exit"
          variants={REVEAL_VARIANTS}
        >
          <span {...stylex.props(styles.renderAnnouncementListSpan)}>
            <Megaphone xstyle={styles.renderAnnouncementListMegaphone} aria-hidden="true" />
          </span>
          <p {...stylex.props(styles.announcementsCardP2)}>暂无公告</p>
        </MotionDiv>
      ) : (
        announcements.map((announcement) => (
          <MotionDiv
            key={announcement.id}
            data-motion-announcement-key={announcement.id}
            {...stylex.props(
              styles.renderAnnouncementListMotionDiv2,
              !announcement.enabled && styles.renderAnnouncementListMotionDivDisabled,
            )}
            initial="hidden"
            animate="visible"
            exit="exit"
            variants={DISCLOSURE_VARIANTS}
          >
            <span
              aria-hidden="true"
              {...stylex.props(
                styles.priorityAccent,
                PRIORITY_ACCENT_STYLES[announcement.priority],
              )}
            />
            <div {...stylex.props(styles.renderAnnouncementListDiv)}>
              <div
                {...stylex.props(
                  styles.renderAnnouncementEditorDivItem,
                  styles.renderAnnouncementListDiv2,
                )}
              >
                <StatusPill tone={PRIORITY_TONES[announcement.priority]}>
                  {PRIORITY_LABELS[announcement.priority]}
                </StatusPill>
                <span {...stylex.props(styles.renderAnnouncementListSpan3)}>
                  更新于 {formatDateTime(announcement.updated_at)}
                </span>
              </div>
              <div
                {...stylex.props(
                  styles.renderAnnouncementEditorDivItem,
                  styles.renderAnnouncementListDiv3,
                )}
              >
                {announcement.title}
              </div>
              <p
                {...stylex.props(
                  styles.renderAnnouncementEditorDivItem,
                  styles.renderAnnouncementListP,
                )}
              >
                {announcement.message}
              </p>
            </div>
            <div {...stylex.props(styles.renderAnnouncementListDiv4)}>
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
              <span aria-hidden="true" {...stylex.props(styles.renderAnnouncementListSpan4)}>
                {announcement.enabled ? '展示中' : '已停用'}
              </span>
              <div {...stylex.props(styles.renderAnnouncementListDiv5)}>
                <Button
                  variant="ghost"
                  size="icon"
                  xstyle={styles.renderAnnouncementListButton}
                  aria-label={`编辑公告 ${announcement.title}`}
                  onClick={() => openEditDialog(announcement)}
                >
                  <Pencil xstyle={styles.renderAnnouncementEditorPlus} />
                </Button>
                <Button
                  variant="ghost"
                  size="icon"
                  xstyle={styles.renderAnnouncementListButton2}
                  aria-label={`删除公告 ${announcement.title}`}
                  disabled={deleteMutation.isPending}
                  onClick={() => {
                    deleteMutation.reset();
                    setAnnouncementToDelete(announcement);
                  }}
                >
                  <Trash2 xstyle={styles.renderAnnouncementEditorPlus} />
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

const styles = stylex.create({
  statusOutput: {
    display: 'block',
  },
  priorityAccent: {
    position: 'absolute',
    insetBlock: '0px',
    left: '0px',
    width: '0.25rem',
  },
  priorityAccentHigh: {
    backgroundColor: colors.destructive,
  },
  priorityAccentLow: {
    backgroundColor: `color-mix(in oklab, ${colors.mutedForeground} 40%, transparent)`,
  },
  priorityAccentNormal: {
    backgroundColor: colors.primary,
  },
  announcementsCardCardTitle: {
    display: 'flex',
    alignItems: 'center',
    gap: '0.5rem',
  },
  announcementsCardBellRing: {
    height: '1.25rem',
    width: '1.25rem',
  },
  announcementsCardCardContent: {
    display: 'flex',
    flexDirection: 'column',
    rowGap: '0.75rem',
  },
  announcementsCardP: {
    fontSize: '0.875rem',
    lineHeight: lineHeights.textSm,
    color: colors.destructive,
  },
  announcementsCardRevealItem: {
    borderTopStyle: 'solid',
    borderTopWidth: '1px',
  },
  announcementsCardP2: {
    fontSize: '0.875rem',
    lineHeight: lineHeights.textSm,
    color: colors.mutedForeground,
  },
  renderAnnouncementEditorPlus: {
    width: '1rem',
    height: '1rem',
  },
  renderAnnouncementEditorDialogContent: {
    maxHeight: '90vh',
    overflowY: 'auto',
    maxWidth: {
      default: 'none',
      '@media (min-width: 40rem)': '42rem',
      '@media (min-width: 48rem)': '36rem',
    },
  },
  renderAnnouncementEditorDiv: {
    display: 'flex',
    flexDirection: 'column',
    rowGap: '1rem',
    paddingBlock: '0.5rem',
  },
  renderAnnouncementEditorDivItem: {
    marginBottom: {
      default: null,
      ':not(:last-child)': '0.5rem',
    },
  },
  renderAnnouncementEditorDiv2: {
    display: 'flex',
    flexDirection: 'column',
    rowGap: '0.5rem',
  },
  renderAnnouncementEditorSelectTrigger: {
    width: '100%',
  },
  renderAnnouncementEditorTextarea: {
    minHeight: '8rem',
    width: '100%',
    borderRadius: '8px',
    borderStyle: 'solid',
    borderWidth: '1px',
    borderColor: {
      default: colors.input,
      ':focus-visible': colors.ring,
    },
    backgroundColor: 'transparent',
    paddingInline: '0.75rem',
    paddingBlock: '0.5rem',
    fontSize: '0.875rem',
    lineHeight: lineHeights.textSm,
    outlineStyle: 'none',
    boxShadow: {
      default: null,
      ':focus-visible': `0 0 0 3px color-mix(in oklab, ${colors.ring} 50%, transparent)`,
    },
    '::placeholder': {
      color: colors.mutedForeground,
    },
  },
  renderAnnouncementEditorDiv3: {
    display: 'flex',
    alignItems: 'center',
    justifyContent: 'space-between',
    gap: '0.75rem',
    borderRadius: '8px',
    backgroundColor: `color-mix(in oklab, ${colors.muted} 60%, transparent)`,
    paddingInline: '0.75rem',
    paddingBlock: '0.75rem',
  },
  renderAnnouncementEditorLabel: {
    fontSize: '0.875rem',
    lineHeight: lineHeights.textSm,
  },
  renderAnnouncementEditorDiv4: {
    display: 'flex',
    flexDirection: {
      default: 'column-reverse',
      '@media (min-width: 40rem)': 'row',
    },
    gap: '0.5rem',
    justifyContent: {
      default: null,
      '@media (min-width: 40rem)': 'flex-end',
    },
  },
  renderAnnouncementEditorButton: {
    width: {
      default: '100%',
      '@media (min-width: 40rem)': 'auto',
    },
  },
  renderAnnouncementListMotionDiv: {
    display: 'flex',
    flexDirection: 'column',
    alignItems: 'center',
    gap: '0.75rem',
    paddingInline: '1.5rem',
    paddingBlock: '3rem',
    textAlign: 'center',
  },
  renderAnnouncementListSpan: {
    display: 'flex',
    width: '3rem',
    height: '3rem',
    alignItems: 'center',
    justifyContent: 'center',
    borderRadius: 'calc(infinity * 1px)',
    backgroundColor: colors.muted,
    color: colors.mutedForeground,
  },
  renderAnnouncementListMegaphone: {
    width: '1.25rem',
    height: '1.25rem',
  },
  renderAnnouncementListMotionDiv2: {
    position: 'relative',
    overflow: 'hidden',
    borderBottomStyle: 'solid',
    borderBottomWidth: '1px',
  },
  renderAnnouncementListMotionDivDisabled: {
    opacity: '70%',
  },
  renderAnnouncementListDiv: {
    paddingInline: '1.25rem',
    paddingTop: '1rem',
    paddingBottom: '0.75rem',
  },
  renderAnnouncementListDiv2: {
    display: 'flex',
    flexWrap: 'wrap',
    alignItems: 'center',
    gap: '0.5rem',
  },
  renderAnnouncementListSpan3: {
    fontSize: '0.75rem',
    lineHeight: lineHeights.textXs,
    color: colors.mutedForeground,
    fontVariantNumeric: 'tabular-nums',
  },
  renderAnnouncementListDiv3: {
    fontSize: '15px',
    lineHeight: '1.5rem',
    fontWeight: 700,
    wordBreak: 'break-all',
  },
  renderAnnouncementListP: {
    fontSize: '0.875rem',
    lineHeight: 1.625,
    overflowWrap: 'break-word',
    whiteSpace: 'pre-wrap',
    color: colors.mutedForeground,
  },
  renderAnnouncementListDiv4: {
    display: 'flex',
    alignItems: 'center',
    gap: '0.5rem',
    paddingInline: '1.25rem',
    paddingBottom: '0.75rem',
  },
  renderAnnouncementListSpan4: {
    fontSize: '0.75rem',
    lineHeight: lineHeights.textXs,
    color: colors.mutedForeground,
  },
  renderAnnouncementListDiv5: {
    marginLeft: 'auto',
    display: 'flex',
    alignItems: 'center',
    gap: '0.25rem',
  },
  renderAnnouncementListButton: {
    width: '2.25rem',
    height: '2.25rem',
  },
  renderAnnouncementListButton2: {
    width: '2.25rem',
    height: '2.25rem',
    color: {
      default: colors.mutedForeground,
      ':hover': {
        default: null,
        '@media (hover: hover)': colors.destructive,
      },
    },
    backgroundColor: {
      default: null,
      ':active': {
        default: colors.accentPressed,
        ':hover': {
          default: null,
          '@media (hover: hover)': colors.accentPressed,
        },
      },
      ':hover': {
        default: null,
        '@media (hover: hover)': `color-mix(in oklab, ${colors.destructive} 10%, transparent)`,
      },
    },
  },
});

const PRIORITY_ACCENT_STYLES = {
  high: styles.priorityAccentHigh,
  low: styles.priorityAccentLow,
  normal: styles.priorityAccentNormal,
};
