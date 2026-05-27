'use client';

/**
 * Desktop account settings workspace.
 */

import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { Copy, Key, Plus, Save, Ticket, Trash2 } from 'lucide-react';
import { useState } from 'react';
import { ShellConfigurator } from '@/components/desktop/shell';
import {
  Badge,
  Button,
  EmptyState,
  Field,
  IconButton,
  Modal,
  Notice,
  Panel,
  TextInput,
} from '@/components/desktop/ui';
import {
  changePassword,
  createAccessToken,
  generateInviteCode,
  getAccessTokens,
  getInviteCode,
  revokeAccessToken,
} from '@/lib/client-api';
import { useAuthSession } from '@/lib/auth-session';
import { formatTimestamp } from '@/lib/format';

const TTL_OPTIONS = [
  { label: '7 天', value: 7 * 86_400 },
  { label: '30 天', value: 30 * 86_400 },
  { label: '90 天', value: 90 * 86_400 },
  { label: '1 年', value: 365 * 86_400 },
];

/**
 * Copy a value to the clipboard.
 *
 * @param value - Value to copy.
 */
async function copyValue(value: string): Promise<void> {
  await navigator.clipboard.writeText(value);
}

/**
 * Render the account settings workspace.
 *
 * @returns Settings workspace.
 */
export function SettingsWorkspace() {
  const { logout, token, user } = useAuthSession();
  const queryClient = useQueryClient();
  const [oldPassword, setOldPassword] = useState('');
  const [newPassword, setNewPassword] = useState('');
  const [passwordMessage, setPasswordMessage] = useState<string | null>(null);
  const [tokenDialogOpen, setTokenDialogOpen] = useState(false);
  const [tokenName, setTokenName] = useState('');
  const [tokenTtl, setTokenTtl] = useState(TTL_OPTIONS[0].value);
  const [newTokenValue, setNewTokenValue] = useState<string | null>(null);
  const [copyMessage, setCopyMessage] = useState<string | null>(null);

  const tokensQuery = useQuery({
    queryKey: ['access-tokens'],
    queryFn: () => getAccessTokens(token!),
    enabled: Boolean(token),
  });

  const inviteQuery = useQuery({
    queryKey: ['invite-code'],
    queryFn: () => getInviteCode(token!),
    enabled: Boolean(token),
  });

  const passwordMutation = useMutation({
    mutationFn: () => changePassword(token!, oldPassword, newPassword),
    onSuccess: () => {
      setPasswordMessage('密码已修改，请重新登录。');
      window.setTimeout(() => {
        void logout();
      }, 1200);
    },
    onError: (error) => {
      setPasswordMessage(error instanceof Error ? error.message : '修改失败');
    },
  });

  const generateInviteMutation = useMutation({
    mutationFn: () => generateInviteCode(token!),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ['invite-code'] }),
  });

  const createTokenMutation = useMutation({
    mutationFn: () => createAccessToken(token!, tokenName.trim(), tokenTtl),
    onSuccess: (data) => {
      setNewTokenValue(data.token);
      setTokenName('');
      queryClient.invalidateQueries({ queryKey: ['access-tokens'] });
    },
  });

  const revokeTokenMutation = useMutation({
    mutationFn: (tokenId: number) => revokeAccessToken(token!, tokenId),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ['access-tokens'] }),
  });

  const handleCopy = async (value: string, message: string) => {
    await copyValue(value);
    setCopyMessage(message);
    window.setTimeout(() => setCopyMessage(null), 1800);
  };

  return (
    <>
      <ShellConfigurator
        title="账号设置"
        actions={
          <>
            <Badge tone="teal">{user?.username}</Badge>
            {user?.is_admin ? <Badge tone="violet">管理员</Badge> : null}
          </>
        }
      />
      <div className="workspace-grid workspace-grid--two">
        <div className="list-stack">
          <Panel title="账号信息" meta="当前登录用户">
            <div className="stat-grid" style={{ gridTemplateColumns: 'repeat(2, minmax(0, 1fr))' }}>
              <div className="stat-tile">
                <div className="stat-tile__label">用户名</div>
                <div className="stat-tile__value" style={{ fontSize: 18 }}>
                  {user?.username}
                </div>
              </div>
              <div className="stat-tile">
                <div className="stat-tile__label">角色</div>
                <div className="stat-tile__value" style={{ fontSize: 18 }}>
                  {user?.is_admin ? '管理员' : '用户'}
                </div>
              </div>
            </div>
          </Panel>

          <Panel title="修改密码" meta="修改后需要重新登录">
            <form
              className="form-grid"
              onSubmit={(event) => {
                event.preventDefault();
                setPasswordMessage(null);
                passwordMutation.mutate();
              }}
            >
              <Field label="原密码">
                <TextInput
                  required
                  type="password"
                  value={oldPassword}
                  onChange={(event) => setOldPassword(event.target.value)}
                />
              </Field>
              <Field label="新密码">
                <TextInput
                  required
                  minLength={6}
                  type="password"
                  value={newPassword}
                  onChange={(event) => setNewPassword(event.target.value)}
                />
              </Field>
              <Button icon={<Save size={15} />} disabled={passwordMutation.isPending}>
                修改密码
              </Button>
              {passwordMessage ? <Notice>{passwordMessage}</Notice> : null}
            </form>
          </Panel>

          <Panel title="邀请码" meta="每个用户一个自助邀请码">
            {inviteQuery.data ? (
              <div className="form-grid">
                <code className="copy-code">{inviteQuery.data.code}</code>
                <div className="toolbar">
                  <Button
                    icon={<Copy size={15} />}
                    variant="secondary"
                    onClick={() => void handleCopy(inviteQuery.data!.code, '邀请码已复制')}
                  >
                    复制邀请码
                  </Button>
                  <Badge tone={inviteQuery.data.used ? 'neutral' : 'teal'}>
                    {inviteQuery.data.used ? '已使用' : '可使用'}
                  </Badge>
                </div>
              </div>
            ) : (
              <Button
                icon={<Ticket size={15} />}
                disabled={generateInviteMutation.isPending}
                onClick={() => generateInviteMutation.mutate()}
              >
                生成邀请码
              </Button>
            )}
          </Panel>
        </div>

        <Panel
          title="访问令牌"
          meta="用于接口访问或第三方集成"
          actions={
            <Button
              icon={<Plus size={15} />}
              variant="violet"
              onClick={() => {
                setNewTokenValue(null);
                setTokenDialogOpen(true);
              }}
            >
              新建令牌
            </Button>
          }
        >
          {copyMessage ? <Notice>{copyMessage}</Notice> : null}
          {tokensQuery.isPending ? (
            <Notice>正在加载令牌...</Notice>
          ) : tokensQuery.data?.length === 0 ? (
            <EmptyState>暂无访问令牌。</EmptyState>
          ) : (
            <div className="list-stack">
              {tokensQuery.data?.map((accessToken) => (
                <div key={accessToken.id} className="article-row">
                  <div className="toolbar">
                    <Key size={16} />
                    <strong>{accessToken.name || '未命名令牌'}</strong>
                    <Badge tone="neutral">到 {formatTimestamp(accessToken.expires_at)} 过期</Badge>
                  </div>
                  <IconButton
                    danger
                    aria-label="撤销令牌"
                    title="撤销令牌"
                    onClick={() => revokeTokenMutation.mutate(accessToken.id)}
                  >
                    <Trash2 size={15} />
                  </IconButton>
                </div>
              ))}
            </div>
          )}
        </Panel>
      </div>

      <Modal
        narrow
        open={tokenDialogOpen}
        title="创建访问令牌"
        description="令牌只显示一次，请在关闭前保存到你的集成配置中。"
        onClose={() => setTokenDialogOpen(false)}
        footer={<Button onClick={() => setTokenDialogOpen(false)}>完成</Button>}
      >
        {newTokenValue ? (
          <div className="form-grid">
            <code className="copy-code">{newTokenValue}</code>
            <Button
              icon={<Copy size={15} />}
              variant="secondary"
              onClick={() => void handleCopy(newTokenValue, '令牌已复制')}
            >
              复制令牌
            </Button>
          </div>
        ) : (
          <form
            className="form-grid"
            onSubmit={(event) => {
              event.preventDefault();
              createTokenMutation.mutate();
            }}
          >
            <Field label="名称">
              <TextInput
                value={tokenName}
                onChange={(event) => setTokenName(event.target.value)}
                placeholder="例如：MCP 集成"
              />
            </Field>
            <Field label="有效期">
              <div className="toolbar toolbar--wrap">
                {TTL_OPTIONS.map((option) => (
                  <Button
                    key={option.value}
                    type="button"
                    variant={tokenTtl === option.value ? 'primary' : 'secondary'}
                    onClick={() => setTokenTtl(option.value)}
                  >
                    {option.label}
                  </Button>
                ))}
              </div>
            </Field>
            <Button disabled={!tokenName.trim() || createTokenMutation.isPending}>创建</Button>
          </form>
        )}
      </Modal>
    </>
  );
}
