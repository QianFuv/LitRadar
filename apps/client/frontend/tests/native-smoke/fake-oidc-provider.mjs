/**
 * Minimal OpenID Connect provider for the native single sign-on smoke tests.
 *
 * Serves discovery, an authorization page with "allow" and "deny" buttons, a token endpoint
 * that enforces `client_secret_basic`, the redirect URI, and the PKCE S256 verifier, and a
 * userinfo endpoint with a verified email. ID tokens are unsigned because the LitRadar server
 * receives them only over its back channel. Plain HTTP on loopback, so the server must run in
 * development mode.
 *
 * @packageDocumentation
 */

import { createHash, randomBytes } from 'node:crypto';
import http from 'node:http';

/** OAuth client registered with the fake provider. */
export const FAKE_OIDC_CLIENT_ID = 'litradar-smoke';
/** Secret of {@link FAKE_OIDC_CLIENT_ID}. */
export const FAKE_OIDC_CLIENT_SECRET = 'litradar-smoke-secret';
/** Display name configured for the provider in LitRadar. */
export const FAKE_OIDC_DISPLAY_NAME = 'Mock IdP';
/** Username the provider proposes as `preferred_username`. */
export const FAKE_OIDC_USERNAME = 'sso_reader';

/**
 * Encode JSON as unpadded base64url.
 *
 * @param value - JSON-serializable value.
 * @returns Encoded text.
 */
function encodeSegment(value) {
  return Buffer.from(JSON.stringify(value)).toString('base64url');
}

/**
 * Read a request body as text.
 *
 * @param request - Incoming request.
 * @returns Body text.
 */
async function readBody(request) {
  const chunks = [];
  for await (const chunk of request) {
    chunks.push(chunk);
  }
  return Buffer.concat(chunks).toString('utf8');
}

/**
 * Escape text for an HTML attribute.
 *
 * @param value - Raw text.
 * @returns Escaped text.
 */
function escapeAttribute(value) {
  return String(value).replace(/[&<>"']/g, (character) => `&#${character.charCodeAt(0)};`);
}

/**
 * Start the provider on a loopback port.
 *
 * @returns Issuer URL, authorization log, and stop function.
 */
export async function startFakeOidcProvider() {
  const grants = new Map();
  const accessTokens = new Map();
  const authorizations = [];
  let issuer = '';

  const server = http.createServer(async (request, response) => {
    const url = new URL(request.url ?? '/', issuer);
    const sendJson = (status, value) => {
      response.writeHead(status, {
        'Content-Type': 'application/json',
        'Cache-Control': 'no-store',
      });
      response.end(JSON.stringify(value));
    };
    if (request.method === 'GET' && url.pathname === '/.well-known/openid-configuration') {
      sendJson(200, {
        issuer,
        authorization_endpoint: `${issuer}authorize`,
        token_endpoint: `${issuer}token`,
        userinfo_endpoint: `${issuer}userinfo`,
        response_types_supported: ['code'],
        code_challenge_methods_supported: ['S256'],
      });
      return;
    }
    if (request.method === 'GET' && url.pathname === '/authorize') {
      const fields = [
        'client_id',
        'redirect_uri',
        'state',
        'nonce',
        'code_challenge',
        'code_challenge_method',
      ];
      const hidden = fields
        .map(
          (name) =>
            `<input type="hidden" name="${name}" value="${escapeAttribute(url.searchParams.get(name) ?? '')}">`,
        )
        .join('');
      response.writeHead(200, { 'Content-Type': 'text/html; charset=utf-8' });
      response.end(`<!doctype html><html lang="en"><head><meta charset="utf-8"><title>Mock IdP</title></head>
<body><main><h1>Mock IdP sign-in</h1><form method="post" action="/authorize">${hidden}
<label>Email <input name="email" value="sso.reader@example.com"></label>
<button name="decision" value="allow">Allow</button>
<button name="decision" value="deny">Deny</button></form></main></body></html>`);
      return;
    }
    if (request.method === 'POST' && url.pathname === '/authorize') {
      const form = new URLSearchParams(await readBody(request));
      const redirect = new URL(form.get('redirect_uri') ?? '');
      authorizations.push({ clientId: form.get('client_id'), decision: form.get('decision') });
      if (
        form.get('client_id') !== FAKE_OIDC_CLIENT_ID ||
        form.get('code_challenge_method') !== 'S256'
      ) {
        redirect.searchParams.set('error', 'invalid_request');
      } else if (form.get('decision') !== 'allow') {
        redirect.searchParams.set('error', 'access_denied');
      } else {
        const code = randomBytes(24).toString('base64url');
        grants.set(code, {
          redirectUri: form.get('redirect_uri'),
          challenge: form.get('code_challenge'),
          nonce: form.get('nonce'),
          email: form.get('email'),
        });
        redirect.searchParams.set('code', code);
      }
      redirect.searchParams.set('state', form.get('state') ?? '');
      response.writeHead(302, { Location: redirect.toString() });
      response.end();
      return;
    }
    if (request.method === 'POST' && url.pathname === '/token') {
      const form = new URLSearchParams(await readBody(request));
      const expectedBasic = `Basic ${Buffer.from(
        `${encodeURIComponent(FAKE_OIDC_CLIENT_ID)}:${encodeURIComponent(FAKE_OIDC_CLIENT_SECRET)}`,
      ).toString('base64')}`;
      const grant = grants.get(form.get('code') ?? '');
      grants.delete(form.get('code') ?? '');
      const verifierChallenge = createHash('sha256')
        .update(form.get('code_verifier') ?? '')
        .digest('base64url');
      if (
        request.headers.authorization !== expectedBasic ||
        form.get('grant_type') !== 'authorization_code' ||
        !grant ||
        grant.redirectUri !== form.get('redirect_uri') ||
        grant.challenge !== verifierChallenge
      ) {
        sendJson(400, { error: 'invalid_grant' });
        return;
      }
      const now = Math.floor(Date.now() / 1000);
      const accessToken = randomBytes(24).toString('base64url');
      const subject = `subject-${grant.email}`;
      accessTokens.set(accessToken, { subject, email: grant.email });
      const idToken = [
        encodeSegment({ alg: 'RS256', typ: 'JWT' }),
        encodeSegment({
          iss: issuer,
          sub: subject,
          aud: FAKE_OIDC_CLIENT_ID,
          exp: now + 300,
          iat: now,
          nonce: grant.nonce,
        }),
        'unsigned',
      ].join('.');
      sendJson(200, {
        access_token: accessToken,
        token_type: 'Bearer',
        expires_in: 300,
        id_token: idToken,
      });
      return;
    }
    if (request.method === 'GET' && url.pathname === '/userinfo') {
      const account = accessTokens.get(
        (request.headers.authorization ?? '').replace(/^Bearer /, ''),
      );
      if (!account) {
        sendJson(401, { error: 'invalid_token' });
        return;
      }
      sendJson(200, {
        sub: account.subject,
        email: account.email,
        email_verified: true,
        preferred_username: FAKE_OIDC_USERNAME,
      });
      return;
    }
    response.writeHead(404);
    response.end();
  });

  await new Promise((resolve) => server.listen(0, '127.0.0.1', resolve));
  issuer = `http://127.0.0.1:${server.address().port}/`;

  /** Stop the provider. */
  async function stop() {
    server.closeAllConnections();
    await new Promise((resolve) => server.close(resolve));
  }

  return { issuer, authorizations, stop };
}
