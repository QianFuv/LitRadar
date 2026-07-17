'use client';

/**
 * Theme preference controls for the general settings category.
 */

import { Check, Monitor, Moon, Sun, type LucideIcon } from 'lucide-react';
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
  icon: LucideIcon;
  label: string;
  value: ThemePreference;
};

const THEME_OPTIONS: readonly ThemeOption[] = [
  {
    description: '跟随设备的浅色或深色外观',
    icon: Monitor,
    label: '跟随系统',
    value: 'system',
  },
  { description: '始终使用浅色界面', icon: Sun, label: '浅色', value: 'light' },
  { description: '始终使用深色界面', icon: Moon, label: '深色', value: 'dark' },
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
          <div className="grid gap-3 sm:grid-cols-3" role="radiogroup" aria-label="外观主题">
            {THEME_OPTIONS.map((option) => {
              const Icon = option.icon;
              const isSelected = selectedTheme === option.value;
              return (
                <button
                  key={option.value}
                  type="button"
                  role="radio"
                  aria-checked={isSelected}
                  className={cn(
                    'relative flex min-h-24 flex-col items-start gap-2 rounded-md border p-4 text-left transition-colors outline-none hover:bg-accent focus-visible:ring-[3px] focus-visible:ring-ring/50',
                    isSelected && 'bg-accent text-accent-foreground',
                  )}
                  onClick={() => setTheme(option.value)}
                >
                  <div className="flex w-full items-center justify-between gap-3">
                    <Icon className="size-5" />
                    {isSelected && <Check className="size-4" aria-hidden="true" />}
                  </div>
                  <span className="text-sm font-medium">{option.label}</span>
                  <span className="text-xs leading-relaxed text-muted-foreground">
                    {option.description}
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
