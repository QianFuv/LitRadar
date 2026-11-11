import { useState } from 'react';
import { useStore } from '@tanstack/react-form';
import { useMutation, useQuery } from '@tanstack/react-query';

import { UserAvatar } from '@/components/shell/user-avatar';
import {
  SettingsGroup,
  SettingsRow,
  SettingsSection,
  SettingsSectionDescription,
  SettingsSectionHeader,
  SettingsSectionTitle,
  StatusPill,
  settingsGroupItemStyle,
} from '@/components/settings/settings-section';
import { LinkIcon } from '@/components/ui/animated-icons';
import { Button } from '@/components/ui/button';
import { ConfirmDialog } from '@/components/ui/confirm-dialog';
import { Input } from '@/components/ui/input';
import { MotionParagraph, MotionPresence, REVEAL_VARIANTS } from '@/components/ui/motion';
import {
  authProvidersQuery,
  startOidcLink,
  unlinkOidc,
  updateAccountEmail,
  type AuthUser,
} from '@/lib/api';
import { useAuth } from '@/lib/auth-context';
import { getAccountErrorMessage, getOidcErrorMessage } from '@/lib/auth-error';
import { CLIENT_PLATFORM } from '@/lib/platform';
import { useAppForm } from '@/lib/form';
import * as stylex from '@stylexjs/stylex';
import { colors, lineHeights } from '@/lib/theme.stylex';

/** Successful `oidc_link` query result written by the link callback. */
const OIDC_LINK_SUCCESS = 'linked';

/** Display name used when the server has not configured a provider label. */
const DEFAULT_OIDC_DISPLAY_NAME = '单点登录';

/**
 * Render immutable account identity information as a profile header.
 *
 * @param props - Authenticated user projection.
 * @returns Account profile header.
 */
export function AccountCard({ user }: { user: AuthUser }) {
  return (
    <SettingsSection aria-label="账号信息">
      <SettingsGroup xstyle={styles.accountCardSettingsGroup}>
        <UserAvatar
          username={user.username}
          xstyle={[settingsGroupItemStyle, styles.accountCardUserAvatar]}
        />
        <div {...stylex.props(settingsGroupItemStyle, styles.accountCardDiv)}>
          <p {...stylex.props(styles.accountCardP)}>用户名</p>
          <p {...stylex.props(styles.accountCardP2)}>{user.username}</p>
          <div {...stylex.props(styles.accountCardDiv2)}>
            <span {...stylex.props(styles.accountCardSpan)}>{user.email ?? '未设置邮箱'}</span>
            {user.auth_kind === 'oidc' ? (
              <StatusPill tone="info">单点登录账号</StatusPill>
            ) : (
              <StatusPill>本地账号</StatusPill>
            )}
          </div>
        </div>
      </SettingsGroup>
    </SettingsSection>
  );
}

type AccountFeedback = { isError: boolean; message: string };

/**
 * Convert a one-shot link callback result into account feedback.
 *
 * @param result - `oidc_link` query value captured by the settings center.
 * @param displayName - Provider display name.
 * @returns Feedback for the result, or null when there is none.
 */
function describeOidcLinkResult(
  result: string | null,
  displayName: string,
): AccountFeedback | null {
  if (result === null) {
    return null;
  }
  if (result === OIDC_LINK_SUCCESS) {
    return { isError: false, message: `已绑定 ${displayName}，之后可以使用它登录。` };
  }
  return { isError: true, message: getOidcErrorMessage(result) };
}

/**
 * Render email and single sign-on management for the active account.
 *
 * Local accounts can set or clear their email and link or unlink the identity provider;
 * single sign-on accounts see their provider-managed email read-only and cannot unlink.
 *
 * @param props - One-shot `oidc_link` callback result captured from the URL.
 * @returns Sign-in methods settings section, or null without an authenticated user.
 */
export function AccountSignInCard({ oidcLinkResult }: { oidcLinkResult: string | null }) {
  const { updateUser, user } = useAuth();
  const [feedback, setFeedback] = useState<AccountFeedback | null>(null);
  const [isUnlinkDialogOpen, setIsUnlinkDialogOpen] = useState(false);
  const isLocalAccount = user?.auth_kind === 'local';
  const { data: providers } = useQuery({
    ...authProvidersQuery(),
    enabled: Boolean(user),
  });
  const displayName = providers?.oidc_display_name.trim() || DEFAULT_OIDC_DISPLAY_NAME;
  const emailMutation = useMutation({
    mutationFn: (email: string | null) => updateAccountEmail(email),
    onSuccess: (updatedUser) => {
      updateUser(updatedUser);
      emailForm.setFieldValue('email', updatedUser.email ?? '');
      setFeedback({
        isError: false,
        message: updatedUser.email ? '邮箱已更新' : '邮箱已清除',
      });
    },
    onError: (error) => setFeedback({ isError: true, message: getAccountErrorMessage(error) }),
  });
  const emailForm = useAppForm({
    defaultValues: { email: user?.email ?? '' },
    onSubmit: ({ value }) => {
      setFeedback(null);
      emailMutation.mutate(value.email.trim() || null);
    },
  });
  const emailDraft = useStore(emailForm.store, (formState) => formState.values.email);
  const linkMutation = useMutation({
    mutationFn: startOidcLink,
    onSuccess: ({ authorization_url: authorizationUrl }) => {
      window.location.assign(authorizationUrl);
    },
    onError: (error) => setFeedback({ isError: true, message: getAccountErrorMessage(error) }),
  });
  const unlinkMutation = useMutation({
    mutationFn: unlinkOidc,
    onSuccess: (updatedUser) => {
      updateUser(updatedUser);
      setIsUnlinkDialogOpen(false);
      setFeedback({ isError: false, message: `已解除 ${displayName} 绑定` });
    },
  });

  if (!user) {
    return null;
  }

  const visibleFeedback = feedback ?? describeOidcLinkResult(oidcLinkResult, displayName);
  const isEmailUnchanged = emailDraft.trim() === (user.email ?? '');
  const isOidcEnabled = providers?.oidc_enabled === true;

  return (
    <SettingsSection>
      <SettingsSectionHeader>
        <SettingsSectionTitle icon={LinkIcon}>登录方式</SettingsSectionTitle>
        <SettingsSectionDescription>
          {isLocalAccount
            ? '邮箱可代替用户名登录；绑定单点登录后，也可以通过身份提供方登录。'
            : `该账号由 ${displayName} 创建，只能通过单点登录访问。`}
        </SettingsSectionDescription>
      </SettingsSectionHeader>
      <SettingsGroup>
        {isLocalAccount ? (
          <form
            {...stylex.props(settingsGroupItemStyle)}
            onSubmit={(event) => {
              event.preventDefault();
              void emailForm.handleSubmit();
            }}
          >
            <SettingsRow title="邮箱" htmlFor="account-email" description="留空保存即可清除邮箱">
              <emailForm.Field name="email">
                {(field) => (
                  <Input
                    id="account-email"
                    name="email"
                    type="email"
                    autoComplete="email"
                    spellCheck={false}
                    value={field.state.value}
                    onValueChange={(value) => field.handleChange(value)}
                    onBlur={field.handleBlur}
                    placeholder="未设置"
                    aria-invalid={emailMutation.isError}
                    xstyle={styles.accountSignInCardInput}
                  />
                )}
              </emailForm.Field>
              <Button
                type="submit"
                size="sm"
                disabled={emailMutation.isPending || isEmailUnchanged}
              >
                保存邮箱
              </Button>
            </SettingsRow>
          </form>
        ) : (
          <SettingsRow title="邮箱" description={`由 ${displayName} 管理，无法在此修改`}>
            <span {...stylex.props(styles.accountSignInCardSpan)}>{user.email ?? '未设置'}</span>
          </SettingsRow>
        )}
        <SettingsRow
          title={displayName}
          description={describeOidcRow(user, isOidcEnabled)}
          data-account-oidc-state={user.is_oidc_linked ? 'linked' : 'unlinked'}
        >
          {user.is_oidc_linked ? (
            <StatusPill tone="success">已绑定</StatusPill>
          ) : (
            <StatusPill>未绑定</StatusPill>
          )}
          {isLocalAccount &&
            CLIENT_PLATFORM.supportsSingleSignOn &&
            (user.is_oidc_linked ? (
              <Button
                type="button"
                variant="outline"
                size="sm"
                onClick={() => {
                  setFeedback(null);
                  unlinkMutation.reset();
                  setIsUnlinkDialogOpen(true);
                }}
              >
                解除绑定
              </Button>
            ) : (
              <Button
                type="button"
                size="sm"
                disabled={!isOidcEnabled || linkMutation.isPending || linkMutation.isSuccess}
                onClick={() => {
                  setFeedback(null);
                  linkMutation.mutate();
                }}
              >
                绑定 {displayName}
              </Button>
            ))}
        </SettingsRow>
        <div {...stylex.props(settingsGroupItemStyle, styles.accountSignInCardDiv)}>
          <MotionPresence>
            {visibleFeedback && (
              <MotionParagraph
                key={visibleFeedback.message}
                data-motion-feedback="account-sign-in"
                role={visibleFeedback.isError ? 'alert' : 'status'}
                {...stylex.props(
                  visibleFeedback.isError
                    ? styles.accountSignInCardMotionParagraphError
                    : styles.accountSignInCardMotionParagraphNotError,
                )}
                initial="hidden"
                animate="visible"
                exit="exit"
                variants={REVEAL_VARIANTS}
              >
                {visibleFeedback.message}
              </MotionParagraph>
            )}
          </MotionPresence>
        </div>
      </SettingsGroup>
      <ConfirmDialog
        open={isUnlinkDialogOpen}
        onOpenChange={(open) => {
          if (!open && !unlinkMutation.isPending) {
            setIsUnlinkDialogOpen(false);
          }
        }}
        title={`解除 ${displayName} 绑定？`}
        description="解除后只能使用用户名或邮箱和密码登录，之后可以重新绑定。"
        actionLabel="解除绑定"
        pendingLabel="解除中…"
        isPending={unlinkMutation.isPending}
        error={unlinkMutation.isError ? getAccountErrorMessage(unlinkMutation.error) : undefined}
        onConfirm={() => unlinkMutation.mutate()}
      />
    </SettingsSection>
  );
}

/**
 * Explain the single sign-on row for the account kind and link state.
 *
 * @param user - Active user.
 * @param isOidcEnabled - Whether the server currently enables OIDC.
 * @returns Row description.
 */
function describeOidcRow(user: AuthUser, isOidcEnabled: boolean): string {
  if (!CLIENT_PLATFORM.supportsSingleSignOn) {
    return '客户端不支持绑定或解除单点登录，请在网页版中操作';
  }
  if (user.auth_kind === 'oidc') {
    return '单点登录账号无法解除绑定';
  }
  if (user.is_oidc_linked) {
    return '可使用单点登录或密码登录';
  }
  return isOidcEnabled ? '绑定后可使用单点登录' : '服务器尚未启用单点登录';
}

const styles = stylex.create({
  accountCardSettingsGroup: {
    display: 'flex',
    alignItems: 'center',
    gap: '1rem',
    paddingInline: {
      default: '1rem',
      '@media (min-width: 40rem)': '1.25rem',
    },
    paddingBlock: '1.25rem',
  },
  accountCardUserAvatar: {
    width: '3.5rem',
    height: '3.5rem',
    fontSize: '1.25rem',
    lineHeight: 1.4,
  },
  accountCardDiv: {
    minWidth: '0px',
  },
  accountCardP: {
    fontSize: '0.75rem',
    lineHeight: lineHeights.textXs,
    color: colors.mutedForeground,
  },
  accountCardP2: {
    overflow: 'hidden',
    textOverflow: 'ellipsis',
    whiteSpace: 'nowrap',
    fontSize: '1.125rem',
    lineHeight: '1.75rem',
    fontWeight: 700,
  },
  accountCardDiv2: {
    marginTop: '0.25rem',
    display: 'flex',
    minWidth: '0px',
    flexWrap: 'wrap',
    alignItems: 'center',
    gap: '0.5rem',
  },
  accountCardSpan: {
    overflow: 'hidden',
    textOverflow: 'ellipsis',
    whiteSpace: 'nowrap',
    fontSize: '0.75rem',
    lineHeight: lineHeights.textXs,
    color: colors.mutedForeground,
  },
  accountSignInCardInput: {
    width: {
      default: null,
      '@media (min-width: 40rem)': '16rem',
    },
  },
  accountSignInCardSpan: {
    fontSize: '0.875rem',
    lineHeight: lineHeights.textSm,
  },
  accountSignInCardDiv: {
    minHeight: '1.25rem',
    backgroundColor: `color-mix(in oklab, ${colors.muted} 40%, transparent)`,
    paddingInline: {
      default: '1rem',
      '@media (min-width: 40rem)': '1.25rem',
    },
    paddingBlock: '0.75rem',
    fontSize: '0.875rem',
    lineHeight: lineHeights.textSm,
  },
  accountSignInCardMotionParagraphError: {
    color: colors.destructive,
  },
  accountSignInCardMotionParagraphNotError: {
    color: colors.mutedForeground,
  },
});
