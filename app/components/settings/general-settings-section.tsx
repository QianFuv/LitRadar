'use client';

/**
 * Theme preference controls for the general settings category.
 */

import { Check } from 'lucide-react';
import { useTheme } from 'next-themes';
import { useSyncExternalStore } from 'react';

import {
  SettingsSection,
  SettingsSectionContent,
  SettingsSectionDescription,
  SettingsSectionHeader,
  SettingsSectionTitle,
} from '@/components/settings/settings-section';
import { cn } from '@/lib/utils';

type ThemePreference = 'system' | 'light' | 'dark';

type ThemeOption = {
  description: string;
  label: string;
  value: ThemePreference;
};

const THEME_OPTIONS: readonly ThemeOption[] = [
  { description: '跟随设备的浅色或深色外观', label: '跟随系统', value: 'system' },
  { description: '始终使用浅色界面', label: '浅色', value: 'light' },
  { description: '始终使用深色界面', label: '深色', value: 'dark' },
];

/**
 * Subscribe to the immutable client-environment signal.
 *
 * @returns No-op unsubscribe callback.
 */
function subscribeToClientEnvironment(): () => void {
  return () => undefined;
}

/**
 * Return the browser snapshot for hydration-safe client detection.
 *
 * @returns Always true in the browser.
 */
function getClientEnvironmentSnapshot(): boolean {
  return true;
}

/**
 * Return the server snapshot for hydration-safe client detection.
 *
 * @returns Always false during server rendering and hydration.
 */
function getServerEnvironmentSnapshot(): boolean {
  return false;
}

/**
 * Render the system, light, and dark theme preferences.
 *
 * @returns General settings theme section.
 */
export function GeneralSettingsSection() {
  const { setTheme, theme } = useTheme();
  const isMounted = useSyncExternalStore(
    subscribeToClientEnvironment,
    getClientEnvironmentSnapshot,
    getServerEnvironmentSnapshot,
  );
  const selectedTheme = isMounted ? (theme ?? 'system') : 'system';

  return (
    <SettingsSection>
      <SettingsSectionHeader>
        <SettingsSectionTitle>外观</SettingsSectionTitle>
        <SettingsSectionDescription>
          选择 LitRadar 的界面主题。该偏好会保存在当前浏览器中。
        </SettingsSectionDescription>
      </SettingsSectionHeader>
      <SettingsSectionContent>
        {isMounted ? (
          <div className="grid grid-cols-3 gap-3 sm:gap-4" role="radiogroup" aria-label="外观主题">
            {THEME_OPTIONS.map((option) => {
              const isSelected = selectedTheme === option.value;
              return (
                <button
                  key={option.value}
                  type="button"
                  role="radio"
                  aria-checked={isSelected}
                  aria-label={option.label}
                  className="group flex flex-col gap-2.5 rounded-xl text-left outline-none focus-visible:ring-[3px] focus-visible:ring-ring/50"
                  onClick={() => setTheme(option.value)}
                >
                  <span
                    className={cn(
                      'motion-control block aspect-[4/3] w-full overflow-hidden rounded-xl border bg-background transition-[box-shadow,border-color]',
                      isSelected
                        ? 'border-primary shadow-[0_0_0_2px_var(--primary)]'
                        : 'group-hover:border-foreground/30',
                    )}
                  >
                    <ThemeThumbnail preference={option.value} />
                  </span>
                  <span className="flex items-start gap-2 px-0.5">
                    <span
                      aria-hidden="true"
                      className={cn(
                        'mt-0.5 flex size-4 shrink-0 items-center justify-center rounded-full border',
                        isSelected && 'border-primary bg-primary text-primary-foreground',
                      )}
                    >
                      {isSelected && <Check className="size-2.5" strokeWidth={3} />}
                    </span>
                    <span className="min-w-0">
                      <span className="block text-sm leading-5 font-bold">{option.label}</span>
                      <span className="hidden text-xs leading-relaxed text-muted-foreground sm:block">
                        {option.description}
                      </span>
                    </span>
                  </span>
                </button>
              );
            })}
          </div>
        ) : (
          <p role="status" className="text-sm text-muted-foreground">
            正在读取主题偏好…
          </p>
        )}
      </SettingsSectionContent>
    </SettingsSection>
  );
}

/**
 * Render a miniature mock of the workspace in one fixed color scheme.
 *
 * @param props - Scheme to depict.
 * @returns Static workspace miniature.
 */
function ThemeMiniature({ scheme }: { scheme: 'light' | 'dark' }) {
  const isDark = scheme === 'dark';
  return (
    <span className={cn('flex h-full w-full', isDark ? 'bg-[#0a0a0a]' : 'bg-[#f4f4f4]')}>
      <span className="flex w-[16%] flex-col items-center gap-1.5 bg-[#111] pt-2.5">
        <i className="size-2 rounded-[3px] bg-[#2563eb]" />
        <i className="h-1 w-3 rounded-full bg-white/40" />
        <i className="h-1 w-3 rounded-full bg-white/20" />
        <i className="h-1 w-3 rounded-full bg-white/20" />
      </span>
      <span className="flex flex-1 gap-1.5 p-2">
        <span className="flex flex-[3] flex-col gap-1.5">
          {[0, 1, 2].map((index) => (
            <span
              key={index}
              className={cn(
                'flex flex-col gap-1 rounded-[3px] p-1.5',
                isDark ? 'bg-[#1a1a1a]' : 'bg-white',
              )}
            >
              <i
                className={cn(
                  'h-1 rounded-full',
                  index === 0 ? 'w-4/5 bg-[#2563eb]' : 'w-3/4',
                  index !== 0 && (isDark ? 'bg-white/50' : 'bg-black/45'),
                )}
              />
              <i className={cn('h-1 w-1/2 rounded-full', isDark ? 'bg-white/20' : 'bg-black/15')} />
            </span>
          ))}
        </span>
        <span
          className={cn(
            'flex flex-[2] flex-col gap-1 rounded-[3px] p-1.5',
            isDark ? 'bg-[#1a1a1a]' : 'bg-white',
          )}
        >
          <i className={cn('h-1.5 w-4/5 rounded-full', isDark ? 'bg-white/60' : 'bg-black/55')} />
          {[0, 1, 2, 3].map((index) => (
            <i
              key={index}
              className={cn('h-1 w-full rounded-full', isDark ? 'bg-white/15' : 'bg-black/10')}
            />
          ))}
        </span>
      </span>
    </span>
  );
}

/**
 * Render the preview artwork for one theme preference.
 *
 * @param props - Theme preference to depict.
 * @returns Theme thumbnail, split diagonally for the system preference.
 */
function ThemeThumbnail({ preference }: { preference: ThemePreference }) {
  if (preference !== 'system') return <ThemeMiniature scheme={preference} />;
  return (
    <span className="relative block h-full w-full">
      <ThemeMiniature scheme="light" />
      <span className="absolute inset-0 [clip-path:polygon(62%_0,100%_0,100%_100%,38%_100%)]">
        <ThemeMiniature scheme="dark" />
      </span>
    </span>
  );
}
