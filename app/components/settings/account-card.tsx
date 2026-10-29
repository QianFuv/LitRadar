import { UserAvatar } from '@/components/shell/user-avatar';
import { SettingsGroup, SettingsSection } from '@/components/settings/settings-section';

/**
 * Render immutable account identity information as a profile header.
 *
 * @param props - Authenticated username.
 * @returns Account profile header.
 */
export function AccountCard({ username }: { username: string }) {
  return (
    <SettingsSection aria-label="账号信息">
      <SettingsGroup className="flex items-center gap-4 px-4 py-5 sm:px-5">
        <UserAvatar username={username} className="size-14 text-xl" />
        <div className="min-w-0">
          <p className="text-xs text-muted-foreground">用户名</p>
          <p className="truncate text-lg leading-7 font-bold">{username}</p>
        </div>
      </SettingsGroup>
    </SettingsSection>
  );
}
