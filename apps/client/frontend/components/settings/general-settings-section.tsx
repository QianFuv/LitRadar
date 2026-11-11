/**
 * Theme preference controls for the general settings category.
 */

import { Check, SunMoon } from '@/components/ui/animated-icons';
import { useTheme } from 'next-themes';
import { useRef, type PointerEvent } from 'react';

import {
  SettingsSection,
  SettingsSectionContent,
  SettingsSectionDescription,
  SettingsSectionHeader,
  SettingsSectionTitle,
} from '@/components/settings/settings-section';
import { RadioGroup, RadioGroupIndicator, RadioGroupItem } from '@/components/ui/radio-group';
import { getFocusedOrigin, revealThemeChange, type ScreenPoint } from '@/lib/theme-transition';
import * as stylex from '@stylexjs/stylex';
import { colors } from '@/lib/theme.stylex';

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
 * Render the system, light, and dark theme preferences. A new theme is revealed as a circle
 * growing from where it was picked: the pointer position, or the focused option for keyboards.
 *
 * @returns General settings theme section.
 */
export function GeneralSettingsSection() {
  const { setTheme, theme } = useTheme();
  const pointerOriginRef = useRef<ScreenPoint | null>(null);

  /**
   * Remember where a pointer pressed the theme options.
   *
   * @param event - Pointer press inside the theme options.
   */
  function handleThemePointerDown(event: PointerEvent<HTMLDivElement>): void {
    pointerOriginRef.current = { x: event.clientX, y: event.clientY };
  }

  /**
   * Reveal the selected theme from where it was picked.
   *
   * @param value - Selected next-themes preference.
   */
  function handleThemeChange(value: string): void {
    const origin = pointerOriginRef.current ?? getFocusedOrigin();
    pointerOriginRef.current = null;
    revealThemeChange(() => setTheme(value), origin);
  }
  const selectedTheme = theme ?? 'system';

  return (
    <SettingsSection>
      <SettingsSectionHeader>
        <SettingsSectionTitle icon={SunMoon}>外观</SettingsSectionTitle>
        <SettingsSectionDescription>
          选择 LitRadar 的界面主题。该偏好会保存在当前浏览器中。
        </SettingsSectionDescription>
      </SettingsSectionHeader>
      <SettingsSectionContent>
        <RadioGroup
          xstyle={styles.generalSettingsSectionRadioGroup}
          aria-label="外观主题"
          value={selectedTheme}
          onValueChange={handleThemeChange}
          onPointerDown={handleThemePointerDown}
        >
          {THEME_OPTIONS.map((option) => {
            const isSelected = selectedTheme === option.value;
            return (
              <RadioGroupItem
                key={option.value}
                value={option.value}
                aria-label={option.label}
                {...stylex.props(
                  styles.generalSettingsSectionRadioGroupItem,
                  stylex.defaultMarker(),
                )}
              >
                <span
                  {...stylex.props(
                    styles.generalSettingsSectionSpan,
                    isSelected
                      ? styles.generalSettingsSectionSpanSelected
                      : styles.generalSettingsSectionSpanNotSelected,
                  )}
                >
                  <ThemeThumbnail preference={option.value} />
                </span>
                <span {...stylex.props(styles.generalSettingsSectionSpan2)}>
                  <span
                    aria-hidden="true"
                    {...stylex.props(
                      styles.generalSettingsSectionSpan3,
                      isSelected && styles.generalSettingsSectionSpanSelected2,
                    )}
                  >
                    <RadioGroupIndicator
                      {...stylex.props(styles.generalSettingsSectionRadioGroupIndicator)}
                    >
                      <Check xstyle={styles.generalSettingsSectionCheck} strokeWidth={3} />
                    </RadioGroupIndicator>
                  </span>
                  <span {...stylex.props(styles.generalSettingsSectionSpan4)}>
                    <span {...stylex.props(styles.generalSettingsSectionSpan5)}>
                      {option.label}
                    </span>
                    <span {...stylex.props(styles.generalSettingsSectionSpan6)}>
                      {option.description}
                    </span>
                  </span>
                </span>
              </RadioGroupItem>
            );
          })}
        </RadioGroup>
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
    <span
      {...stylex.props(
        styles.themeMiniatureSpan,
        isDark ? styles.themeMiniatureSpanDark : styles.themeMiniatureSpanNotDark,
      )}
    >
      <span {...stylex.props(styles.themeMiniatureSpan2)}>
        <i {...stylex.props(styles.themeMiniatureI)} />
        <i {...stylex.props(styles.themeMiniatureI2)} />
        <i {...stylex.props(styles.themeMiniatureI3)} />
        <i {...stylex.props(styles.themeMiniatureI3)} />
      </span>
      <span {...stylex.props(styles.themeMiniatureSpan3)}>
        <span {...stylex.props(styles.themeMiniatureSpan4)}>
          {[0, 1, 2].map((index) => (
            <span
              key={index}
              {...stylex.props(
                styles.themeMiniatureSpan5,
                isDark ? styles.themeMiniatureSpanDark2 : styles.themeMiniatureSpanNotDark2,
              )}
            >
              <i
                {...stylex.props(
                  styles.themeMiniatureI4,
                  index === 0 ? styles.themeMiniatureIFirst : styles.themeMiniatureIRest,
                  index !== 0 &&
                    (isDark ? styles.themeMiniatureIRestDark : styles.themeMiniatureIRestLight),
                )}
              />
              <i
                {...stylex.props(
                  styles.themeMiniatureI5,
                  isDark ? styles.themeMiniatureIDark : styles.themeMiniatureINotDark,
                )}
              />
            </span>
          ))}
        </span>
        <span
          {...stylex.props(
            styles.themeMiniatureSpan6,
            isDark ? styles.themeMiniatureSpanDark2 : styles.themeMiniatureSpanNotDark2,
          )}
        >
          <i
            {...stylex.props(
              styles.themeMiniatureI6,
              isDark ? styles.themeMiniatureIDark2 : styles.themeMiniatureINotDark2,
            )}
          />
          {[0, 1, 2, 3].map((index) => (
            <i
              key={index}
              {...stylex.props(
                styles.themeMiniatureI7,
                isDark ? styles.themeMiniatureIDark3 : styles.themeMiniatureINotDark3,
              )}
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
    <span {...stylex.props(styles.themeThumbnailSpan)}>
      <ThemeMiniature scheme="light" />
      <span {...stylex.props(styles.themeThumbnailSpan2)}>
        <ThemeMiniature scheme="dark" />
      </span>
    </span>
  );
}

const styles = stylex.create({
  generalSettingsSectionRadioGroup: {
    display: 'grid',
    gridTemplateColumns: 'repeat(3, minmax(0, 1fr))',
    gap: {
      default: '0.75rem',
      '@media (min-width: 40rem)': '1rem',
    },
  },
  generalSettingsSectionRadioGroupItem: {
    display: 'flex',
    flexDirection: 'column',
    gap: '0.625rem',
    borderRadius: '12px',
    textAlign: 'left',
    outlineStyle: 'none',
    boxShadow: {
      default: null,
      ':focus-visible': `0 0 0 3px color-mix(in oklab, ${colors.ring} 50%, transparent)`,
    },
  },
  generalSettingsSectionSpan: {
    display: 'block',
    aspectRatio: '4/3',
    width: '100%',
    overflow: 'hidden',
    borderRadius: '12px',
    borderStyle: 'solid',
    borderWidth: '1px',
    backgroundColor: colors.background,
    transitionProperty: 'box-shadow, border-color',
    transitionTimingFunction: 'cubic-bezier(0.25, 0.1, 0.25, 1)',
    transitionDuration: '200ms',
  },
  generalSettingsSectionSpanSelected: {
    borderColor: colors.primary,
    boxShadow: `0 0 0 2px ${colors.primary}`,
  },
  generalSettingsSectionSpanNotSelected: {
    borderColor: {
      default: null,
      '@media (hover: hover)': {
        default: null,
        [stylex.when.ancestor(':hover')]:
          `color-mix(in oklab, ${colors.foreground} 30%, transparent)`,
      },
    },
  },
  generalSettingsSectionSpan2: {
    display: 'flex',
    alignItems: 'flex-start',
    gap: '0.5rem',
    paddingInline: '0.125rem',
  },
  generalSettingsSectionSpan3: {
    marginTop: '0.125rem',
    display: 'flex',
    width: '1rem',
    height: '1rem',
    flexShrink: 0,
    alignItems: 'center',
    justifyContent: 'center',
    borderRadius: 'calc(infinity * 1px)',
    borderStyle: 'solid',
    borderWidth: '1px',
  },
  generalSettingsSectionSpanSelected2: {
    borderColor: colors.primary,
    backgroundColor: colors.primary,
    color: colors.primaryForeground,
  },
  generalSettingsSectionRadioGroupIndicator: {
    display: 'flex',
    alignItems: 'center',
    justifyContent: 'center',
  },
  generalSettingsSectionCheck: {
    width: '0.625rem',
    height: '0.625rem',
  },
  generalSettingsSectionSpan4: {
    minWidth: '0px',
  },
  generalSettingsSectionSpan5: {
    display: 'block',
    fontSize: '0.875rem',
    lineHeight: '1.25rem',
    fontWeight: 700,
  },
  generalSettingsSectionSpan6: {
    display: {
      default: 'none',
      '@media (min-width: 40rem)': 'block',
    },
    fontSize: '0.75rem',
    lineHeight: 1.625,
    color: colors.mutedForeground,
  },
  themeMiniatureSpan: {
    display: 'flex',
    height: '100%',
    width: '100%',
  },
  themeMiniatureSpanDark: {
    backgroundColor: '#141414',
  },
  themeMiniatureSpanNotDark: {
    backgroundColor: '#f6f6f6',
  },
  themeMiniatureSpan2: {
    display: 'flex',
    width: '16%',
    flexDirection: 'column',
    alignItems: 'center',
    gap: '0.375rem',
    backgroundColor: '#1c1c1c',
    paddingTop: '0.625rem',
  },
  themeMiniatureI: {
    width: '0.5rem',
    height: '0.5rem',
    borderRadius: '3px',
    backgroundColor: '#346bf1',
  },
  themeMiniatureI2: {
    height: '0.25rem',
    width: '0.75rem',
    borderRadius: 'calc(infinity * 1px)',
    backgroundColor: 'color-mix(in oklab, #fff 40%, transparent)',
  },
  themeMiniatureI3: {
    height: '0.25rem',
    width: '0.75rem',
    borderRadius: 'calc(infinity * 1px)',
    backgroundColor: 'color-mix(in oklab, #fff 20%, transparent)',
  },
  themeMiniatureSpan3: {
    display: 'flex',
    flex: '1',
    gap: '0.375rem',
    padding: '0.5rem',
  },
  themeMiniatureSpan4: {
    display: 'flex',
    flex: '3',
    flexDirection: 'column',
    gap: '0.375rem',
  },
  themeMiniatureSpan5: {
    display: 'flex',
    flexDirection: 'column',
    gap: '0.25rem',
    borderRadius: '3px',
    padding: '0.375rem',
  },
  themeMiniatureSpanDark2: {
    backgroundColor: '#1b1b1b',
  },
  themeMiniatureSpanNotDark2: {
    backgroundColor: '#fbfbfb',
  },
  themeMiniatureI4: {
    height: '0.25rem',
    borderRadius: 'calc(infinity * 1px)',
  },
  themeMiniatureIFirst: {
    width: 'calc(4 / 5 * 100%)',
    backgroundColor: '#346bf1',
  },
  themeMiniatureIRest: {
    width: 'calc(3 / 4 * 100%)',
  },
  themeMiniatureIRestDark: {
    backgroundColor: 'color-mix(in oklab, #fff 50%, transparent)',
  },
  themeMiniatureIRestLight: {
    backgroundColor: 'color-mix(in oklab, #000 45%, transparent)',
  },
  themeMiniatureI5: {
    height: '0.25rem',
    width: 'calc(1 / 2 * 100%)',
    borderRadius: 'calc(infinity * 1px)',
  },
  themeMiniatureIDark: {
    backgroundColor: 'color-mix(in oklab, #fff 20%, transparent)',
  },
  themeMiniatureINotDark: {
    backgroundColor: 'color-mix(in oklab, #000 15%, transparent)',
  },
  themeMiniatureSpan6: {
    display: 'flex',
    flex: '2',
    flexDirection: 'column',
    gap: '0.25rem',
    borderRadius: '3px',
    padding: '0.375rem',
  },
  themeMiniatureI6: {
    height: '0.375rem',
    width: 'calc(4 / 5 * 100%)',
    borderRadius: 'calc(infinity * 1px)',
  },
  themeMiniatureIDark2: {
    backgroundColor: 'color-mix(in oklab, #fff 60%, transparent)',
  },
  themeMiniatureINotDark2: {
    backgroundColor: 'color-mix(in oklab, #000 55%, transparent)',
  },
  themeMiniatureI7: {
    height: '0.25rem',
    width: '100%',
    borderRadius: 'calc(infinity * 1px)',
  },
  themeMiniatureIDark3: {
    backgroundColor: 'color-mix(in oklab, #fff 15%, transparent)',
  },
  themeMiniatureINotDark3: {
    backgroundColor: 'color-mix(in oklab, #000 10%, transparent)',
  },
  themeThumbnailSpan: {
    position: 'relative',
    display: 'block',
    height: '100%',
    width: '100%',
  },
  themeThumbnailSpan2: {
    position: 'absolute',
    inset: '0px',
    clipPath: 'polygon(62% 0,100% 0,100% 100%,38% 100%)',
  },
});
