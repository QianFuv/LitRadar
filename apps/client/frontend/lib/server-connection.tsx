/**
 * Native server selection state shared between the setup gate and signed-in screens.
 */

import { createContext, use } from 'react';

import { PlatformBridgeError } from '@/lib/platform/errors';

/** Active server selection exposed to the signed-in application. */
export interface ServerConnectionState {
  /** Canonical origin of the connected server. */
  origin: string;
  /** Revoke and delete the local token, forget the server, and return to setup. */
  switchServer: () => Promise<void>;
}

/** Context provided by the native server setup gate; null in the browser. */
export const ServerConnectionContext = createContext<ServerConnectionState | null>(null);

const SERVER_ERROR_MESSAGES = new Map<string, string>([
  ['invalid_origin', '服务器地址无效，请输入形如 https://litradar.example.com 的地址。'],
  ['insecure_origin', '仅支持 https:// 地址；http:// 只能用于本机地址（127.0.0.1、localhost）。'],
  ['server_unreachable', '无法连接到该服务器，请检查地址和网络。'],
  ['not_litradar_server', '该地址不是 LitRadar 服务器。'],
  ['server_not_ready', '服务器暂未就绪，请稍后重试。'],
  ['storage_failure', '无法保存服务器配置，请重试。'],
]);

/**
 * Convert a server setup failure into a user-facing message.
 *
 * @param error - Failure raised while connecting.
 * @returns Actionable message.
 */
export function getServerConnectionErrorMessage(error: unknown): string {
  if (error instanceof PlatformBridgeError) {
    return SERVER_ERROR_MESSAGES.get(error.code) ?? '连接失败，请重试。';
  }
  return '连接失败，请重试。';
}

/**
 * Read the active server selection.
 *
 * @returns Server selection on native hosts, or null in the browser.
 */
export function useServerConnection(): ServerConnectionState | null {
  return use(ServerConnectionContext);
}
