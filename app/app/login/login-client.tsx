'use client';

import { useRouter, useSearchParams } from 'next/navigation';
import { useEffect, useState, type FormEvent } from 'react';
import { Button } from '@/components/ui/button';
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/components/ui/card';
import { Input } from '@/components/ui/input';
import { Label } from '@/components/ui/label';
import { useAuth } from '@/lib/auth-context';

const API_BASE_URL = process.env.NEXT_PUBLIC_API_URL || '';

function resolveBase(): string {
  if (API_BASE_URL) return API_BASE_URL;
  if (typeof window !== 'undefined') return window.location.origin;
  return 'http://localhost:8000';
}

export default function LoginClient() {
  const router = useRouter();
  const searchParams = useSearchParams();
  const { login, register } = useAuth();
  const nextParam = searchParams.get('next') || '';
  const nextPath = nextParam.startsWith('/') && !nextParam.startsWith('//') ? nextParam : '/';

  const [username, setUsername] = useState('');
  const [password, setPassword] = useState('');
  const [inviteCode, setInviteCode] = useState('');
  const [error, setError] = useState<string | null>(null);
  const [isSubmitting, setIsSubmitting] = useState(false);
  const [mode, setMode] = useState<'login' | 'register'>('login');
  const [inviteRequired, setInviteRequired] = useState(true);

  useEffect(() => {
    fetch(`${resolveBase()}/api/auth/invite-required`)
      .then((res) => res.json())
      .then((data) => setInviteRequired(data.required))
      .catch(() => {});
  }, []);

  const handleSubmit = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    setError(null);
    setIsSubmitting(true);

    try {
      if (mode === 'register') {
        await register(username.trim(), password, inviteCode.trim());
      } else {
        await login(username.trim(), password);
      }
      router.replace(nextPath);
    } catch (err) {
      setError(err instanceof Error ? err.message : '操作失败，请重试');
    } finally {
      setIsSubmitting(false);
    }
  };

  return (
    <div className="min-h-screen flex items-center justify-center bg-background px-6">
      <Card className="w-full max-w-md">
        <CardHeader>
          <CardTitle>{mode === 'login' ? '登录' : '注册'}</CardTitle>
          <CardDescription>
            {mode === 'login' ? '输入账号和密码登录' : '创建一个新账号'}
          </CardDescription>
        </CardHeader>
        <CardContent>
          <form onSubmit={handleSubmit} className="space-y-4">
            <div className="space-y-2">
              <Label htmlFor="username">用户名</Label>
              <Input
                id="username"
                type="text"
                value={username}
                autoComplete="username"
                onChange={(event) => setUsername(event.target.value)}
                placeholder="3-32位字母数字下划线"
                required
              />
            </div>
            <div className="space-y-2">
              <Label htmlFor="password">密码</Label>
              <Input
                id="password"
                type="password"
                value={password}
                autoComplete={mode === 'register' ? 'new-password' : 'current-password'}
                onChange={(event) => setPassword(event.target.value)}
                placeholder="至少6位"
                required
              />
            </div>
            {mode === 'register' && inviteRequired && (
              <div className="space-y-2">
                <Label htmlFor="invite-code">邀请码</Label>
                <Input
                  id="invite-code"
                  type="text"
                  value={inviteCode}
                  onChange={(event) => setInviteCode(event.target.value)}
                  placeholder="输入邀请码"
                  required
                />
              </div>
            )}
            {error && (
              <div className="rounded-md border border-destructive/30 bg-destructive/10 px-3 py-2 text-sm text-destructive">
                {error}
              </div>
            )}
            <Button type="submit" className="w-full" disabled={isSubmitting}>
              {isSubmitting ? '请稍候...' : mode === 'login' ? '登录' : '注册'}
            </Button>
          </form>
          <div className="mt-4 text-center text-sm text-muted-foreground">
            {mode === 'login' ? (
              <>
                没有账号？{' '}
                <button
                  type="button"
                  className="underline text-primary hover:text-primary/80"
                  onClick={() => {
                    setMode('register');
                    setError(null);
                  }}
                >
                  注册
                </button>
              </>
            ) : (
              <>
                已有账号？{' '}
                <button
                  type="button"
                  className="underline text-primary hover:text-primary/80"
                  onClick={() => {
                    setMode('login');
                    setError(null);
                  }}
                >
                  登录
                </button>
              </>
            )}
          </div>
        </CardContent>
      </Card>
    </div>
  );
}
