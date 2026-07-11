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
import {
  readLocalStorageValue,
  removeLocalStorageValue,
  removeSessionStorageValue,
  writeLocalStorageValue,
} from '@/lib/browser-storage';

export type { AuthUser };

interface AuthState {
  user: AuthUser | null;
  loading: boolean;
  login: (username: string, password: string) => Promise<void>;
  register: (username: string, password: string, inviteCode: string) => Promise<void>;
  logout: () => Promise<void>;
}

const AuthContext = createContext<AuthState | null>(null);
const ACCESS_TOKEN_STORAGE_KEY = 'litradar:v1:session_access_token';
const USER_STORAGE_KEY = 'litradar:v1:user';

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
    typeof user.is_admin === 'boolean'
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
  const rawUser = readLocalStorageValue(USER_STORAGE_KEY);
  if (!rawUser) {
    return null;
  }
  try {
    const parsedUser: unknown = JSON.parse(rawUser);
    if (isAuthUser(parsedUser)) {
      return parsedUser;
    }
    removeLocalStorageValue(USER_STORAGE_KEY);
    return null;
  } catch {
    removeLocalStorageValue(USER_STORAGE_KEY);
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
}

/**
 * Remove access tokens stored in the current frontend namespace.
 */
function clearStoredAccessTokens(): void {
  removeSessionStorageValue(ACCESS_TOKEN_STORAGE_KEY);
}

/**
 * Remove locally persisted non-secret session metadata and access tokens.
 */
function clearStoredSession(): void {
  clearStoredAccessTokens();
  removeLocalStorageValue(USER_STORAGE_KEY);
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
  const [loading, setLoading] = useState(true);

  const clearSession = useCallback(() => {
    clearStoredSession();
    setUser(null);
    queryClient.clear();
  }, [queryClient]);

  useEffect(() => {
    let didCancel = false;

    const restoreSession = async () => {
      const storedUser = readStoredUser();
      if (storedUser && !didCancel) {
        setUser(storedUser);
      }

      try {
        const currentUser = await getCurrentUser();
        if (didCancel) {
          return;
        }
        clearStoredAccessTokens();
        writeStoredUser(currentUser);
        setUser(currentUser);
      } catch {
        clearStoredSession();
        if (!didCancel) {
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
      clearStoredAccessTokens();
      writeStoredUser(response.user);
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
    try {
      await logoutUser();
    } finally {
      clearSession();
    }
  }, [clearSession]);

  const value = useMemo(
    () => ({ user, loading, login, register, logout }),
    [loading, login, logout, register, user],
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
