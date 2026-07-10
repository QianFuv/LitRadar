'use client';

import { useState } from 'react';
import { useMutation } from '@tanstack/react-query';

import { changePassword } from '@/lib/api';
import { useAuth } from '@/lib/auth-context';
import { Button } from '@/components/ui/button';
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card';
import { Input } from '@/components/ui/input';
import { Label } from '@/components/ui/label';

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
  const changePwdMut = useMutation({
    mutationFn: () => changePassword(oldPwd, newPwd),
    onSuccess: () => {
      setPwdMsg('密码修改成功，请重新登录');
      setTimeout(() => void logout(), 1500);
    },
    onError: (err) => setPwdMsg(err instanceof Error ? err.message : '修改失败'),
  });

  return (
    <Card>
      <CardHeader>
        <CardTitle>修改密码</CardTitle>
      </CardHeader>
      <CardContent>
        <form
          onSubmit={(e) => {
            e.preventDefault();
            setPwdMsg(null);
            changePwdMut.mutate();
          }}
          className="space-y-4 max-w-sm"
        >
          <div className="space-y-2">
            <Label htmlFor="old-password">原密码</Label>
            <Input
              id="old-password"
              name="old_password"
              type="password"
              autoComplete="current-password"
              value={oldPwd}
              onChange={(e) => setOldPwd(e.target.value)}
              aria-invalid={changePwdMut.isError}
              required
            />
          </div>
          <div className="space-y-2">
            <Label htmlFor="new-password">新密码</Label>
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
              required
            />
          </div>
          {pwdMsg && (
            <p
              role={changePwdMut.isError ? 'alert' : 'status'}
              className="text-sm text-muted-foreground"
            >
              {pwdMsg}
            </p>
          )}
          <Button type="submit" disabled={changePwdMut.isPending}>
            修改密码
          </Button>
        </form>
      </CardContent>
    </Card>
  );
}
