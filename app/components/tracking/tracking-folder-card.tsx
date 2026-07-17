'use client';

/**
 * Tracking-folder selection and creation section.
 */

import { FolderPlus } from 'lucide-react';

import {
  SettingsSection,
  SettingsSectionContent,
  SettingsSectionDescription,
  SettingsSectionHeader,
  SettingsSectionTitle,
} from '@/components/settings/settings-section';
import type { TrackingPageViewModel } from '@/components/tracking/use-tracking-page';
import { Badge } from '@/components/ui/badge';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@/components/ui/select';

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
  return (
    <SettingsSection>
      <SettingsSectionHeader>
        <SettingsSectionTitle>追踪文件夹</SettingsSectionTitle>
        <SettingsSectionDescription>
          设置追踪文件夹后，每周推送的新文章将自动收藏到该文件夹中
        </SettingsSectionDescription>
      </SettingsSectionHeader>
      <SettingsSectionContent className="space-y-4">
        {model.trackingFolder ? (
          <div className="flex items-center gap-2">
            <Badge variant="secondary" className="text-sm">
              当前追踪: {model.trackingFolder.name}
            </Badge>
          </div>
        ) : (
          <p className="text-sm text-muted-foreground">尚未设置追踪文件夹</p>
        )}

        {model.folders.length > 0 && (
          <div className="flex flex-col gap-2 sm:flex-row sm:items-center">
            <Select
              name="tracking_folder_id"
              value={model.trackingFolder?.id?.toString() || ''}
              onValueChange={(value: string) => model.setTrackingMutation.mutate(Number(value))}
            >
              <SelectTrigger className="w-full sm:w-60">
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
          </div>
        )}

        <div className="flex flex-col gap-2 sm:flex-row sm:items-center">
          <Input
            aria-label="新建追踪文件夹名称"
            name="tracking_folder_name"
            autoComplete="off"
            value={model.name}
            onChange={(event) => model.setName(event.target.value)}
            placeholder="新建追踪文件夹"
            className="w-full sm:w-60"
          />
          <Button
            variant="outline"
            size="sm"
            className="w-full sm:w-auto"
            disabled={!model.name.trim() || model.createAndSetMutation.isPending}
            onClick={() => model.createAndSetMutation.mutate(model.name.trim())}
          >
            <FolderPlus className="mr-1 h-4 w-4" />
            创建并设为追踪
          </Button>
        </div>
      </SettingsSectionContent>
    </SettingsSection>
  );
}
