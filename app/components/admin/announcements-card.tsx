'use client';

import { useMemo, useState } from 'react';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { BellRing, Pencil, Plus, Trash2 } from 'lucide-react';

import {
  adminCreateAnnouncement,
  adminDeleteAnnouncement,
  adminGetAnnouncements,
  adminUpdateAnnouncement,
  type AnnouncementInfo,
} from '@/lib/api';
import { Button } from '@/components/ui/button';
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/components/ui/card';
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
import { Switch } from '@/components/ui/switch';

type AnnouncementsCardProps = {
  token: string;
};

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

function formatDateTime(value: number): string {
  return new Date(value * 1000).toLocaleString('zh-CN', {
    hour: '2-digit',
    minute: '2-digit',
    month: '2-digit',
    day: '2-digit',
    year: 'numeric',
  });
}

export function AnnouncementsCard({ token }: AnnouncementsCardProps) {
  const queryClient = useQueryClient();
  const [dialogOpen, setDialogOpen] = useState(false);
  const [editingAnnouncement, setEditingAnnouncement] = useState<AnnouncementInfo | null>(null);
  const [form, setForm] = useState<AnnouncementFormState>(DEFAULT_FORM);

  const {
    data: announcements = [],
    error,
    isLoading,
  } = useQuery({
    queryKey: ['admin-announcements'],
    queryFn: () => adminGetAnnouncements(token),
  });

  const saveMutation = useMutation({
    mutationFn: async () => {
      if (editingAnnouncement) {
        return adminUpdateAnnouncement(token, editingAnnouncement.id, form);
      }
      return adminCreateAnnouncement(token, form);
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
      adminUpdateAnnouncement(token, announcementId, { enabled }),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['admin-announcements'] });
      queryClient.invalidateQueries({ queryKey: ['announcements'] });
      queryClient.invalidateQueries({ queryKey: ['admin-stats'] });
    },
  });

  const deleteMutation = useMutation({
    mutationFn: (announcementId: number) => adminDeleteAnnouncement(token, announcementId),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['admin-announcements'] });
      queryClient.invalidateQueries({ queryKey: ['announcements'] });
      queryClient.invalidateQueries({ queryKey: ['admin-stats'] });
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

  return (
    <Card>
      <CardHeader>
        <CardTitle className="flex items-center gap-2">
          <BellRing className="h-5 w-5" />
          系统公告
        </CardTitle>
        <CardDescription>管理首页自动弹出的全局公告</CardDescription>
      </CardHeader>
      <CardContent className="space-y-4">
        <Dialog open={dialogOpen} onOpenChange={setDialogOpen}>
          <DialogTrigger asChild>
            <Button
              variant="outline"
              size="sm"
              className="w-full sm:w-auto"
              onClick={openCreateDialog}
            >
              <Plus className="mr-2 h-4 w-4" />
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
                  value={form.message}
                  onChange={(event) =>
                    setForm((current) => ({ ...current, message: event.target.value }))
                  }
                  placeholder="公告内容"
                  className="min-h-28 w-full rounded-md border bg-transparent px-3 py-2 text-sm shadow-xs outline-none"
                />
              </div>
              <div className="flex items-start justify-between gap-3 rounded-md border px-3 py-2">
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
              {mutationError && (
                <p role="alert" className="text-sm text-destructive">
                  {mutationError}
                </p>
              )}
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

        {error instanceof Error && (
          <p role="alert" className="text-sm text-destructive">
            {error.message}
          </p>
        )}

        {isLoading ? (
          <p role="status" className="text-sm text-muted-foreground">
            加载中...
          </p>
        ) : announcements.length === 0 ? (
          <p className="text-sm text-muted-foreground">暂无公告</p>
        ) : (
          <div className="space-y-3">
            {announcements.map((announcement) => (
              <div key={announcement.id} className="rounded-lg border p-4">
                <div className="flex flex-col gap-3 sm:flex-row sm:items-start sm:justify-between">
                  <div className="min-w-0 flex-1 space-y-1">
                    <div className="flex flex-wrap items-center gap-2">
                      <div className="font-medium break-all">{announcement.title}</div>
                      <span className="rounded-full border px-2 py-0.5 text-xs text-muted-foreground">
                        {PRIORITY_LABELS[announcement.priority]}
                      </span>
                    </div>
                    <p className="text-sm text-muted-foreground break-words whitespace-pre-wrap">
                      {announcement.message}
                    </p>
                    <div className="text-xs text-muted-foreground">
                      更新于 {formatDateTime(announcement.updated_at)}
                    </div>
                  </div>
                  <div className="flex w-full flex-wrap items-center justify-end gap-2 sm:w-auto sm:flex-nowrap">
                    <Switch
                      checked={announcement.enabled}
                      aria-label={`${announcement.enabled ? '停用' : '启用'}公告 ${announcement.title}`}
                      onCheckedChange={(checked: boolean) =>
                        toggleMutation.mutate({ announcementId: announcement.id, enabled: checked })
                      }
                    />
                    <Button
                      variant="ghost"
                      size="icon"
                      aria-label={`编辑公告 ${announcement.title}`}
                      onClick={() => openEditDialog(announcement)}
                    >
                      <Pencil className="h-4 w-4" />
                    </Button>
                    <Button
                      variant="ghost"
                      size="icon"
                      className="text-destructive hover:text-destructive"
                      aria-label={`删除公告 ${announcement.title}`}
                      onClick={() => deleteMutation.mutate(announcement.id)}
                    >
                      <Trash2 className="h-4 w-4" />
                    </Button>
                  </div>
                </div>
              </div>
            ))}
          </div>
        )}
      </CardContent>
    </Card>
  );
}
