/**
 * Connected-server settings for native hosts.
 */

import { useState } from 'react';
import { useMutation, useQueryClient } from '@tanstack/react-query';

import {
  SettingsGroup,
  SettingsRow,
  SettingsSection,
  SettingsSectionDescription,
  SettingsSectionHeader,
  SettingsSectionTitle,
} from '@/components/settings/settings-section';
import { Layers } from '@/components/ui/animated-icons';
import { Button } from '@/components/ui/button';
import { ConfirmDialog } from '@/components/ui/confirm-dialog';
import { useServerConnection } from '@/lib/server-connection';

/**
 * Show the connected server and switch to another one.
 *
 * Switching revokes and deletes the local token, clears cached queries and the local user
 * cache, and returns to server setup.
 *
 * @returns Server settings section, or null in the browser.
 */
export function ServerConnectionCard() {
  const connection = useServerConnection();
  const queryClient = useQueryClient();
  const [isConfirmOpen, setIsConfirmOpen] = useState(false);
  const switchMutation = useMutation({
    mutationFn: async () => {
      await queryClient.cancelQueries();
      queryClient.clear();
      await connection?.switchServer();
    },
  });

  if (!connection) {
    return null;
  }

  return (
    <SettingsSection>
      <SettingsSectionHeader>
        <SettingsSectionTitle icon={Layers}>服务器</SettingsSectionTitle>
        <SettingsSectionDescription>
          客户端连接的 LitRadar 服务器。更换服务器会退出当前账号并删除本机保存的令牌。
        </SettingsSectionDescription>
      </SettingsSectionHeader>
      <SettingsGroup>
        <SettingsRow title="当前服务器" description={connection.origin}>
          <Button
            type="button"
            variant="outline"
            size="sm"
            onClick={() => {
              switchMutation.reset();
              setIsConfirmOpen(true);
            }}
          >
            更换服务器
          </Button>
        </SettingsRow>
      </SettingsGroup>
      <ConfirmDialog
        open={isConfirmOpen}
        onOpenChange={(open) => {
          if (!open && !switchMutation.isPending) {
            setIsConfirmOpen(false);
          }
        }}
        title="更换服务器？"
        description="将撤销并删除本机保存的访问令牌、清除缓存的数据，然后返回服务器设置。"
        actionLabel="更换服务器"
        pendingLabel="正在更换…"
        isPending={switchMutation.isPending}
        error={switchMutation.isError ? '更换服务器失败，请重试。' : undefined}
        onConfirm={() => switchMutation.mutate()}
      />
    </SettingsSection>
  );
}
