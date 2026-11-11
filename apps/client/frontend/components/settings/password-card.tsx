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
  settingsGroupItemStyle,
} from '@/components/settings/settings-section';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import { MotionParagraph, MotionPresence, REVEAL_VARIANTS } from '@/components/ui/motion';
import { KeyRound } from '@/components/ui/animated-icons';
import { useAppForm } from '@/lib/form';
import * as stylex from '@stylexjs/stylex';
import { colors, lineHeights } from '@/lib/theme.stylex';

/**
 * Render and manage the active user's password change form.
 *
 * @returns Password settings card.
 */
export function PasswordCard() {
  const { logout } = useAuth();
  const [pwdMsg, setPwdMsg] = useState<string | null>(null);
  const changePwdMut = useMutation({
    mutationFn: (passwords: { oldPassword: string; newPassword: string }) =>
      changePassword(passwords.oldPassword, passwords.newPassword),
    onSuccess: () => {
      setPwdMsg('密码修改成功，请重新登录');
      setTimeout(() => void logout(), 1500);
    },
    onError: (err) => setPwdMsg(err instanceof Error ? err.message : '修改失败'),
  });
  const passwordForm = useAppForm({
    defaultValues: { oldPassword: '', newPassword: '' },
    onSubmit: ({ value }) => {
      setPwdMsg(null);
      changePwdMut.mutate(value);
    },
  });

  return (
    <SettingsSection>
      <SettingsSectionHeader>
        <SettingsSectionTitle icon={KeyRound}>修改密码</SettingsSectionTitle>
        <SettingsSectionDescription>
          修改成功后需要重新登录，新密码至少 12 位。
        </SettingsSectionDescription>
      </SettingsSectionHeader>
      <form
        onSubmit={(event) => {
          event.preventDefault();
          void passwordForm.handleSubmit();
        }}
      >
        <SettingsGroup>
          <SettingsRow title="原密码" htmlFor="old-password">
            <passwordForm.Field name="oldPassword">
              {(field) => (
                <Input
                  id="old-password"
                  name="old_password"
                  type="password"
                  autoComplete="current-password"
                  value={field.state.value}
                  onValueChange={(value) => field.handleChange(value)}
                  onBlur={field.handleBlur}
                  aria-invalid={changePwdMut.isError}
                  xstyle={styles.passwordCardInput}
                  required
                />
              )}
            </passwordForm.Field>
          </SettingsRow>
          <SettingsRow title="新密码" htmlFor="new-password">
            <passwordForm.Field name="newPassword">
              {(field) => (
                <Input
                  id="new-password"
                  name="new_password"
                  type="password"
                  autoComplete="new-password"
                  value={field.state.value}
                  onValueChange={(value) => field.handleChange(value)}
                  onBlur={field.handleBlur}
                  placeholder="至少12位"
                  minLength={12}
                  aria-invalid={changePwdMut.isError}
                  xstyle={styles.passwordCardInput}
                  required
                />
              )}
            </passwordForm.Field>
          </SettingsRow>
          <div {...stylex.props(settingsGroupItemStyle, styles.passwordCardDiv)}>
            <div {...stylex.props(styles.passwordCardDiv2)}>
              <MotionPresence>
                {pwdMsg && (
                  <MotionParagraph
                    key="password-feedback"
                    data-motion-feedback="password"
                    role={changePwdMut.isError ? 'alert' : 'status'}
                    {...stylex.props(
                      changePwdMut.isError
                        ? styles.passwordCardMotionParagraphError
                        : styles.passwordCardMotionParagraphNotError,
                    )}
                    initial="hidden"
                    animate="visible"
                    exit="exit"
                    variants={REVEAL_VARIANTS}
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

const styles = stylex.create({
  passwordCardInput: {
    width: {
      default: null,
      '@media (min-width: 40rem)': '16rem',
    },
  },
  passwordCardDiv: {
    display: 'flex',
    flexDirection: {
      default: 'column',
      '@media (min-width: 40rem)': 'row',
    },
    gap: '0.75rem',
    backgroundColor: `color-mix(in oklab, ${colors.muted} 40%, transparent)`,
    paddingInline: {
      default: '1rem',
      '@media (min-width: 40rem)': '1.25rem',
    },
    paddingBlock: '0.75rem',
    alignItems: {
      default: null,
      '@media (min-width: 40rem)': 'center',
    },
    justifyContent: {
      default: null,
      '@media (min-width: 40rem)': 'space-between',
    },
  },
  passwordCardDiv2: {
    minHeight: '1.25rem',
    fontSize: '0.875rem',
    lineHeight: lineHeights.textSm,
  },
  passwordCardMotionParagraphError: {
    color: colors.destructive,
  },
  passwordCardMotionParagraphNotError: {
    color: colors.mutedForeground,
  },
});
