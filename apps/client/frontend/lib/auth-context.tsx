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
import { z } from 'zod';
import {
  ApiError,
  getCurrentUser,
  loginUser,
  loginWithNativeOidc,
  logoutAllSessions,
  logoutUser,
  registerUser,
  type AuthUser,
} from '@/lib/api';
import {
  parseStoredJson,
  readLocalStorageValue,
  removeLocalStorageValue,
  removeSessionStorageValue,
  writeLocalStorageValue,
} from '@/lib/browser-storage';
import { CLIENT_PLATFORM } from '@/lib/platform';

export type { AuthUser };

interface AuthState {
  user: AuthUser | null;
  loading: boolean;
  logoutWarning: LogoutRevocationWarning | null;
  login: (username: string, password: string) => Promise<void>;
  signInWithNativeOidc: (signal: AbortSignal) => Promise<void>;
  register: (
    username: string,
    password: string,
    inviteCode: string,
    email?: string,
  ) => Promise<void>;
  logout: () => Promise<void>;
  recoverLogout: (username: string, password: string) => Promise<void>;
  updateUser: (nextUser: AuthUser) => void;
}

/** Stored shape of the logout-revocation warning; arbitrary display text is never accepted. */
const LOGOUT_REVOCATION_WARNING_SCHEMA = z.object({
  occurredAt: z.number(),
  requestId: z.string().nullable(),
});

/**
 * Non-secret evidence that durable server-side logout was not confirmed.
 */
export type LogoutRevocationWarning = z.output<typeof LOGOUT_REVOCATION_WARNING_SCHEMA>;

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
export function clearStoredSession(): void {
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
  const warning = parseStoredJson(rawWarning, LOGOUT_REVOCATION_WARNING_SCHEMA);
  if (!warning) {
    removeLocalStorageValue(LOGOUT_WARNING_STORAGE_KEY);
  }
  return warning;
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
 * Ask a native host to delete a stored token that is revoked or no longer accepted.
 */
function discardNativeCredential(): void {
  CLIENT_PLATFORM.nativeSession?.discardCredential().catch(() => undefined);
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
    discardNativeCredential();
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
      } catch (error) {
        if (didCancel || generation !== sessionGeneration.current) {
          return;
        }
        if (error instanceof ApiError && error.status === 401) {
          discardNativeCredential();
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

  /** Adopt a freshly authenticated identity after discarding any previous private state. */
  const adoptSignedInUser = useCallback(
    (signedInUser: AuthUser) => {
      sessionGeneration.current += 1;
      clearPrivateState();
      clearStoredAccessTokens();
      writeStoredUser(signedInUser);
      verifiedUser.current = signedInUser;
      setUser(signedInUser);
      setLoading(false);
    },
    [clearPrivateState],
  );

  const login = useCallback(
    async (username: string, password: string) => {
      const response = await loginUser(username, password);
      adoptSignedInUser(response.user);
    },
    [adoptSignedInUser],
  );

  const signInWithNativeOidc = useCallback(
    async (signal: AbortSignal) => {
      const response = await loginWithNativeOidc(signal);
      adoptSignedInUser(response.user);
    },
    [adoptSignedInUser],
  );

  const register = useCallback(
    async (username: string, password: string, inviteCode: string, email = '') => {
      await registerUser(username, password, inviteCode, email);
      await login(username, password);
    },
    [login],
  );

  /** Adopt a server-returned projection of the verified user after an account change. */
  const updateUser = useCallback((nextUser: AuthUser) => {
    if (verifiedUser.current?.id !== nextUser.id) {
      return;
    }
    verifiedUser.current = nextUser;
    writeStoredUser(nextUser);
    setUser(nextUser);
  }, []);

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
    () => ({
      user,
      loading,
      logoutWarning,
      login,
      signInWithNativeOidc,
      register,
      logout,
      recoverLogout,
      updateUser,
    }),
    [
      loading,
      login,
      logout,
      logoutWarning,
      recoverLogout,
      register,
      signInWithNativeOidc,
      updateUser,
      user,
    ],
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
