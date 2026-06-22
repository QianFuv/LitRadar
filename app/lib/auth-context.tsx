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
import { getCurrentUser, loginUser, logoutUser, registerUser, type AuthUser } from '@/lib/api';

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
const ACCESS_TOKEN_STORAGE_KEY = 'ps:v1:session_access_token';
const LEGACY_SESSION_ACCESS_TOKEN_KEY = 'ps_session_access_token';
const LEGACY_ACCESS_TOKEN_KEY = 'ps_access_token';
const USER_STORAGE_KEY = 'ps:v1:user';
const LEGACY_USER_STORAGE_KEY = 'ps_user';

/**
 * Read a localStorage value without assuming browser storage is available.
 *
 * @param key - Storage key.
 * @returns Stored value or null.
 */
function readLocalStorageValue(key: string): string | null {
  if (typeof window === 'undefined') {
    return null;
  }
  try {
    return window.localStorage.getItem(key);
  } catch {
    return null;
  }
}

/**
 * Write a localStorage value without surfacing quota or privacy-mode errors.
 *
 * @param key - Storage key.
 * @param value - Value to store.
 */
function writeLocalStorageValue(key: string, value: string): void {
  if (typeof window === 'undefined') {
    return;
  }
  try {
    window.localStorage.setItem(key, value);
  } catch {}
}

/**
 * Remove a localStorage value without assuming browser storage is available.
 *
 * @param key - Storage key.
 */
function removeLocalStorageValue(key: string): void {
  if (typeof window === 'undefined') {
    return;
  }
  try {
    window.localStorage.removeItem(key);
  } catch {}
}

/**
 * Read a sessionStorage value without assuming browser storage is available.
 *
 * @param key - Storage key.
 * @returns Stored value or null.
 */
function readSessionStorageValue(key: string): string | null {
  if (typeof window === 'undefined') {
    return null;
  }
  try {
    return window.sessionStorage.getItem(key);
  } catch {
    return null;
  }
}

/**
 * Write a sessionStorage value without surfacing quota or privacy-mode errors.
 *
 * @param key - Storage key.
 * @param value - Value to store.
 */
function writeSessionStorageValue(key: string, value: string): void {
  if (typeof window === 'undefined') {
    return;
  }
  try {
    window.sessionStorage.setItem(key, value);
  } catch {}
}

/**
 * Remove a sessionStorage value without assuming browser storage is available.
 *
 * @param key - Storage key.
 */
function removeSessionStorageValue(key: string): void {
  if (typeof window === 'undefined') {
    return;
  }
  try {
    window.sessionStorage.removeItem(key);
  } catch {}
}

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
  let rawUser = readLocalStorageValue(USER_STORAGE_KEY);
  const didReadLegacyUser = rawUser === null;
  if (rawUser === null) {
    rawUser = readLocalStorageValue(LEGACY_USER_STORAGE_KEY);
  }
  if (!rawUser) {
    return null;
  }
  try {
    const parsedUser: unknown = JSON.parse(rawUser);
    if (isAuthUser(parsedUser)) {
      if (didReadLegacyUser) {
        writeLocalStorageValue(USER_STORAGE_KEY, rawUser);
        removeLocalStorageValue(LEGACY_USER_STORAGE_KEY);
      }
      return parsedUser;
    }
    removeLocalStorageValue(USER_STORAGE_KEY);
    removeLocalStorageValue(LEGACY_USER_STORAGE_KEY);
    return null;
  } catch {
    removeLocalStorageValue(USER_STORAGE_KEY);
    removeLocalStorageValue(LEGACY_USER_STORAGE_KEY);
    return null;
  }
}

/**
 * Persist non-secret authenticated user metadata locally.
 *
 * @param user - Authenticated user.
 */
function writeStoredUser(user: AuthUser): void {
  writeLocalStorageValue(USER_STORAGE_KEY, JSON.stringify(user));
  removeLocalStorageValue(LEGACY_USER_STORAGE_KEY);
}

/**
 * Read a stored access token from the current browser tab session.
 *
 * @returns Stored access token or null.
 */
function readStoredAccessToken(): string | null {
  if (typeof window === 'undefined') {
    return null;
  }
  const sessionToken = readSessionStorageValue(ACCESS_TOKEN_STORAGE_KEY);
  if (sessionToken) {
    return sessionToken;
  }
  const legacySessionToken = readSessionStorageValue(LEGACY_SESSION_ACCESS_TOKEN_KEY);
  if (legacySessionToken) {
    writeSessionStorageValue(ACCESS_TOKEN_STORAGE_KEY, legacySessionToken);
    removeSessionStorageValue(LEGACY_SESSION_ACCESS_TOKEN_KEY);
    return legacySessionToken;
  }
  const legacyToken = readLocalStorageValue(LEGACY_ACCESS_TOKEN_KEY);
  if (!legacyToken) {
    return null;
  }
  writeSessionStorageValue(ACCESS_TOKEN_STORAGE_KEY, legacyToken);
  removeLocalStorageValue(LEGACY_ACCESS_TOKEN_KEY);
  return legacyToken;
}

/**
 * Store an access token for the current browser tab session.
 *
 * @param token - Access token returned by the backend.
 */
function writeStoredAccessToken(token: string): void {
  writeSessionStorageValue(ACCESS_TOKEN_STORAGE_KEY, token);
  removeSessionStorageValue(LEGACY_SESSION_ACCESS_TOKEN_KEY);
  removeLocalStorageValue(LEGACY_ACCESS_TOKEN_KEY);
}

/**
 * Remove access tokens stored by current and older frontend versions.
 */
function clearStoredAccessTokens(): void {
  removeSessionStorageValue(ACCESS_TOKEN_STORAGE_KEY);
  removeSessionStorageValue(LEGACY_SESSION_ACCESS_TOKEN_KEY);
  removeLocalStorageValue(LEGACY_ACCESS_TOKEN_KEY);
}

/**
 * Remove locally persisted non-secret session metadata and legacy tokens.
 */
function clearStoredSession(): void {
  clearStoredAccessTokens();
  removeLocalStorageValue(USER_STORAGE_KEY);
  removeLocalStorageValue(LEGACY_USER_STORAGE_KEY);
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
    let didCancel = false;

    const restoreSession = async () => {
      const storedToken = readStoredAccessToken();
      const storedUser = readStoredUser();

      if (!storedToken) {
        if (storedUser) {
          clearStoredSession();
        }
        if (!didCancel) {
          setLoading(false);
        }
        return;
      }

      try {
        const currentUser = await getCurrentUser(storedToken);
        if (didCancel) {
          return;
        }
        writeStoredUser(currentUser);
        setToken(storedToken);
        setUser(currentUser);
      } catch {
        clearStoredSession();
        if (!didCancel) {
          setToken(null);
          setUser(null);
          queryClient.clear();
        }
      } finally {
        if (!didCancel) {
          setLoading(false);
        }
      }
    };

    void restoreSession();

    return () => {
      didCancel = true;
    };
  }, [queryClient]);

  const login = useCallback(
    async (username: string, password: string) => {
      const response = await loginUser(username, password);
      queryClient.clear();
      writeStoredAccessToken(response.access_token);
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
