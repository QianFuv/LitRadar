'use client';

/**
 * Desktop login and registration workspace.
 */

import { useQuery } from '@tanstack/react-query';
import { ArrowRight, Lock, Search, ShieldCheck, UserPlus } from 'lucide-react';
import { useRouter, useSearchParams } from 'next/navigation';
import { useState, type FormEvent } from 'react';
import { Badge, Button, Field, Notice, TextInput } from '@/components/desktop/ui';
import { getInviteRequirement } from '@/lib/client-api';
import { useAuthSession } from '@/lib/auth-session';

type LoginMode = 'login' | 'register';

/**
 * Resolve a safe redirect target from the next query parameter.
 *
 * @param value - Raw query parameter.
 * @returns Safe internal path.
 */
function resolveNextPath(value: string | null): string {
  if (!value || !value.startsWith('/') || value.startsWith('//')) {
    return '/';
  }
  return value;
}

/**
 * Render the public login/register page.
 *
 * @returns Login workspace.
 */
export function LoginWorkspace() {
  const router = useRouter();
  const searchParams = useSearchParams();
  const { login, register } = useAuthSession();
  const [mode, setMode] = useState<LoginMode>('login');
  const [username, setUsername] = useState('');
  const [password, setPassword] = useState('');
  const [inviteCode, setInviteCode] = useState('');
  const [error, setError] = useState<string | null>(null);
  const [isSubmitting, setIsSubmitting] = useState(false);
  const nextPath = resolveNextPath(searchParams.get('next'));

  const inviteRequirementQuery = useQuery({
    queryKey: ['invite-required'],
    queryFn: getInviteRequirement,
  });

  const inviteRequired = inviteRequirementQuery.data?.required ?? true;
  const isRegister = mode === 'register';

  const handleSubmit = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    setError(null);
    setIsSubmitting(true);
    try {
      if (isRegister) {
        await register(username.trim(), password, inviteCode.trim());
      } else {
        await login(username.trim(), password);
      }
      router.replace(nextPath);
    } catch (caughtError) {
      setError(caughtError instanceof Error ? caughtError.message : '操作失败，请重试');
    } finally {
      setIsSubmitting(false);
    }
  };

  return (
    <main className="login-workspace">
      <section className="login-hero">
        <div className="login-hero__brand">
          <div className="desktop-shell__brand-mark">P</div>
          <div>
            <strong>Paper Scanner</strong>
            <span>文献检索工作台</span>
          </div>
        </div>
        <div className="login-hero__copy">
          <Badge tone="teal">
            <Search size={13} />
            PC Research Console
          </Badge>
          <h1>进入你的文献检索工作台</h1>
          <p>统一管理检索、收藏、追踪、推送和系统维护流程。</p>
        </div>
        <div className="login-hero__metrics">
          <div>
            <strong>FTS5</strong>
            <span>全文检索</span>
          </div>
          <div>
            <strong>Weekly</strong>
            <span>每周更新</span>
          </div>
          <div>
            <strong>Push</strong>
            <span>追踪推送</span>
          </div>
        </div>
      </section>

      <section className="login-panel">
        <div className="login-panel__header">
          <Badge tone={isRegister ? 'violet' : 'teal'}>
            {isRegister ? <UserPlus size={13} /> : <ShieldCheck size={13} />}
            {isRegister ? '注册账号' : '安全登录'}
          </Badge>
          <h2>{isRegister ? '创建研究账号' : '登录 Paper Scanner'}</h2>
          <p>{isRegister ? '使用邀请码创建新账号。' : '使用你的账号继续工作。'}</p>
        </div>

        <form className="form-grid" onSubmit={handleSubmit}>
          <Field label="用户名">
            <TextInput
              required
              autoComplete="username"
              value={username}
              onChange={(event) => setUsername(event.target.value)}
              placeholder="3-32 位字母、数字或下划线"
            />
          </Field>
          <Field label="密码">
            <TextInput
              required
              minLength={6}
              autoComplete={isRegister ? 'new-password' : 'current-password'}
              type="password"
              value={password}
              onChange={(event) => setPassword(event.target.value)}
              placeholder="至少 6 位"
            />
          </Field>
          {isRegister && inviteRequired ? (
            <Field label="邀请码">
              <TextInput
                required
                value={inviteCode}
                onChange={(event) => setInviteCode(event.target.value)}
                placeholder="输入管理员或用户分享的邀请码"
              />
            </Field>
          ) : null}
          {error ? <Notice tone="error">{error}</Notice> : null}
          <Button
            icon={isRegister ? <UserPlus size={15} /> : <ArrowRight size={15} />}
            disabled={isSubmitting}
            type="submit"
            wide
          >
            {isSubmitting ? '请稍候...' : isRegister ? '注册并登录' : '登录'}
          </Button>
        </form>

        <div className="login-panel__footer">
          <span>{isRegister ? '已有账号？' : '还没有账号？'}</span>
          <button
            type="button"
            onClick={() => {
              setMode(isRegister ? 'login' : 'register');
              setError(null);
            }}
          >
            {isRegister ? '返回登录' : '创建账号'}
          </button>
        </div>
        <div className="login-panel__security">
          <Lock size={15} />
          会话令牌保存在本机浏览器，退出登录会主动撤销当前令牌。
        </div>
        {inviteRequirementQuery.isError ? (
          <Notice tone="error">邀请码状态获取失败，注册时将按需要填写。</Notice>
        ) : null}
      </section>
    </main>
  );
}
