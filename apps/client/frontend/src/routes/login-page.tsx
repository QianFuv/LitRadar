/**
 * Public authentication route and document metadata.
 */

import LoginClient from '@/src/routes/login-client';

/** Document title rendered by the login route. */
export const LOGIN_PAGE_TITLE = '登录 | LitRadar';

/** Document description rendered by the login route. */
export const LOGIN_PAGE_DESCRIPTION = '登录或注册 LitRadar 账号。';

/**
 * Render the login client with route-specific document metadata.
 *
 * @returns Login route content.
 */
export default function LoginPage() {
  return (
    <>
      <title>{LOGIN_PAGE_TITLE}</title>
      <meta name="description" content={LOGIN_PAGE_DESCRIPTION} />
      <LoginClient />
    </>
  );
}
