'use client';

import { createContext, useCallback, useContext, useEffect, useMemo, useState } from 'react';
import { useQueryClient } from '@tanstack/react-query';

export interface AuthUser {
  id: number;
  username: string;
  is_admin?: boolean;
}

interface AuthState {
  user: AuthUser | null;
  token: string | null;
  loading: boolean;
  login: (username: string, password: string) => Promise<void>;
  register: (username: string, password: string, inviteCode: string) => Promise<void>;
  logout: () => Promise<void>;
}

const AuthContext = createContext<AuthState | null>(null);

const TOKEN_KEY = 'ps_access_token';
const USER_KEY = 'ps_user';

function getStoredToken(): string | null {
  if (typeof window === 'undefined') return null;
  return localStorage.getItem(TOKEN_KEY);
}

function getStoredUser(): AuthUser | null {
  if (typeof window === 'undefined') return null;
  try {
    const raw = localStorage.getItem(USER_KEY);
    return raw ? JSON.parse(raw) : null;
  } catch {
    return null;
  }
}

const API_BASE_URL = process.env.NEXT_PUBLIC_API_URL || '';

function resolveBase(): string {
  if (API_BASE_URL) return API_BASE_URL;
  if (typeof window !== 'undefined') return window.location.origin;
  return 'http://localhost:8000';
}

export function AuthProvider({ children }: { children: React.ReactNode }) {
  const queryClient = useQueryClient();
  const [user, setUser] = useState<AuthUser | null>(null);
  const [token, setToken] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);

  const clearSession = useCallback(() => {
    localStorage.removeItem(TOKEN_KEY);
    localStorage.removeItem(USER_KEY);
    setToken(null);
    setUser(null);
    queryClient.clear();
  }, [queryClient]);

  useEffect(() => {
    const storedToken = getStoredToken();
    const storedUser = getStoredUser();
    if (!storedToken || !storedUser) {
      setLoading(false);
      return;
    }
    setToken(storedToken);
    setUser(storedUser);
    fetch(`${resolveBase()}/api/auth/me`, {
      headers: { Authorization: `Bearer ${storedToken}` },
    })
      .then((res) => {
        if (!res.ok) {
          clearSession();
          return;
        }
        return res.json();
      })
      .then((data) => {
        if (data) {
          const refreshed: AuthUser = {
            id: data.id,
            username: data.username,
            is_admin: data.is_admin,
          };
          localStorage.setItem(USER_KEY, JSON.stringify(refreshed));
          setUser(refreshed);
        }
      })
      .catch(() => {})
      .finally(() => setLoading(false));
  }, [clearSession]);

  const login = useCallback(async (username: string, password: string) => {
    const res = await fetch(`${resolveBase()}/api/auth/login`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ username, password }),
    });
    if (!res.ok) {
      const payload = await res.json().catch(() => ({}));
      throw new Error(payload.detail || '登录失败');
    }
    const data = await res.json();
    const newToken = data.access_token as string;
    const newUser: AuthUser = data.user;
    queryClient.clear();
    localStorage.setItem(TOKEN_KEY, newToken);
    localStorage.setItem(USER_KEY, JSON.stringify(newUser));
    setToken(newToken);
    setUser(newUser);
  }, [queryClient]);

  const register = useCallback(async (username: string, password: string, inviteCode: string) => {
    const res = await fetch(`${resolveBase()}/api/auth/register`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ username, password, invite_code: inviteCode }),
    });
    if (!res.ok) {
      const payload = await res.json().catch(() => ({}));
      throw new Error(payload.detail || '注册失败');
    }
    await login(username, password);
  }, [login]);

  const logout = useCallback(async () => {
    const activeToken = token || getStoredToken();
    try {
      if (activeToken) {
        await fetch(`${resolveBase()}/api/auth/logout`, {
          method: 'POST',
          headers: { Authorization: `Bearer ${activeToken}` },
        });
      }
    } finally {
      clearSession();
    }
  }, [clearSession, token]);

  const value = useMemo(
    () => ({ user, token, loading, login, register, logout }),
    [user, token, loading, login, register, logout],
  );

  return <AuthContext.Provider value={value}>{children}</AuthContext.Provider>;
}

export function useAuth(): AuthState {
  const ctx = useContext(AuthContext);
  if (!ctx) throw new Error('useAuth must be inside AuthProvider');
  return ctx;
}

/** Helper to get auth headers for API requests */
export function authHeaders(token: string | null): Record<string, string> {
  if (!token) return {};
  return { Authorization: `Bearer ${token}` };
}
