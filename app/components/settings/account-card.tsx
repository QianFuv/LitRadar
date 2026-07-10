import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card';

/**
 * Render immutable account identity information.
 *
 * @param props - Authenticated username.
 * @returns Account information card.
 */
export function AccountCard({ username }: { username: string }) {
  return (
    <Card>
      <CardHeader>
        <CardTitle>账号信息</CardTitle>
      </CardHeader>
      <CardContent>
        <div className="text-sm">
          用户名: <span className="font-medium">{username}</span>
        </div>
      </CardContent>
    </Card>
  );
}
