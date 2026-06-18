'use client';

/**
 * Authentication context for the restored pre-desktop frontend.
 */

import { useQueryClient } from '@tanstack/react-query';
import {
  createContext,
  use,
  useCallback,
  useEffect,
  useMemo,
  useState,
  type ReactNode,
} from 'react';
import { loginUser, logoutUser, registerUser, type AuthUser } from '@/lib/api';

export type { AuthUser };

interface AuthState {
  user: AuthUser | null;
  token: string | null;
  loading: boolean;
  login: (username: string, password: string) => Promise<void>;
  register: (username: string, password: string, inviteCode: string) => Promise<void>;
  logout: () => Promise<void>;
}

const AuthContext = createContext<AuthState | null>(null);
const LEGACY_ACCESS_TOKEN_KEY = 'ps_access_token';
const USER_STORAGE_KEY = 'ps_user';

/**
 * Check whether a parsed value matches the stored user shape.
 *
 * @param value - Parsed storage value.
 * @returns Whether the value is an auth user.
 */
function isAuthUser(value: unknown): value is AuthUser {
  if (!value || typeof value !== 'object') {
    return false;
  }
  const user = value as Record<string, unknown>;
  return (
    typeof user.id === 'number' &&
    typeof user.username === 'string' &&
    (user.is_admin === undefined || typeof user.is_admin === 'boolean')
  );
}

/**
 * Read the stored authenticated user snapshot.
 *
 * @returns User snapshot or null.
 */
function readStoredUser(): AuthUser | null {
  if (typeof window === 'undefined') {
    return null;
  }
  const rawUser = window.localStorage.getItem(USER_STORAGE_KEY);
  if (!rawUser) {
    return null;
  }
  try {
    const parsedUser: unknown = JSON.parse(rawUser);
    if (isAuthUser(parsedUser)) {
      return parsedUser;
    }
    window.localStorage.removeItem(USER_STORAGE_KEY);
    return null;
  } catch {
    window.localStorage.removeItem(USER_STORAGE_KEY);
    return null;
  }
}

/**
 * Persist non-secret authenticated user metadata locally.
 *
 * @param user - Authenticated user.
 */
function writeStoredUser(user: AuthUser): void {
  window.localStorage.setItem(USER_STORAGE_KEY, JSON.stringify(user));
}

/**
 * Remove access tokens written by older frontend versions.
 */
function clearLegacyStoredToken(): void {
  window.localStorage.removeItem(LEGACY_ACCESS_TOKEN_KEY);
}

/**
 * Remove locally persisted non-secret session metadata and legacy tokens.
 */
function clearStoredSession(): void {
  clearLegacyStoredToken();
  window.localStorage.removeItem(USER_STORAGE_KEY);
}

/**
 * Provide authentication state and operations.
 *
 * @param props - Provider props.
 * @returns Authentication provider.
 */
export function AuthProvider({ children }: { children: ReactNode }) {
  const queryClient = useQueryClient();
  const [user, setUser] = useState<AuthUser | null>(null);
  const [token, setToken] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);

  const clearSession = useCallback(() => {
    clearStoredSession();
    setUser(null);
    setToken(null);
    queryClient.clear();
  }, [queryClient]);

  useEffect(() => {
    clearLegacyStoredToken();
    readStoredUser();
    setLoading(false);
  }, []);

  const login = useCallback(
    async (username: string, password: string) => {
      const response = await loginUser(username, password);
      queryClient.clear();
      writeStoredUser(response.user);
      setToken(response.access_token);
      setUser(response.user);
    },
    [queryClient],
  );

  const register = useCallback(
    async (username: string, password: string, inviteCode: string) => {
      await registerUser(username, password, inviteCode);
      await login(username, password);
    },
    [login],
  );

  const logout = useCallback(async () => {
    const activeToken = token;
    try {
      if (activeToken) {
        await logoutUser(activeToken);
      }
    } finally {
      clearSession();
    }
  }, [clearSession, token]);

  const value = useMemo(
    () => ({ user, token, loading, login, register, logout }),
    [loading, login, logout, register, token, user],
  );

  return <AuthContext.Provider value={value}>{children}</AuthContext.Provider>;
}

/**
 * Read the restored frontend authentication state.
 *
 * @returns Authentication state.
 */
export function useAuth(): AuthState {
  const context = use(AuthContext);
  if (!context) {
    throw new Error('useAuth must be used inside AuthProvider');
  }
  return context;
}

/**
 * Build bearer authorization headers.
 *
 * @param token - Access token.
 * @returns Headers containing bearer auth when a token is available.
 */
export function authHeaders(token: string | null): Record<string, string> {
  if (!token) {
    return {};
  }
  return { Authorization: `Bearer ${token}` };
}
