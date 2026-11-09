/**
 * Active client platform for the current build.
 */

import { ACTIVE_PLATFORM } from '@/lib/platform/active';
import type { ClientPlatform } from '@/lib/platform/types';

export type { ClientPlatform, NativeSession, ServerConnection } from '@/lib/platform/types';

/** Platform used by every shared client module. */
export const CLIENT_PLATFORM: ClientPlatform = ACTIVE_PLATFORM;
