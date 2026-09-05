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
  useRef,
  useState,
  type ReactNode,
} from 'react';
import {
  ApiError,
  getCurrentUser,
  loginUser,
  logoutAllSessions,
  logoutUser,
  registerUser,
  type AuthUser,
} from '@/lib/api';
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
  logoutWarning: LogoutRevocationWarning | null;
  login: (username: string, password: string) => Promise<void>;
  register: (username: string, password: string, inviteCode: string) => Promise<void>;
  logout: () => Promise<void>;
  recoverLogout: (username: string, password: string) => Promise<void>;
}

/**
 * Non-secret evidence that durable server-side logout was not confirmed.
 */
export interface LogoutRevocationWarning {
  occurredAt: number;
  requestId: string | null;
}

const AuthContext = createContext<AuthState | null>(null);
const ACCESS_TOKEN_STORAGE_KEY = 'litradar:v1:session_access_token';
const LOGOUT_WARNING_STORAGE_KEY = 'litradar:v1:logout_revocation_unconfirmed';
const USER_STORAGE_KEY = 'litradar:v1:user';

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
 * Read a persisted logout-revocation warning without trusting arbitrary display text.
 *
 * @returns Validated warning metadata or null.
 */
function readStoredLogoutWarning(): LogoutRevocationWarning | null {
  const rawWarning = readLocalStorageValue(LOGOUT_WARNING_STORAGE_KEY);
  if (!rawWarning) {
    return null;
  }
  try {
    const parsedWarning: unknown = JSON.parse(rawWarning);
    if (!parsedWarning || typeof parsedWarning !== 'object') {
      throw new TypeError('Invalid logout warning');
    }
    const warning = parsedWarning as Record<string, unknown>;
    if (
      typeof warning.occurredAt !== 'number' ||
      !Number.isFinite(warning.occurredAt) ||
      (warning.requestId !== null && typeof warning.requestId !== 'string')
    ) {
      throw new TypeError('Invalid logout warning');
    }
    return {
      occurredAt: warning.occurredAt,
      requestId: warning.requestId,
    };
  } catch {
    removeLocalStorageValue(LOGOUT_WARNING_STORAGE_KEY);
    return null;
  }
}

/**
 * Persist fixed-shape non-secret logout warning metadata.
 *
 * @param warning - Warning metadata to preserve across refreshes.
 */
function writeStoredLogoutWarning(warning: LogoutRevocationWarning): void {
  writeLocalStorageValue(LOGOUT_WARNING_STORAGE_KEY, JSON.stringify(warning));
}

/**
 * Remove a previously persisted logout warning after durable revocation succeeds.
 */
function clearStoredLogoutWarning(): void {
  removeLocalStorageValue(LOGOUT_WARNING_STORAGE_KEY);
}

/**
 * Convert a logout failure into fixed warning metadata.
 *
 * @param error - Failure returned by the logout transport.
 * @returns Non-secret warning metadata.
 */
function logoutWarningFromError(error: unknown): LogoutRevocationWarning {
  return {
    occurredAt: Date.now(),
    requestId: error instanceof ApiError ? error.requestId : null,
  };
}

/**
 * Provide authentication state and operations.
 *
 * @param props - Provider props.
 * @returns Authentication provider.
 */
export function AuthProvider({ children }: { children: ReactNode }) {
  const queryClient = useQueryClient();
  const sessionGeneration = useRef(0);
  const verifiedUser = useRef<AuthUser | null>(null);
  const [user, setUser] = useState<AuthUser | null>(null);
  const [loading, setLoading] = useState(true);
  const [logoutWarning, setLogoutWarning] = useState<LogoutRevocationWarning | null>(null);

  /** Hide private state and cancel requests before discarding their cached results. */
  const clearPrivateState = useCallback(() => {
    verifiedUser.current = null;
    setUser(null);
    void queryClient.cancelQueries();
    queryClient.clear();
  }, [queryClient]);

  /** Invalidate pending identity checks and clear the local session after logout. */
  const clearSession = useCallback(() => {
    sessionGeneration.current += 1;
    clearStoredSession();
    clearPrivateState();
    setLoading(false);
  }, [clearPrivateState]);

  useEffect(() => {
    let didCancel = false;

    /** Verify the shared cookie without trusting a stored identity or a stale response. */
    const restoreSession = async (shouldClearPrivateState = true) => {
      const generation = ++sessionGeneration.current;
      if (shouldClearPrivateState) {
        setLoading(true);
        clearPrivateState();
      }
      setLogoutWarning(readStoredLogoutWarning());
      try {
        const currentUser = await getCurrentUser();
        if (didCancel || generation !== sessionGeneration.current) {
          return;
        }
        const previousUser = verifiedUser.current;
        if (
          previousUser &&
          (previousUser.id !== currentUser.id ||
            previousUser.is_admin !== currentUser.is_admin ||
            previousUser.username !== currentUser.username)
        ) {
          clearPrivateState();
        }
        verifiedUser.current = currentUser;
        clearStoredAccessTokens();
        writeStoredUser(currentUser);
        setUser(currentUser);
      } catch {
        if (didCancel || generation !== sessionGeneration.current) {
          return;
        }
        clearStoredSession();
        clearPrivateState();
      } finally {
        if (!didCancel && generation === sessionGeneration.current) {
          setLoading(false);
        }
      }
    };

    /** Treat another document's session metadata only as a server verification signal. */
    const handleSessionStorage = (event: StorageEvent) => {
      if (
        event.key === null ||
        event.key === USER_STORAGE_KEY ||
        event.key === LOGOUT_WARNING_STORAGE_KEY
      ) {
        void restoreSession();
      }
    };

    /** Recheck expired or externally changed cookies when the document becomes visible. */
    const handleSessionVisibility = () => {
      if (document.visibilityState === 'visible') {
        void restoreSession(false);
      }
    };

    window.addEventListener('storage', handleSessionStorage);
    document.addEventListener('visibilitychange', handleSessionVisibility);
    void restoreSession();

    return () => {
      didCancel = true;
      sessionGeneration.current += 1;
      window.removeEventListener('storage', handleSessionStorage);
      document.removeEventListener('visibilitychange', handleSessionVisibility);
    };
  }, [clearPrivateState]);

  const login = useCallback(
    async (username: string, password: string) => {
      const response = await loginUser(username, password);
      sessionGeneration.current += 1;
      clearPrivateState();
      clearStoredAccessTokens();
      writeStoredUser(response.user);
      verifiedUser.current = response.user;
      setUser(response.user);
      setLoading(false);
    },
    [clearPrivateState],
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
      clearStoredLogoutWarning();
      setLogoutWarning(null);
    } catch (error) {
      const warning = logoutWarningFromError(error);
      writeStoredLogoutWarning(warning);
      setLogoutWarning(warning);
      throw error;
    } finally {
      clearSession();
    }
  }, [clearSession]);

  const recoverLogout = useCallback(
    async (username: string, password: string) => {
      await loginUser(username, password);
      try {
        await logoutAllSessions();
        clearStoredLogoutWarning();
        setLogoutWarning(null);
      } catch (error) {
        const warning = logoutWarningFromError(error);
        writeStoredLogoutWarning(warning);
        setLogoutWarning(warning);
        throw error;
      } finally {
        clearSession();
      }
    },
    [clearSession],
  );

  const value = useMemo(
    () => ({ user, loading, logoutWarning, login, register, logout, recoverLogout }),
    [loading, login, logout, logoutWarning, recoverLogout, register, user],
  );

  return (
    <AuthContext.Provider key={user?.id ?? 'anonymous'} value={value}>
      {children}
    </AuthContext.Provider>
  );
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
