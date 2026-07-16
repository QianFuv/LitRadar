'use client';

import Link from 'next/link';
import { ArrowLeft } from 'lucide-react';

import { useAuth } from '@/lib/auth-context';
import { AccessTokensCard } from '@/components/settings/access-tokens-card';
import { AccountCard } from '@/components/settings/account-card';
import { CnkiSettingsCard } from '@/components/settings/cnki-card';
import { InviteCodeCard } from '@/components/settings/invite-code-card';
import { PasswordCard } from '@/components/settings/password-card';
import { useSettingsCopy } from '@/components/settings/use-settings-copy';
import { Button } from '@/components/ui/button';

/**
 * Compose account setting cards behind the authentication gate.
 *
 * @returns Account settings page.
 */
export default function SettingsPage() {
  const { user } = useAuth();
  const { copyFeedback, handleCopy } = useSettingsCopy();
  if (!user) {
    return (
      <main
        id="main-content"
        className="flex flex-col items-center justify-center min-h-[60vh] gap-4"
      >
        <p className="text-muted-foreground">请先登录</p>
        <Button asChild>
          <Link href="/login?next=/settings">登录</Link>
        </Button>
      </main>
    );
  }

  return (
    <main
      id="main-content"
      className="mx-auto max-w-3xl space-y-4 p-4 sm:space-y-6 sm:p-6"
      style={{
        paddingBottom:
          'calc(6rem + var(--safe-area-inset-bottom, env(safe-area-inset-bottom, 0px)))',
      }}
    >
      <div className="flex items-center gap-2 sm:gap-3">
        <Button variant="ghost" size="icon" aria-label="返回首页" asChild>
          <Link href="/">
            <ArrowLeft className="h-5 w-5" />
          </Link>
        </Button>
        <h1 className="text-2xl font-bold">账号设置</h1>
      </div>
      <AccountCard username={user.username} />
      <CnkiSettingsCard userId={user.id} copyFeedback={copyFeedback} handleCopy={handleCopy} />
      <PasswordCard />
      <InviteCodeCard copyFeedback={copyFeedback} handleCopy={handleCopy} />
      <AccessTokensCard copyFeedback={copyFeedback} handleCopy={handleCopy} />
    </main>
  );
}
