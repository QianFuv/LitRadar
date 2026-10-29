'use client';

import { useState } from 'react';
import { useMutation } from '@tanstack/react-query';

import { changePassword } from '@/lib/api';
import { useAuth } from '@/lib/auth-context';
import {
  SettingsGroup,
  SettingsRow,
  SettingsSection,
  SettingsSectionDescription,
  SettingsSectionHeader,
  SettingsSectionTitle,
} from '@/components/settings/settings-section';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import {
  FADE_UP_VARIANTS,
  MOTION_DURATION_SECONDS,
  MotionParagraph,
  MotionPresence,
  useMotionTransition,
} from '@/components/ui/motion';

/**
 * Render and manage the active user's password change form.
 *
 * @returns Password settings card.
 */
export function PasswordCard() {
  const { logout } = useAuth();
  const [oldPwd, setOldPwd] = useState('');
  const [newPwd, setNewPwd] = useState('');
  const [pwdMsg, setPwdMsg] = useState<string | null>(null);
  const feedbackTransition = useMotionTransition(MOTION_DURATION_SECONDS.fast);
  const changePwdMut = useMutation({
    mutationFn: () => changePassword(oldPwd, newPwd),
    onSuccess: () => {
      setPwdMsg('密码修改成功，请重新登录');
      setTimeout(() => void logout(), 1500);
    },
    onError: (err) => setPwdMsg(err instanceof Error ? err.message : '修改失败'),
  });

  return (
    <SettingsSection>
      <SettingsSectionHeader>
        <SettingsSectionTitle>修改密码</SettingsSectionTitle>
        <SettingsSectionDescription>
          修改成功后需要重新登录，新密码至少 12 位。
        </SettingsSectionDescription>
      </SettingsSectionHeader>
      <form
        onSubmit={(e) => {
          e.preventDefault();
          setPwdMsg(null);
          changePwdMut.mutate();
        }}
      >
        <SettingsGroup>
          <SettingsRow title="原密码" htmlFor="old-password">
            <Input
              id="old-password"
              name="old_password"
              type="password"
              autoComplete="current-password"
              value={oldPwd}
              onChange={(e) => setOldPwd(e.target.value)}
              aria-invalid={changePwdMut.isError}
              className="sm:w-64"
              required
            />
          </SettingsRow>
          <SettingsRow title="新密码" htmlFor="new-password">
            <Input
              id="new-password"
              name="new_password"
              type="password"
              autoComplete="new-password"
              value={newPwd}
              onChange={(e) => setNewPwd(e.target.value)}
              placeholder="至少12位"
              minLength={12}
              aria-invalid={changePwdMut.isError}
              className="sm:w-64"
              required
            />
          </SettingsRow>
          <div className="flex flex-col gap-3 bg-muted/40 px-4 py-3 sm:flex-row sm:items-center sm:justify-between sm:px-5">
            <div className="min-h-5 text-sm">
              <MotionPresence>
                {pwdMsg && (
                  <MotionParagraph
                    key="password-feedback"
                    data-motion-feedback="password"
                    role={changePwdMut.isError ? 'alert' : 'status'}
                    className={changePwdMut.isError ? 'text-destructive' : 'text-muted-foreground'}
                    initial="hidden"
                    animate="visible"
                    exit="exit"
                    variants={FADE_UP_VARIANTS}
                    transition={feedbackTransition}
                  >
                    {pwdMsg}
                  </MotionParagraph>
                )}
              </MotionPresence>
            </div>
            <Button type="submit" size="sm" disabled={changePwdMut.isPending}>
              修改密码
            </Button>
          </div>
        </SettingsGroup>
      </form>
    </SettingsSection>
  );
}
