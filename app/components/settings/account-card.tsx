import {
  SettingsSection,
  SettingsSectionContent,
  SettingsSectionHeader,
  SettingsSectionTitle,
} from '@/components/settings/settings-section';

/**
 * Render immutable account identity information.
 *
 * @param props - Authenticated username.
 * @returns Account information card.
 */
export function AccountCard({ username }: { username: string }) {
  return (
    <SettingsSection>
      <SettingsSectionHeader>
        <SettingsSectionTitle>账号信息</SettingsSectionTitle>
      </SettingsSectionHeader>
      <SettingsSectionContent>
        <div className="text-sm">
          用户名: <span className="font-medium">{username}</span>
        </div>
      </SettingsSectionContent>
    </SettingsSection>
  );
}
