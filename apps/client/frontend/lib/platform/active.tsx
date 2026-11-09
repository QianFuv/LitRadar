/**
 * Platform selected for the current build.
 *
 * The web build resolves this module as written. The native build aliases it to
 * `lib/platform/native.tsx` in `vite.config.ts`, so the Wails runtime never enters
 * the web bundle.
 */

export { WEB_PLATFORM as ACTIVE_PLATFORM } from '@/lib/platform/web';
