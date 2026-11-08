/**
 * Active client platform for the current build.
 */

import type { ClientPlatform } from '@/lib/platform/types';
import { WEB_PLATFORM } from '@/lib/platform/web';

export type { ClientPlatform } from '@/lib/platform/types';

// ponytail: web build only; select by import.meta.env.MODE once the native platform exists.
/** Platform used by every shared client module. */
export const CLIENT_PLATFORM: ClientPlatform = WEB_PLATFORM;
