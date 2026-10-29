'use client';

/**
 * Tracking-folder selection and creation section.
 */

import { FolderHeart, FolderPlus } from 'lucide-react';

import {
  SettingsGroup,
  SettingsRow,
  SettingsSection,
  SettingsSectionContent,
  SettingsSectionDescription,
  SettingsSectionHeader,
  SettingsSectionTitle,
} from '@/components/settings/settings-section';
import type { TrackingPageViewModel } from '@/components/tracking/use-tracking-page';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@/components/ui/select';
import { cn } from '@/lib/utils';

type TrackingFolderCardProps = {
  model: TrackingPageViewModel['folder'];
};

/**
 * Render tracking-folder status, selection, and creation controls.
 *
 * @param props - Folder-specific tracking view model.
 * @returns Tracking folder card.
 */
export function TrackingFolderCard({ model }: TrackingFolderCardProps) {
  const mutationError =
    model.setTrackingMutation.error instanceof Error
      ? model.setTrackingMutation.error.message
      : model.createAndSetMutation.error instanceof Error
        ? model.createAndSetMutation.error.message
        : null;

  return (
    <SettingsSection>
      <SettingsSectionHeader>
        <SettingsSectionTitle>追踪文件夹</SettingsSectionTitle>
        <SettingsSectionDescription>
          设置追踪文件夹后，每周推送的新文章将自动收藏到该文件夹中
        </SettingsSectionDescription>
      </SettingsSectionHeader>
      <SettingsSectionContent className="space-y-2">
        <SettingsGroup>
          <SettingsRow
            leading={
              <span
                aria-hidden="true"
                className={cn(
                  'flex size-10 shrink-0 items-center justify-center rounded-xl',
                  model.trackingFolder
                    ? 'bg-primary text-primary-foreground'
                    : 'bg-muted text-muted-foreground',
                )}
              >
                <FolderHeart className="size-5" />
              </span>
            }
            title="当前文件夹"
            description={
              model.trackingFolder ? (
                <span>当前追踪: {model.trackingFolder.name}</span>
              ) : (
                '尚未设置追踪文件夹'
              )
            }
          >
            {model.folders.length > 0 && (
              <Select
                name="tracking_folder_id"
                value={model.trackingFolder?.id?.toString() || ''}
                onValueChange={(value: string) => model.setTrackingMutation.mutate(Number(value))}
              >
                <SelectTrigger className="w-full sm:w-60" aria-label="选择追踪文件夹">
                  <SelectValue placeholder="选择追踪文件夹" />
                </SelectTrigger>
                <SelectContent>
                  {model.folders.map((folder) => (
                    <SelectItem key={folder.id} value={folder.id.toString()}>
                      {folder.name} ({folder.article_count})
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            )}
          </SettingsRow>
          <SettingsRow title="新建追踪文件夹" description="创建一个新的收藏夹并立即设为追踪目标">
            <div className="flex w-full gap-2 sm:w-auto">
              <Input
                aria-label="新建追踪文件夹名称"
                name="tracking_folder_name"
                autoComplete="off"
                value={model.name}
                onChange={(event) => model.setName(event.target.value)}
                placeholder="文件夹名称"
                className="min-w-0 flex-1 sm:w-44"
              />
              <Button
                variant="outline"
                className="shrink-0"
                disabled={!model.name.trim() || model.createAndSetMutation.isPending}
                onClick={() => model.createAndSetMutation.mutate(model.name.trim())}
              >
                <FolderPlus className="size-4" />
                创建并设为追踪
              </Button>
            </div>
          </SettingsRow>
        </SettingsGroup>
        {mutationError && (
          <p role="alert" className="px-1 text-sm text-destructive">
            {mutationError}
          </p>
        )}
      </SettingsSectionContent>
    </SettingsSection>
  );
}
