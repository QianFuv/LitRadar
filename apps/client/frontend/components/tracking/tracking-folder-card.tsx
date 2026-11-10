/**
 * Tracking-folder selection and creation section.
 */

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
import { Folder, Plus } from '@/components/ui/animated-icons';
import * as stylex from '@stylexjs/stylex';
import { colors, lineHeights } from '@/lib/theme.stylex';

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
        <SettingsSectionTitle icon={Folder}>追踪文件夹</SettingsSectionTitle>
        <SettingsSectionDescription>
          设置追踪文件夹后，每周推送的新文章将自动收藏到该文件夹中
        </SettingsSectionDescription>
      </SettingsSectionHeader>
      <SettingsSectionContent>
        <SettingsGroup xstyle={styles.trackingFolderCardSettingsSectionContentItem}>
          <SettingsRow
            leading={
              <span
                aria-hidden="true"
                {...stylex.props(
                  styles.trackingFolderCardSpan,
                  model.trackingFolder
                    ? styles.trackingFolderCardSpanTrackingFolder
                    : styles.trackingFolderCardSpanNotTrackingFolder,
                )}
              >
                <Folder xstyle={styles.trackingFolderCardFolder} />
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
                items={model.folders.map((folder) => ({
                  value: folder.id.toString(),
                  label: `${folder.name} (${folder.article_count})`,
                }))}
                onValueChange={(value) => model.setTrackingMutation.mutate(Number(value))}
              >
                <SelectTrigger
                  xstyle={styles.trackingFolderCardSelectTrigger}
                  aria-label="选择追踪文件夹"
                >
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
            <div {...stylex.props(styles.trackingFolderCardDiv)}>
              <Input
                aria-label="新建追踪文件夹名称"
                name="tracking_folder_name"
                autoComplete="off"
                value={model.name}
                onChange={(event) => model.setName(event.target.value)}
                placeholder="文件夹名称"
                xstyle={styles.trackingFolderCardInput}
              />
              <Button
                variant="outline"
                xstyle={styles.trackingFolderCardButton}
                disabled={!model.name.trim() || model.createAndSetMutation.isPending}
                onClick={() => model.createAndSetMutation.mutate(model.name.trim())}
              >
                <Plus xstyle={styles.trackingFolderCardPlus} />
                创建并设为追踪
              </Button>
            </div>
          </SettingsRow>
        </SettingsGroup>
        {mutationError && (
          <p
            role="alert"
            {...stylex.props(
              styles.trackingFolderCardSettingsSectionContentItem,
              styles.trackingFolderCardP,
            )}
          >
            {mutationError}
          </p>
        )}
      </SettingsSectionContent>
    </SettingsSection>
  );
}

const styles = stylex.create({
  trackingFolderCardSettingsSectionContentItem: {
    marginBottom: {
      default: null,
      ':not(:last-child)': '0.5rem',
    },
  },
  trackingFolderCardSpan: {
    display: 'flex',
    width: '2.5rem',
    height: '2.5rem',
    flexShrink: 0,
    alignItems: 'center',
    justifyContent: 'center',
    borderRadius: '12px',
  },
  trackingFolderCardSpanTrackingFolder: {
    backgroundColor: colors.primary,
    color: colors.primaryForeground,
  },
  trackingFolderCardSpanNotTrackingFolder: {
    backgroundColor: colors.muted,
    color: colors.mutedForeground,
  },
  trackingFolderCardFolder: {
    width: '1.25rem',
    height: '1.25rem',
  },
  trackingFolderCardSelectTrigger: {
    width: {
      default: '100%',
      '@media (min-width: 40rem)': '15rem',
    },
  },
  trackingFolderCardDiv: {
    display: 'flex',
    width: {
      default: '100%',
      '@media (min-width: 40rem)': 'auto',
    },
    gap: '0.5rem',
  },
  trackingFolderCardInput: {
    minWidth: '0px',
    flex: '1',
    width: {
      default: null,
      '@media (min-width: 40rem)': '11rem',
    },
  },
  trackingFolderCardButton: {
    flexShrink: 0,
  },
  trackingFolderCardPlus: {
    width: '1rem',
    height: '1rem',
  },
  trackingFolderCardP: {
    paddingInline: '0.25rem',
    fontSize: '0.875rem',
    lineHeight: lineHeights.textSm,
    color: colors.destructive,
  },
});
